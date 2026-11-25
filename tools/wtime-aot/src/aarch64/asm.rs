//! Pure-Rust (`#![forbid(unsafe_code)]`) AArch64 (ARMv8-A) machine-code assembler.
//!
//! Emits fixed-width 32-bit little-endian AArch64 instructions, manages local
//! PC-relative labels (`b`, `b.cond`, `cbz`, `cbnz`, `bl`, `adr`), local `.rodata`
//! literal pools, and link-time relocations (`bl`, `adrp`+`add` pairs).

#![allow(dead_code)]

use std::collections::HashMap;

/// 64-bit / 32-bit general-purpose register (`0..=30` = `X0..X30`, `31` = `SP`/`XZR`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Reg(pub u8);

impl Reg {
    pub const X0: Self = Self(0);
    pub const X1: Self = Self(1);
    pub const X2: Self = Self(2);
    pub const X3: Self = Self(3);
    pub const X4: Self = Self(4);
    pub const X5: Self = Self(5);
    pub const X6: Self = Self(6);
    pub const X7: Self = Self(7);
    pub const X8: Self = Self(8);
    pub const X9: Self = Self(9);
    pub const X10: Self = Self(10);
    pub const X11: Self = Self(11);
    pub const X12: Self = Self(12);
    pub const X13: Self = Self(13);
    pub const X14: Self = Self(14);
    pub const X15: Self = Self(15);
    pub const X16: Self = Self(16);
    pub const X17: Self = Self(17);
    pub const X18: Self = Self(18);
    pub const X19: Self = Self(19);
    pub const X20: Self = Self(20);
    pub const X21: Self = Self(21);
    pub const X22: Self = Self(22);
    pub const X23: Self = Self(23);
    pub const X24: Self = Self(24);
    pub const X25: Self = Self(25);
    pub const X26: Self = Self(26);
    pub const X27: Self = Self(27);
    pub const X28: Self = Self(28);
    pub const FP: Self = Self(29);
    pub const LR: Self = Self(30);
    pub const SP: Self = Self(31);
    pub const XZR: Self = Self(31);

    #[inline]
    pub const fn id(self) -> u8 {
        self.0
    }
}

/// 128-bit FP / AdvSIMD register (`V0..V31`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VReg(pub u8);

impl VReg {
    pub const V0: Self = Self(0);
    pub const V1: Self = Self(1);
    pub const V2: Self = Self(2);
    pub const V3: Self = Self(3);
    pub const V4: Self = Self(4);
    pub const V5: Self = Self(5);
    pub const V6: Self = Self(6);
    pub const V7: Self = Self(7);
    pub const V8: Self = Self(8);
    pub const V9: Self = Self(9);
    pub const V10: Self = Self(10);
    pub const V11: Self = Self(11);
    pub const V12: Self = Self(12);
    pub const V13: Self = Self(13);
    pub const V14: Self = Self(14);
    pub const V15: Self = Self(15);
    pub const V16: Self = Self(16);
    pub const V17: Self = Self(17);
    pub const V18: Self = Self(18);
    pub const V19: Self = Self(19);
    pub const V20: Self = Self(20);
    pub const V21: Self = Self(21);
    pub const V22: Self = Self(22);
    pub const V23: Self = Self(23);
    pub const V24: Self = Self(24);
    pub const V25: Self = Self(25);
    pub const V26: Self = Self(26);
    pub const V27: Self = Self(27);
    pub const V28: Self = Self(28);
    pub const V29: Self = Self(29);
    pub const V30: Self = Self(30);
    pub const V31: Self = Self(31);

    #[inline]
    pub const fn id(self) -> u8 {
        self.0
    }
}

/// AArch64 condition codes (`0..=14`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Cond {
    Eq = 0x0,
    Ne = 0x1,
    Hs = 0x2, // unsigned >= (CS)
    Lo = 0x3, // unsigned <  (CC)
    Mi = 0x4, // negative / FP <
    Pl = 0x5, // non-negative / FP >= or NaN
    Vs = 0x6, // overflow / FP unordered (NaN)
    Vc = 0x7, // no overflow / FP ordered
    Hi = 0x8, // unsigned > / FP > or NaN
    Ls = 0x9, // unsigned <= / FP <=
    Ge = 0xa, // signed >= / FP >=
    Lt = 0xb, // signed <  / FP < or NaN
    Gt = 0xc, // signed >  / FP >
    Le = 0xd, // signed <= / FP <= or NaN
    Al = 0xe,
}

impl Cond {
    #[inline]
    pub const fn invert(self) -> Self {
        match self {
            Cond::Eq => Cond::Ne,
            Cond::Ne => Cond::Eq,
            Cond::Hs => Cond::Lo,
            Cond::Lo => Cond::Hs,
            Cond::Mi => Cond::Pl,
            Cond::Pl => Cond::Mi,
            Cond::Vs => Cond::Vc,
            Cond::Vc => Cond::Vs,
            Cond::Hi => Cond::Ls,
            Cond::Ls => Cond::Hi,
            Cond::Ge => Cond::Lt,
            Cond::Lt => Cond::Ge,
            Cond::Gt => Cond::Le,
            Cond::Le => Cond::Gt,
            Cond::Al => Cond::Al,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Label(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum A64RelocTarget {
    Func(u32),
    Helper(u32),
    Rodata(u32),
    LocalRodata(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum A64RelocKind {
    /// 26-bit PC-relative branch (`b` / `bl`, ±128 MiB).
    Branch26,
    /// `adrp rd, target_page` + immediately following `add rd, rd, #:lo12:target` (±4 GiB).
    AdrpAdd,
}

#[derive(Clone, Copy, Debug)]
pub struct A64Reloc {
    pub patch_off: usize,
    pub kind: A64RelocKind,
    pub target: A64RelocTarget,
}

#[derive(Clone, Copy, Debug)]
enum FixupKind {
    Branch26,
    Cond19,
    Adr21,
}

#[derive(Clone, Copy, Debug)]
struct Fixup {
    off: usize,
    lbl: Label,
    kind: FixupKind,
}

/// Deferred local jump table (`u32` byte offsets relative to `table_base_off` in local rodata).
pub struct A64JumpTable {
    pub rodata_off: u32,
    pub targets: Vec<Label>,
}

/// Fixed-width 32-bit AArch64 assembler with label resolution, local rodata pooling, and relocations.
pub struct A64Asm {
    pub code: Vec<u8>,
    pub relocs: Vec<A64Reloc>,
    pub local_rodata: Vec<u8>,
    pub jump_tables: Vec<A64JumpTable>,
    labels: Vec<Option<usize>>,
    fixups: Vec<Fixup>,
    f32_pool: HashMap<u32, u32>,
    f64_pool: HashMap<u64, u32>,
    v128_pool: HashMap<[u8; 16], u32>,
}

impl Default for A64Asm {
    fn default() -> Self {
        Self::new()
    }
}

impl A64Asm {
    pub fn new() -> Self {
        Self {
            code: Vec::with_capacity(256),
            relocs: Vec::new(),
            local_rodata: Vec::new(),
            jump_tables: Vec::new(),
            labels: Vec::new(),
            fixups: Vec::new(),
            f32_pool: HashMap::new(),
            f64_pool: HashMap::new(),
            v128_pool: HashMap::new(),
        }
    }

    #[inline]
    pub fn pos(&self) -> usize {
        self.code.len()
    }

    #[inline]
    pub fn emit(&mut self, insn: u32) {
        self.code.extend_from_slice(&insn.to_le_bytes());
    }

    pub fn new_label(&mut self) -> Label {
        let id = self.labels.len();
        self.labels.push(None);
        Label(id)
    }

    pub fn bind(&mut self, lbl: Label) {
        debug_assert!(self.labels[lbl.0].is_none(), "label bound twice");
        self.labels[lbl.0] = Some(self.code.len());
    }

    pub fn add_local_rodata(&mut self, align: usize, bytes: &[u8]) -> u32 {
        assert!(align > 0 && align.is_power_of_two());
        let pad = (align - (self.local_rodata.len() & (align - 1))) & (align - 1);
        self.local_rodata.resize(self.local_rodata.len() + pad, 0);
        let off = self.local_rodata.len() as u32;
        self.local_rodata.extend_from_slice(bytes);
        off
    }

    pub fn intern_f32(&mut self, bits: u32) -> u32 {
        if let Some(&off) = self.f32_pool.get(&bits) {
            return off;
        }
        let off = self.add_local_rodata(4, &bits.to_le_bytes());
        self.f32_pool.insert(bits, off);
        off
    }

    pub fn intern_f64(&mut self, bits: u64) -> u32 {
        if let Some(&off) = self.f64_pool.get(&bits) {
            return off;
        }
        let off = self.add_local_rodata(8, &bits.to_le_bytes());
        self.f64_pool.insert(bits, off);
        off
    }

    pub fn intern_v128(&mut self, bytes: [u8; 16]) -> u32 {
        if let Some(&off) = self.v128_pool.get(&bytes) {
            return off;
        }
        let off = self.add_local_rodata(16, &bytes);
        self.v128_pool.insert(bytes, off);
        off
    }

    pub fn add_jump_table(&mut self, targets: Vec<Label>) -> u32 {
        let zeros = vec![0u8; targets.len() * 4];
        let rodata_off = self.add_local_rodata(4, &zeros);
        self.jump_tables.push(A64JumpTable {
            rodata_off,
            targets,
        });
        rodata_off
    }

    /// Resolves all local branch fixups; emits out-of-range branch veneers automatically if a
    /// conditional branch (`±1 MiB`) ever exceeds `imm19`.
    pub fn finish(&mut self) -> Result<(), String> {
        let fixups = std::mem::take(&mut self.fixups);
        for fx in fixups {
            let target = self.labels[fx.lbl.0]
                .ok_or_else(|| format!("aarch64: unbound label {}", fx.lbl.0))?;
            let diff = (target as isize) - (fx.off as isize);
            let mut insn = u32::from_le_bytes(self.code[fx.off..fx.off + 4].try_into().unwrap());
            match fx.kind {
                FixupKind::Branch26 => {
                    let imm = diff >> 2;
                    if !(-(1 << 25)..(1 << 25)).contains(&imm) {
                        return Err(format!("aarch64: b/bl displacement {diff} out of ±128 MiB"));
                    }
                    insn = (insn & 0xfc00_0000) | ((imm as u32) & 0x03ff_ffff);
                    self.code[fx.off..fx.off + 4].copy_from_slice(&insn.to_le_bytes());
                }
                FixupKind::Cond19 => {
                    let imm = diff >> 2;
                    if (-(1 << 18)..(1 << 18)).contains(&imm) {
                        insn = (insn & 0xff00_001f) | (((imm as u32) & 0x7_ffff) << 5);
                        self.code[fx.off..fx.off + 4].copy_from_slice(&insn.to_le_bytes());
                    } else {
                        // Emit a 1-instruction unconditional branch trampoline at end of function.
                        let veneer_off = self.code.len();
                        let vdiff = (target as isize) - (veneer_off as isize);
                        let vimm = vdiff >> 2;
                        if !(-(1 << 25)..(1 << 25)).contains(&vimm) {
                            return Err(
                                "aarch64: branch veneer displacement out of ±128 MiB".into()
                            );
                        }
                        self.emit(0x1400_0000 | ((vimm as u32) & 0x03ff_ffff));
                        let to_veneer = ((veneer_off as isize) - (fx.off as isize)) >> 2;
                        if !(-(1 << 18)..(1 << 18)).contains(&to_veneer) {
                            return Err("aarch64: conditional branch veneer out of ±1 MiB".into());
                        }
                        insn = (insn & 0xff00_001f) | (((to_veneer as u32) & 0x7_ffff) << 5);
                        self.code[fx.off..fx.off + 4].copy_from_slice(&insn.to_le_bytes());
                    }
                }
                FixupKind::Adr21 => {
                    if !(-(1 << 20)..(1 << 20)).contains(&diff) {
                        return Err(format!("aarch64: adr displacement {diff} out of ±1 MiB"));
                    }
                    let u = diff as u32;
                    let immlo = u & 3;
                    let immhi = (u >> 2) & 0x7_ffff;
                    insn = (insn & 0x9f00_001f) | (immlo << 29) | (immhi << 5);
                    self.code[fx.off..fx.off + 4].copy_from_slice(&insn.to_le_bytes());
                }
            }
        }
        Ok(())
    }

    /// Patches local jump tables in `local_rodata` to signed 32-bit offsets from `table_vaddr` to
    /// each target label's absolute vaddr (`fn_vaddr + label_off`).
    pub fn patch_jump_tables(
        &mut self,
        fn_vaddr: u64,
        local_rodata_vaddr: u64,
    ) -> Result<(), String> {
        for jt in &self.jump_tables {
            let table_vaddr = local_rodata_vaddr + u64::from(jt.rodata_off);
            let base = jt.rodata_off as usize;
            for (i, &lbl) in jt.targets.iter().enumerate() {
                let target_off = self.labels[lbl.0]
                    .ok_or_else(|| format!("aarch64: unbound jump table label {}", lbl.0))?;
                let target_vaddr = fn_vaddr + target_off as u64;
                let rel = (target_vaddr as i64) - (table_vaddr as i64);
                let rel32 = i32::try_from(rel)
                    .map_err(|_| "aarch64: jump table target exceeds i32 range".to_string())?;
                self.local_rodata[base + i * 4..base + (i + 1) * 4]
                    .copy_from_slice(&rel32.to_le_bytes());
            }
        }
        Ok(())
    }

    /// Extracts resolved jump-table metadata `(rodata_off, target_code_offs)` for incremental caching.
    pub fn resolved_jump_tables(&self) -> Result<Vec<(u32, Vec<u32>)>, String> {
        let mut out = Vec::with_capacity(self.jump_tables.len());
        for jt in &self.jump_tables {
            let mut offs = Vec::with_capacity(jt.targets.len());
            for &lbl in &jt.targets {
                let target_off = self.labels[lbl.0]
                    .ok_or_else(|| format!("aarch64: unbound jump table label {}", lbl.0))?;
                let off32 = u32::try_from(target_off)
                    .map_err(|_| "aarch64: jump table target exceeds u32".to_string())?;
                offs.push(off32);
            }
            out.push((jt.rodata_off, offs));
        }
        Ok(out)
    }

    /// Reconstructs a link-ready `A64Asm` directly from cached pre-link parts.
    pub fn from_cached_parts(
        code: Vec<u8>,
        relocs: Vec<A64Reloc>,
        local_rodata: Vec<u8>,
        jt_resolved: Vec<(u32, Vec<u32>)>,
    ) -> Self {
        let mut labels = Vec::new();
        let mut jump_tables = Vec::with_capacity(jt_resolved.len());
        for (rodata_off, target_offs) in jt_resolved {
            let mut targets = Vec::with_capacity(target_offs.len());
            for toff in target_offs {
                let id = labels.len();
                labels.push(Some(toff as usize));
                targets.push(Label(id));
            }
            jump_tables.push(A64JumpTable {
                rodata_off,
                targets,
            });
        }
        Self {
            code,
            relocs,
            local_rodata,
            jump_tables,
            labels,
            fixups: Vec::new(),
            f32_pool: HashMap::new(),
            f64_pool: HashMap::new(),
            v128_pool: HashMap::new(),
        }
    }

    // ---- Integer ALU (register & immediate) ----

    #[inline]
    fn sf(is64: bool) -> u32 {
        u32::from(is64) << 31
    }

    pub fn add_r(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg) {
        self.emit(
            Self::sf(is64)
                | 0x0b00_0000
                | ((rm.0 as u32) << 16)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    /// `add rd, rn, rm, lsl #shift` (shifted register, 1-cycle small-constant multiply building block).
    pub fn add_r_lsl(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg, shift: u8) {
        debug_assert!(shift < if is64 { 64 } else { 32 });
        self.emit(
            Self::sf(is64)
                | 0x0b00_0000
                | ((rm.0 as u32) << 16)
                | ((u32::from(shift) & 0x3f) << 10)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    /// `add xd, xn, wm, uxtw #0` (zero-extends 32-bit `wm` and adds to 64-bit `xn` in 1 insn).
    pub fn add_ext_uxtw(&mut self, rd: Reg, rn: Reg, rm: Reg) {
        self.emit(0x8b20_4000 | ((rm.0 as u32) << 16) | ((rn.0 as u32) << 5) | rd.0 as u32);
    }

    pub fn adds_r(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg) {
        self.emit(
            Self::sf(is64)
                | 0x2b00_0000
                | ((rm.0 as u32) << 16)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn sub_r(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg) {
        self.emit(
            Self::sf(is64)
                | 0x4b00_0000
                | ((rm.0 as u32) << 16)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    /// `sub rd, rn, rm, lsl #shift` (shifted register).
    pub fn sub_r_lsl(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg, shift: u8) {
        debug_assert!(shift < if is64 { 64 } else { 32 });
        self.emit(
            Self::sf(is64)
                | 0x4b00_0000
                | ((rm.0 as u32) << 16)
                | ((u32::from(shift) & 0x3f) << 10)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn subs_r(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg) {
        self.emit(
            Self::sf(is64)
                | 0x6b00_0000
                | ((rm.0 as u32) << 16)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn cmp_r(&mut self, is64: bool, rn: Reg, rm: Reg) {
        self.subs_r(is64, Reg::XZR, rn, rm);
    }

    pub fn add_imm(&mut self, is64: bool, rd: Reg, rn: Reg, imm12: u16) {
        assert!(imm12 < 4096);
        self.emit(
            Self::sf(is64)
                | 0x1100_0000
                | ((imm12 as u32) << 10)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn add_imm_lsl12(&mut self, is64: bool, rd: Reg, rn: Reg, imm12: u16) {
        assert!(imm12 < 4096);
        self.emit(
            Self::sf(is64)
                | 0x1140_0000
                | ((imm12 as u32) << 10)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn sub_imm(&mut self, is64: bool, rd: Reg, rn: Reg, imm12: u16) {
        assert!(imm12 < 4096);
        self.emit(
            Self::sf(is64)
                | 0x5100_0000
                | ((imm12 as u32) << 10)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn sub_imm_lsl12(&mut self, is64: bool, rd: Reg, rn: Reg, imm12: u16) {
        assert!(imm12 < 4096);
        self.emit(
            Self::sf(is64)
                | 0x5140_0000
                | ((imm12 as u32) << 10)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn subs_imm(&mut self, is64: bool, rd: Reg, rn: Reg, imm12: u16) {
        assert!(imm12 < 4096);
        self.emit(
            Self::sf(is64)
                | 0x7100_0000
                | ((imm12 as u32) << 10)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn cmp_imm(&mut self, is64: bool, rn: Reg, imm12: u16) {
        self.subs_imm(is64, Reg::XZR, rn, imm12);
    }

    /// Adds an arbitrary signed 32-bit delta to `rn` into `rd` (`SP`-safe when `tmp` is provided).
    pub fn add_i32_checked(&mut self, is64: bool, rd: Reg, rn: Reg, delta: i32, tmp: Reg) {
        if delta == 0 {
            if rd != rn {
                if rd == Reg::SP || rn == Reg::SP {
                    self.add_imm(true, rd, rn, 0);
                } else {
                    self.mov_r(is64, rd, rn);
                }
            }
            return;
        }
        let abs = delta.unsigned_abs();
        let neg = delta < 0;
        if abs < 4096 {
            if neg {
                self.sub_imm(is64, rd, rn, abs as u16);
            } else {
                self.add_imm(is64, rd, rn, abs as u16);
            }
        } else if abs < (1 << 24) {
            let hi = (abs >> 12) as u16;
            let lo = (abs & 0xfff) as u16;
            if neg {
                self.sub_imm_lsl12(is64, rd, rn, hi);
                if lo != 0 {
                    self.sub_imm(is64, rd, rd, lo);
                }
            } else {
                self.add_imm_lsl12(is64, rd, rn, hi);
                if lo != 0 {
                    self.add_imm(is64, rd, rd, lo);
                }
            }
        } else {
            self.mov_imm64(tmp, delta as i64 as u64);
            if rd == Reg::SP || rn == Reg::SP {
                // `ADD (extended register)` with UXTX (`option=011`) supports `SP`!
                self.emit(
                    Self::sf(true)
                        | 0x0b20_6000
                        | ((tmp.0 as u32) << 16)
                        | ((rn.0 as u32) << 5)
                        | rd.0 as u32,
                );
            } else {
                self.add_r(is64, rd, rn, tmp);
            }
        }
    }

    pub fn and_r(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg) {
        self.emit(
            Self::sf(is64)
                | 0x0a00_0000
                | ((rm.0 as u32) << 16)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn bic_r(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg) {
        self.emit(
            Self::sf(is64)
                | 0x0a20_0000
                | ((rm.0 as u32) << 16)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn orr_r(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg) {
        self.emit(
            Self::sf(is64)
                | 0x2a00_0000
                | ((rm.0 as u32) << 16)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn eor_r(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg) {
        self.emit(
            Self::sf(is64)
                | 0x4a00_0000
                | ((rm.0 as u32) << 16)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn ands_r(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg) {
        self.emit(
            Self::sf(is64)
                | 0x6a00_0000
                | ((rm.0 as u32) << 16)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn tst_r(&mut self, is64: bool, rn: Reg, rm: Reg) {
        self.ands_r(is64, Reg::XZR, rn, rm);
    }

    pub fn mov_r(&mut self, is64: bool, rd: Reg, rm: Reg) {
        if rd == rm && is64 {
            return;
        }
        if rd == Reg::SP || rm == Reg::SP {
            self.add_imm(true, rd, rm, 0);
        } else {
            self.orr_r(is64, rd, Reg::XZR, rm);
        }
    }

    pub fn mvn_r(&mut self, is64: bool, rd: Reg, rm: Reg) {
        self.emit(Self::sf(is64) | 0x2a20_03e0 | ((rm.0 as u32) << 16) | rd.0 as u32);
    }

    pub fn neg_r(&mut self, is64: bool, rd: Reg, rm: Reg) {
        self.sub_r(is64, rd, Reg::XZR, rm);
    }

    pub fn movz(&mut self, is64: bool, rd: Reg, imm16: u16, hw: u8) {
        assert!(hw < if is64 { 4 } else { 2 });
        self.emit(
            Self::sf(is64)
                | 0x5280_0000
                | ((hw as u32) << 21)
                | ((imm16 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn movn(&mut self, is64: bool, rd: Reg, imm16: u16, hw: u8) {
        assert!(hw < if is64 { 4 } else { 2 });
        self.emit(
            Self::sf(is64)
                | 0x1280_0000
                | ((hw as u32) << 21)
                | ((imm16 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn movk(&mut self, is64: bool, rd: Reg, imm16: u16, hw: u8) {
        assert!(hw < if is64 { 4 } else { 2 });
        self.emit(
            Self::sf(is64)
                | 0x7280_0000
                | ((hw as u32) << 21)
                | ((imm16 as u32) << 5)
                | rd.0 as u32,
        );
    }

    /// Materializes any 32-bit constant into `Wd` (zero-extending upper 32 bits of `Xd`).
    pub fn mov_imm32(&mut self, rd: Reg, v: u32) {
        let lo = (v & 0xffff) as u16;
        let hi = (v >> 16) as u16;
        if hi == 0 {
            self.movz(false, rd, lo, 0);
        } else if lo == 0 {
            self.movz(false, rd, hi, 1);
        } else if hi == 0xffff {
            self.movn(false, rd, !lo, 0);
        } else if lo == 0xffff {
            self.movn(false, rd, !hi, 1);
        } else {
            self.movz(false, rd, lo, 0);
            self.movk(false, rd, hi, 1);
        }
    }

    /// Materializes any 64-bit constant into `Xd` using minimal `movz`/`movn`/`movk` instructions.
    pub fn mov_imm64(&mut self, rd: Reg, v: u64) {
        if v <= u32::MAX as u64 {
            self.mov_imm32(rd, v as u32);
            return;
        }
        let h = [
            (v & 0xffff) as u16,
            ((v >> 16) & 0xffff) as u16,
            ((v >> 32) & 0xffff) as u16,
            ((v >> 48) & 0xffff) as u16,
        ];
        let zeros = h.iter().filter(|&&x| x == 0).count();
        let ones = h.iter().filter(|&&x| x == 0xffff).count();
        if ones > zeros {
            let mut first = true;
            for (i, &w) in h.iter().enumerate() {
                if w != 0xffff {
                    if first {
                        self.movn(true, rd, !w, i as u8);
                        first = false;
                    } else {
                        self.movk(true, rd, w, i as u8);
                    }
                }
            }
            if first {
                self.movn(true, rd, 0, 0);
            }
        } else {
            let mut first = true;
            for (i, &w) in h.iter().enumerate() {
                if w != 0 {
                    if first {
                        self.movz(true, rd, w, i as u8);
                        first = false;
                    } else {
                        self.movk(true, rd, w, i as u8);
                    }
                }
            }
            if first {
                self.movz(true, rd, 0, 0);
            }
        }
    }

    pub fn madd(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg, ra: Reg) {
        self.emit(
            Self::sf(is64)
                | 0x1b00_0000
                | ((rm.0 as u32) << 16)
                | ((ra.0 as u32) << 10)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn msub(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg, ra: Reg) {
        self.emit(
            Self::sf(is64)
                | 0x1b00_8000
                | ((rm.0 as u32) << 16)
                | ((ra.0 as u32) << 10)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn mul(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg) {
        self.madd(is64, rd, rn, rm, Reg::XZR);
    }

    /// 64-bit unsigned high multiply: `rd = ((u128(rn) * u128(rm)) >> 64) as u64`.
    pub fn umulh(&mut self, rd: Reg, rn: Reg, rm: Reg) {
        self.emit(0x9bc0_7c00 | ((rm.0 as u32) << 16) | ((rn.0 as u32) << 5) | rd.0 as u32);
    }

    /// Add with carry: `rd = rn + rm + C`.
    pub fn adc(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg) {
        self.emit(
            Self::sf(is64)
                | 0x1a00_0000
                | ((rm.0 as u32) << 16)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn udiv(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg) {
        self.emit(
            Self::sf(is64)
                | 0x1ac0_0800
                | ((rm.0 as u32) << 16)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn sdiv(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg) {
        self.emit(
            Self::sf(is64)
                | 0x1ac0_0c00
                | ((rm.0 as u32) << 16)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn lslv(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg) {
        self.emit(
            Self::sf(is64)
                | 0x1ac0_2000
                | ((rm.0 as u32) << 16)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn lsrv(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg) {
        self.emit(
            Self::sf(is64)
                | 0x1ac0_2400
                | ((rm.0 as u32) << 16)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn asrv(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg) {
        self.emit(
            Self::sf(is64)
                | 0x1ac0_2800
                | ((rm.0 as u32) << 16)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn rorv(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg) {
        self.emit(
            Self::sf(is64)
                | 0x1ac0_2c00
                | ((rm.0 as u32) << 16)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn clz(&mut self, is64: bool, rd: Reg, rn: Reg) {
        self.emit(Self::sf(is64) | 0x5ac0_1000 | ((rn.0 as u32) << 5) | rd.0 as u32);
    }

    pub fn rbit(&mut self, is64: bool, rd: Reg, rn: Reg) {
        self.emit(Self::sf(is64) | 0x5ac0_0000 | ((rn.0 as u32) << 5) | rd.0 as u32);
    }

    pub fn sbfm(&mut self, is64: bool, rd: Reg, rn: Reg, immr: u8, imms: u8) {
        let n = u32::from(is64) << 22;
        self.emit(
            Self::sf(is64)
                | 0x1300_0000
                | n
                | (((immr & 63) as u32) << 16)
                | (((imms & 63) as u32) << 10)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn ubfm(&mut self, is64: bool, rd: Reg, rn: Reg, immr: u8, imms: u8) {
        let n = u32::from(is64) << 22;
        self.emit(
            Self::sf(is64)
                | 0x5300_0000
                | n
                | (((immr & 63) as u32) << 16)
                | (((imms & 63) as u32) << 10)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn lsl_imm(&mut self, is64: bool, rd: Reg, rn: Reg, shift: u8) {
        let w = if is64 { 64u8 } else { 32u8 };
        let sh = shift & (w - 1);
        if sh == 0 {
            self.mov_r(is64, rd, rn);
        } else {
            self.ubfm(is64, rd, rn, w - sh, w - 1 - sh);
        }
    }

    pub fn lsr_imm(&mut self, is64: bool, rd: Reg, rn: Reg, shift: u8) {
        let w = if is64 { 64u8 } else { 32u8 };
        let sh = shift & (w - 1);
        if sh == 0 {
            self.mov_r(is64, rd, rn);
        } else {
            self.ubfm(is64, rd, rn, sh, w - 1);
        }
    }

    pub fn asr_imm(&mut self, is64: bool, rd: Reg, rn: Reg, shift: u8) {
        let w = if is64 { 64u8 } else { 32u8 };
        let sh = shift & (w - 1);
        if sh == 0 {
            self.mov_r(is64, rd, rn);
        } else {
            self.sbfm(is64, rd, rn, sh, w - 1);
        }
    }

    pub fn ror_imm(&mut self, is64: bool, rd: Reg, rn: Reg, shift: u8) {
        let w = if is64 { 64u8 } else { 32u8 };
        let sh = shift & (w - 1);
        if sh == 0 {
            self.mov_r(is64, rd, rn);
        } else {
            let n = u32::from(is64) << 22;
            self.emit(
                Self::sf(is64)
                    | 0x1380_0000
                    | n
                    | ((rn.0 as u32) << 16)
                    | ((sh as u32) << 10)
                    | ((rn.0 as u32) << 5)
                    | rd.0 as u32,
            );
        }
    }

    pub fn sxtb(&mut self, is64: bool, rd: Reg, rn: Reg) {
        self.sbfm(is64, rd, rn, 0, 7);
    }

    pub fn sxth(&mut self, is64: bool, rd: Reg, rn: Reg) {
        self.sbfm(is64, rd, rn, 0, 15);
    }

    pub fn sxtw(&mut self, rd: Reg, rn: Reg) {
        self.sbfm(true, rd, rn, 0, 31);
    }

    pub fn uxtb(&mut self, rd: Reg, rn: Reg) {
        self.ubfm(false, rd, rn, 0, 7);
    }

    pub fn uxth(&mut self, rd: Reg, rn: Reg) {
        self.ubfm(false, rd, rn, 0, 15);
    }

    /// Zero-extends 32-bit `Wn` into 64-bit `Xd` (`mov Wd, Wn`).
    pub fn uxtw(&mut self, rd: Reg, rn: Reg) {
        self.orr_r(false, rd, Reg::XZR, rn);
    }

    pub fn csel(&mut self, is64: bool, rd: Reg, rn: Reg, rm: Reg, cond: Cond) {
        self.emit(
            Self::sf(is64)
                | 0x1a80_0000
                | ((rm.0 as u32) << 16)
                | ((cond as u32) << 12)
                | ((rn.0 as u32) << 5)
                | rd.0 as u32,
        );
    }

    pub fn cset(&mut self, is64: bool, rd: Reg, cond: Cond) {
        let inv = cond.invert();
        self.emit(Self::sf(is64) | 0x1a9f_07e0 | ((inv as u32) << 12) | rd.0 as u32);
    }

    // ---- Control Flow & PC-Relative Addressing ----

    pub fn b(&mut self, lbl: Label) {
        let off = self.pos();
        self.emit(0x1400_0000);
        self.fixups.push(Fixup {
            off,
            lbl,
            kind: FixupKind::Branch26,
        });
    }

    pub fn bl_label(&mut self, lbl: Label) {
        let off = self.pos();
        self.emit(0x9400_0000);
        self.fixups.push(Fixup {
            off,
            lbl,
            kind: FixupKind::Branch26,
        });
    }

    pub fn bl_reloc(&mut self, target: A64RelocTarget) {
        let patch_off = self.pos();
        self.emit(0x9400_0000);
        self.relocs.push(A64Reloc {
            patch_off,
            kind: A64RelocKind::Branch26,
            target,
        });
    }

    pub fn b_reloc(&mut self, target: A64RelocTarget) {
        let patch_off = self.pos();
        self.emit(0x1400_0000);
        self.relocs.push(A64Reloc {
            patch_off,
            kind: A64RelocKind::Branch26,
            target,
        });
    }

    pub fn b_cond(&mut self, cond: Cond, lbl: Label) {
        let off = self.pos();
        self.emit(0x5400_0000 | (cond as u32));
        self.fixups.push(Fixup {
            off,
            lbl,
            kind: FixupKind::Cond19,
        });
    }

    pub fn cbz(&mut self, is64: bool, rt: Reg, lbl: Label) {
        let off = self.pos();
        self.emit(Self::sf(is64) | 0x3400_0000 | rt.0 as u32);
        self.fixups.push(Fixup {
            off,
            lbl,
            kind: FixupKind::Cond19,
        });
    }

    pub fn cbnz(&mut self, is64: bool, rt: Reg, lbl: Label) {
        let off = self.pos();
        self.emit(Self::sf(is64) | 0x3500_0000 | rt.0 as u32);
        self.fixups.push(Fixup {
            off,
            lbl,
            kind: FixupKind::Cond19,
        });
    }

    pub fn br(&mut self, rn: Reg) {
        self.emit(0xd61f_0000 | ((rn.0 as u32) << 5));
    }

    pub fn blr(&mut self, rn: Reg) {
        self.emit(0xd63f_0000 | ((rn.0 as u32) << 5));
    }

    pub fn ret(&mut self) {
        self.emit(0xd65f_03c0);
    }

    pub fn adr(&mut self, rd: Reg, lbl: Label) {
        let off = self.pos();
        self.emit(0x1000_0000 | rd.0 as u32);
        self.fixups.push(Fixup {
            off,
            lbl,
            kind: FixupKind::Adr21,
        });
    }

    /// Emits `adrp rd, target@PAGE; add rd, rd, target@PAGEOFF` (patched at ELF link time).
    pub fn adr_reloc(&mut self, rd: Reg, target: A64RelocTarget) {
        let patch_off = self.pos();
        self.emit(0x9000_0000 | rd.0 as u32); // adrp rd, 0
        self.emit(0x9100_0000 | ((rd.0 as u32) << 5) | rd.0 as u32); // add rd, rd, #0
        self.relocs.push(A64Reloc {
            patch_off,
            kind: A64RelocKind::AdrpAdd,
            target,
        });
    }

    pub fn svc(&mut self, imm16: u16) {
        self.emit(0xd400_0001 | ((imm16 as u32) << 5));
    }

    pub fn nop(&mut self) {
        self.emit(0xd503_201f);
    }

    pub fn brk(&mut self, imm16: u16) {
        self.emit(0xd420_0000 | ((imm16 as u32) << 5));
    }

    // ---- Loads & Stores (unsigned scaled immediate & register offset & pair) ----

    /// Encodes `[rn, #off]` unsigned scaled immediate load/store.
    #[allow(clippy::too_many_arguments)]
    fn ldst_uimm(
        &mut self,
        size: u32,
        is_fp: bool,
        opc: u32,
        rt: u8,
        rn: Reg,
        byte_off: u32,
        shift: u32,
    ) {
        debug_assert!((byte_off & ((1 << shift) - 1)) == 0);
        let imm12 = byte_off >> shift;
        assert!(
            imm12 < 4096,
            "aarch64: unsigned offset {byte_off} exceeds imm12<<{shift}"
        );
        self.emit(
            (size << 30)
                | 0x3900_0000
                | (u32::from(is_fp) << 26)
                | (opc << 22)
                | (imm12 << 10)
                | ((rn.0 as u32) << 5)
                | (rt as u32),
        );
    }

    /// Encodes `[rn, rm, LSL #0/#scale]` (`uxtw=false`) or `[rn, wm, UXTW #0/#scale]` (`uxtw=true`).
    #[allow(clippy::too_many_arguments)]
    fn ldst_reg_ext(
        &mut self,
        size: u32,
        is_fp: bool,
        opc: u32,
        rt: u8,
        rn: Reg,
        rm: Reg,
        uxtw: bool,
        scaled: bool,
    ) {
        let opt = if uxtw { 0b010u32 } else { 0b011u32 };
        let s = u32::from(scaled);
        self.emit(
            (size << 30)
                | 0x3820_0800
                | (u32::from(is_fp) << 26)
                | (opc << 22)
                | ((rm.0 as u32) << 16)
                | (opt << 13)
                | (s << 12)
                | ((rn.0 as u32) << 5)
                | (rt as u32),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn ldst_reg(&mut self, size: u32, is_fp: bool, opc: u32, rt: u8, rn: Reg, rm: Reg, uxtw: bool) {
        self.ldst_reg_ext(size, is_fp, opc, rt, rn, rm, uxtw, false);
    }

    pub fn ldr_uimm(&mut self, is64: bool, rt: Reg, rn: Reg, off: u32) {
        if is64 {
            self.ldst_uimm(3, false, 1, rt.0, rn, off, 3);
        } else {
            self.ldst_uimm(2, false, 1, rt.0, rn, off, 2);
        }
    }

    pub fn str_uimm(&mut self, is64: bool, rt: Reg, rn: Reg, off: u32) {
        if is64 {
            self.ldst_uimm(3, false, 0, rt.0, rn, off, 3);
        } else {
            self.ldst_uimm(2, false, 0, rt.0, rn, off, 2);
        }
    }

    pub fn ldrb_uimm(&mut self, rt: Reg, rn: Reg, off: u32) {
        self.ldst_uimm(0, false, 1, rt.0, rn, off, 0);
    }

    pub fn ldrsb_uimm(&mut self, is64: bool, rt: Reg, rn: Reg, off: u32) {
        self.ldst_uimm(0, false, if is64 { 2 } else { 3 }, rt.0, rn, off, 0);
    }

    pub fn strb_uimm(&mut self, rt: Reg, rn: Reg, off: u32) {
        self.ldst_uimm(0, false, 0, rt.0, rn, off, 0);
    }

    pub fn ldrh_uimm(&mut self, rt: Reg, rn: Reg, off: u32) {
        self.ldst_uimm(1, false, 1, rt.0, rn, off, 1);
    }

    pub fn ldrsh_uimm(&mut self, is64: bool, rt: Reg, rn: Reg, off: u32) {
        self.ldst_uimm(1, false, if is64 { 2 } else { 3 }, rt.0, rn, off, 1);
    }

    pub fn strh_uimm(&mut self, rt: Reg, rn: Reg, off: u32) {
        self.ldst_uimm(1, false, 0, rt.0, rn, off, 1);
    }

    pub fn ldrsw_uimm(&mut self, rt: Reg, rn: Reg, off: u32) {
        self.ldst_uimm(2, false, 2, rt.0, rn, off, 2);
    }

    pub fn ldr_s_uimm(&mut self, vt: VReg, rn: Reg, off: u32) {
        self.ldst_uimm(2, true, 1, vt.0, rn, off, 2);
    }

    pub fn str_s_uimm(&mut self, vt: VReg, rn: Reg, off: u32) {
        self.ldst_uimm(2, true, 0, vt.0, rn, off, 2);
    }

    pub fn ldr_d_uimm(&mut self, vt: VReg, rn: Reg, off: u32) {
        self.ldst_uimm(3, true, 1, vt.0, rn, off, 3);
    }

    pub fn str_d_uimm(&mut self, vt: VReg, rn: Reg, off: u32) {
        self.ldst_uimm(3, true, 0, vt.0, rn, off, 3);
    }

    pub fn ldr_q_uimm(&mut self, vt: VReg, rn: Reg, off: u32) {
        self.ldst_uimm(0, true, 3, vt.0, rn, off, 4);
    }

    pub fn str_q_uimm(&mut self, vt: VReg, rn: Reg, off: u32) {
        self.ldst_uimm(0, true, 2, vt.0, rn, off, 4);
    }

    // Register-indexed `[rn, rm]` (unscaled 64-bit `rm` or `UXTW` 32-bit `wm`)
    pub fn ldr_reg(&mut self, is64: bool, rt: Reg, rn: Reg, rm: Reg, uxtw: bool) {
        self.ldst_reg(if is64 { 3 } else { 2 }, false, 1, rt.0, rn, rm, uxtw);
    }

    pub fn str_reg(&mut self, is64: bool, rt: Reg, rn: Reg, rm: Reg, uxtw: bool) {
        self.ldst_reg(if is64 { 3 } else { 2 }, false, 0, rt.0, rn, rm, uxtw);
    }

    pub fn ldrb_reg(&mut self, rt: Reg, rn: Reg, rm: Reg, uxtw: bool) {
        self.ldst_reg(0, false, 1, rt.0, rn, rm, uxtw);
    }

    pub fn ldrsb_reg(&mut self, is64: bool, rt: Reg, rn: Reg, rm: Reg, uxtw: bool) {
        self.ldst_reg(0, false, if is64 { 2 } else { 3 }, rt.0, rn, rm, uxtw);
    }

    pub fn strb_reg(&mut self, rt: Reg, rn: Reg, rm: Reg, uxtw: bool) {
        self.ldst_reg(0, false, 0, rt.0, rn, rm, uxtw);
    }

    pub fn ldrh_reg(&mut self, rt: Reg, rn: Reg, rm: Reg, uxtw: bool) {
        self.ldst_reg(1, false, 1, rt.0, rn, rm, uxtw);
    }

    pub fn ldrsh_reg(&mut self, is64: bool, rt: Reg, rn: Reg, rm: Reg, uxtw: bool) {
        self.ldst_reg(1, false, if is64 { 2 } else { 3 }, rt.0, rn, rm, uxtw);
    }

    pub fn strh_reg(&mut self, rt: Reg, rn: Reg, rm: Reg, uxtw: bool) {
        self.ldst_reg(1, false, 0, rt.0, rn, rm, uxtw);
    }

    pub fn ldrsw_reg(&mut self, rt: Reg, rn: Reg, rm: Reg, uxtw: bool) {
        self.ldst_reg(2, false, 2, rt.0, rn, rm, uxtw);
    }

    pub fn ldr_s_reg(&mut self, vt: VReg, rn: Reg, rm: Reg, uxtw: bool) {
        self.ldst_reg(2, true, 1, vt.0, rn, rm, uxtw);
    }

    pub fn str_s_reg(&mut self, vt: VReg, rn: Reg, rm: Reg, uxtw: bool) {
        self.ldst_reg(2, true, 0, vt.0, rn, rm, uxtw);
    }

    pub fn ldr_d_reg(&mut self, vt: VReg, rn: Reg, rm: Reg, uxtw: bool) {
        self.ldst_reg(3, true, 1, vt.0, rn, rm, uxtw);
    }

    pub fn str_d_reg(&mut self, vt: VReg, rn: Reg, rm: Reg, uxtw: bool) {
        self.ldst_reg(3, true, 0, vt.0, rn, rm, uxtw);
    }

    pub fn ldr_q_reg(&mut self, vt: VReg, rn: Reg, rm: Reg, uxtw: bool) {
        self.ldst_reg(0, true, 3, vt.0, rn, rm, uxtw);
    }

    pub fn str_q_reg(&mut self, vt: VReg, rn: Reg, rm: Reg, uxtw: bool) {
        self.ldst_reg(0, true, 2, vt.0, rn, rm, uxtw);
    }

    /// Scaled register-indexed `[rn, rm, UXTW/LSL #scale]` (`scale = 2` for 32-bit, `3` for 64-bit).
    pub fn ldr_reg_scaled(&mut self, is64: bool, rt: Reg, rn: Reg, rm: Reg, uxtw: bool) {
        self.ldst_reg_ext(if is64 { 3 } else { 2 }, false, 1, rt.0, rn, rm, uxtw, true);
    }

    pub fn str_reg_scaled(&mut self, is64: bool, rt: Reg, rn: Reg, rm: Reg, uxtw: bool) {
        self.ldst_reg_ext(if is64 { 3 } else { 2 }, false, 0, rt.0, rn, rm, uxtw, true);
    }

    pub fn ldrsw_reg_scaled(&mut self, rt: Reg, rn: Reg, rm: Reg, uxtw: bool) {
        self.ldst_reg_ext(2, false, 2, rt.0, rn, rm, uxtw, true);
    }

    // Load/Store Pair (64-bit GP & 128-bit Q)
    #[allow(clippy::too_many_arguments)]
    fn ldp_stp(
        &mut self,
        is_q: bool,
        is_load: bool,
        mode: u32,
        rt1: u8,
        rt2: u8,
        rn: Reg,
        off: i32,
    ) {
        let shift = if is_q { 4 } else { 3 };
        debug_assert!((off & ((1 << shift) - 1)) == 0);
        let imm7 = off >> shift;
        assert!(
            (-64..=63).contains(&imm7),
            "aarch64: stp/ldp offset {off} out of range"
        );
        self.emit(
            0xa800_0000
                | (u32::from(is_q) << 26)
                | (mode << 23)
                | (u32::from(is_load) << 22)
                | (((imm7 as u32) & 0x7f) << 15)
                | ((rt2 as u32) << 10)
                | ((rn.0 as u32) << 5)
                | (rt1 as u32),
        );
    }

    pub fn stp_x_pre(&mut self, rt1: Reg, rt2: Reg, rn: Reg, off: i32) {
        self.ldp_stp(false, false, 0b011, rt1.0, rt2.0, rn, off);
    }

    pub fn ldp_x_post(&mut self, rt1: Reg, rt2: Reg, rn: Reg, off: i32) {
        self.ldp_stp(false, true, 0b001, rt1.0, rt2.0, rn, off);
    }

    pub fn stp_x(&mut self, rt1: Reg, rt2: Reg, rn: Reg, off: i32) {
        self.ldp_stp(false, false, 0b010, rt1.0, rt2.0, rn, off);
    }

    pub fn ldp_x(&mut self, rt1: Reg, rt2: Reg, rn: Reg, off: i32) {
        self.ldp_stp(false, true, 0b010, rt1.0, rt2.0, rn, off);
    }

    pub fn stp_q(&mut self, vt1: VReg, vt2: VReg, rn: Reg, off: i32) {
        self.ldp_stp(true, false, 0b010, vt1.0, vt2.0, rn, off);
    }

    pub fn ldp_q(&mut self, vt1: VReg, vt2: VReg, rn: Reg, off: i32) {
        self.ldp_stp(true, true, 0b010, vt1.0, vt2.0, rn, off);
    }

    // ---- Scalar IEEE-754 Floating-Point ----

    fn fp_bin(&mut self, is64: bool, op: u32, vd: VReg, vn: VReg, vm: VReg) {
        let ft = u32::from(is64) << 22;
        self.emit(
            0x1e20_0800
                | ft
                | ((vm.0 as u32) << 16)
                | (op << 12)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }

    fn fp_un(&mut self, is64: bool, opcode: u32, vd: VReg, vn: VReg) {
        let ft = u32::from(is64) << 22;
        self.emit(0x1e20_4000 | ft | (opcode << 15) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }

    pub fn fadd(&mut self, is64: bool, vd: VReg, vn: VReg, vm: VReg) {
        self.fp_bin(is64, 0b0010, vd, vn, vm);
    }
    pub fn fsub(&mut self, is64: bool, vd: VReg, vn: VReg, vm: VReg) {
        self.fp_bin(is64, 0b0011, vd, vn, vm);
    }
    pub fn fmul(&mut self, is64: bool, vd: VReg, vn: VReg, vm: VReg) {
        self.fp_bin(is64, 0b0000, vd, vn, vm);
    }
    pub fn fdiv(&mut self, is64: bool, vd: VReg, vn: VReg, vm: VReg) {
        self.fp_bin(is64, 0b0001, vd, vn, vm);
    }
    pub fn fmax(&mut self, is64: bool, vd: VReg, vn: VReg, vm: VReg) {
        self.fp_bin(is64, 0b0100, vd, vn, vm);
    }
    pub fn fmin(&mut self, is64: bool, vd: VReg, vn: VReg, vm: VReg) {
        self.fp_bin(is64, 0b0101, vd, vn, vm);
    }
    pub fn fmov_fp(&mut self, is64: bool, vd: VReg, vn: VReg) {
        if vd != vn {
            self.fp_un(is64, 0b00_0000, vd, vn);
        }
    }
    pub fn fabs(&mut self, is64: bool, vd: VReg, vn: VReg) {
        self.fp_un(is64, 0b00_0001, vd, vn);
    }
    pub fn fneg(&mut self, is64: bool, vd: VReg, vn: VReg) {
        self.fp_un(is64, 0b00_0010, vd, vn);
    }
    pub fn fsqrt(&mut self, is64: bool, vd: VReg, vn: VReg) {
        self.fp_un(is64, 0b00_0011, vd, vn);
    }
    pub fn frintn(&mut self, is64: bool, vd: VReg, vn: VReg) {
        self.fp_un(is64, 0b00_1000, vd, vn);
    }
    pub fn frintp(&mut self, is64: bool, vd: VReg, vn: VReg) {
        self.fp_un(is64, 0b00_1001, vd, vn);
    }
    pub fn frintm(&mut self, is64: bool, vd: VReg, vn: VReg) {
        self.fp_un(is64, 0b00_1010, vd, vn);
    }
    pub fn frintz(&mut self, is64: bool, vd: VReg, vn: VReg) {
        self.fp_un(is64, 0b00_1011, vd, vn);
    }
    /// `fcvt d, s` (f32 -> f64).
    pub fn fcvt_d_s(&mut self, vd: VReg, vn: VReg) {
        self.emit(0x1e22_c000 | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    /// `fcvt s, d` (f64 -> f32).
    pub fn fcvt_s_d(&mut self, vd: VReg, vn: VReg) {
        self.emit(0x1e62_4000 | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    pub fn fcmp(&mut self, is64: bool, vn: VReg, vm: VReg) {
        let ft = u32::from(is64) << 22;
        self.emit(0x1e20_2000 | ft | ((vm.0 as u32) << 16) | ((vn.0 as u32) << 5));
    }
    pub fn fcsel(&mut self, is64: bool, vd: VReg, vn: VReg, vm: VReg, cond: Cond) {
        let ft = u32::from(is64) << 22;
        self.emit(
            0x1e20_0c00
                | ft
                | ((vm.0 as u32) << 16)
                | ((cond as u32) << 12)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }

    pub fn scvtf(&mut self, int64: bool, fp64: bool, vd: VReg, rn: Reg) {
        let ft = u32::from(fp64) << 22;
        self.emit(Self::sf(int64) | 0x1e22_0000 | ft | ((rn.0 as u32) << 5) | vd.0 as u32);
    }
    pub fn ucvtf(&mut self, int64: bool, fp64: bool, vd: VReg, rn: Reg) {
        let ft = u32::from(fp64) << 22;
        self.emit(Self::sf(int64) | 0x1e23_0000 | ft | ((rn.0 as u32) << 5) | vd.0 as u32);
    }
    pub fn fcvtzs(&mut self, int64: bool, fp64: bool, rd: Reg, vn: VReg) {
        let ft = u32::from(fp64) << 22;
        self.emit(Self::sf(int64) | 0x1e38_0000 | ft | ((vn.0 as u32) << 5) | rd.0 as u32);
    }
    pub fn fcvtzu(&mut self, int64: bool, fp64: bool, rd: Reg, vn: VReg) {
        let ft = u32::from(fp64) << 22;
        self.emit(Self::sf(int64) | 0x1e39_0000 | ft | ((vn.0 as u32) << 5) | rd.0 as u32);
    }
    pub fn fmov_gp_from_fp(&mut self, is64: bool, rd: Reg, vn: VReg) {
        let ft = u32::from(is64) << 22;
        self.emit(Self::sf(is64) | 0x1e26_0000 | ft | ((vn.0 as u32) << 5) | rd.0 as u32);
    }
    pub fn fmov_fp_from_gp(&mut self, is64: bool, vd: VReg, rn: Reg) {
        let ft = u32::from(is64) << 22;
        self.emit(Self::sf(is64) | 0x1e27_0000 | ft | ((rn.0 as u32) << 5) | vd.0 as u32);
    }

    // ---- 128-bit AdvSIMD (NEON) ----

    /// `orr vd.16b, vn.16b, vm.16b` (`mov vd.16b, vn.16b` when `vn == vm`).
    pub fn orr_16b(&mut self, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(0x4ea0_1c00 | ((vm.0 as u32) << 16) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    pub fn mov_16b(&mut self, vd: VReg, vn: VReg) {
        if vd != vn {
            self.orr_16b(vd, vn, vn);
        }
    }
    pub fn and_16b(&mut self, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(0x4e20_1c00 | ((vm.0 as u32) << 16) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    pub fn bic_16b(&mut self, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(0x4e60_1c00 | ((vm.0 as u32) << 16) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    pub fn eor_16b(&mut self, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(0x6e20_1c00 | ((vm.0 as u32) << 16) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    pub fn bsl_16b(&mut self, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(0x6e60_1c00 | ((vm.0 as u32) << 16) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    pub fn not_16b(&mut self, vd: VReg, vn: VReg) {
        self.emit(0x6e20_5800 | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    /// `cnt vd.8b, vn.8b` (or `16b` when `q=true`): per-byte bit population count!
    pub fn cnt_v(&mut self, q: bool, vd: VReg, vn: VReg) {
        self.emit((u32::from(q) << 30) | 0x0e20_5800 | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    /// `addv bd, vn.8b`: unsigned sum of 8 bytes into `B0` of `vd`!
    pub fn addv_8b(&mut self, vd: VReg, vn: VReg) {
        self.emit(0x0e31_b800 | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    /// `uaddlp vd.<T>, vn.<2T>` (`size`: `0`=16b->8h, `1`=8h->4s, `2`=4s->2d).
    pub fn uaddlp_128(&mut self, size: u8, vd: VReg, vn: VReg) {
        self.emit(0x6e20_2800 | (((size & 3) as u32) << 22) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    /// `add vd.<T>, vn.<T>, vm.<T>` (`size`: `0`=16b, `1`=8h, `2`=4s, `3`=2d).
    pub fn add_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(
            0x4e20_8400
                | (((size & 3) as u32) << 22)
                | ((vm.0 as u32) << 16)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }
    pub fn sub_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(
            0x6e20_8400
                | (((size & 3) as u32) << 22)
                | ((vm.0 as u32) << 16)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }
    pub fn mul_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        // size: 0=16b, 1=8h, 2=4s
        self.emit(
            0x4e20_9c00
                | (((size & 3) as u32) << 22)
                | ((vm.0 as u32) << 16)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }
    /// `fadd/fsub/fmul/fdiv/fmax/fmin` for `4s` (`is64=false`) or `2d` (`is64=true`).
    pub fn fadd_v128(&mut self, is64: bool, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(
            0x4e20_d400
                | (u32::from(is64) << 22)
                | ((vm.0 as u32) << 16)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }
    pub fn fsub_v128(&mut self, is64: bool, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(
            0x4ea0_d400
                | (u32::from(is64) << 22)
                | ((vm.0 as u32) << 16)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }
    pub fn fmul_v128(&mut self, is64: bool, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(
            0x6e20_dc00
                | (u32::from(is64) << 22)
                | ((vm.0 as u32) << 16)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }
    pub fn fdiv_v128(&mut self, is64: bool, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(
            0x6e20_fc00
                | (u32::from(is64) << 22)
                | ((vm.0 as u32) << 16)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }
    pub fn fmax_v128(&mut self, is64: bool, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(
            0x4e20_f400
                | (u32::from(is64) << 22)
                | ((vm.0 as u32) << 16)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }
    pub fn fmin_v128(&mut self, is64: bool, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(
            0x4ea0_f400
                | (u32::from(is64) << 22)
                | ((vm.0 as u32) << 16)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }
    pub fn fcmgt_v128(&mut self, is64: bool, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(
            0x6ea0_e400
                | (u32::from(is64) << 22)
                | ((vm.0 as u32) << 16)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }
    pub fn fcmge_v128(&mut self, is64: bool, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(
            0x6e20_e400
                | (u32::from(is64) << 22)
                | ((vm.0 as u32) << 16)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }
    pub fn fcmeq_v128(&mut self, is64: bool, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(
            0x4e20_e400
                | (u32::from(is64) << 22)
                | ((vm.0 as u32) << 16)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }
    pub fn fabs_v128(&mut self, is64: bool, vd: VReg, vn: VReg) {
        self.emit(0x4ea0_f800 | (u32::from(is64) << 22) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    pub fn fneg_v128(&mut self, is64: bool, vd: VReg, vn: VReg) {
        self.emit(0x6ea0_f800 | (u32::from(is64) << 22) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    pub fn fsqrt_v128(&mut self, is64: bool, vd: VReg, vn: VReg) {
        self.emit(0x6ea1_f800 | (u32::from(is64) << 22) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    /// `dup vd.<T>, rn` (`size`: `0`=16b, `1`=8h, `2`=4s, `3`=2d).
    pub fn dup_gp_128(&mut self, size: u8, vd: VReg, rn: Reg) {
        let imm5 = 1u32 << size;
        self.emit(0x4e00_0c00 | (imm5 << 16) | ((rn.0 as u32) << 5) | vd.0 as u32);
    }
    /// `dup vd.<T>, vn.<t>[lane]` (`size`: `0`=16b, `1`=8h, `2`=4s, `3`=2d).
    pub fn dup_elt_128(&mut self, size: u8, vd: VReg, vn: VReg, lane: u8) {
        let imm5 = ((lane as u32) << (size + 1)) | (1u32 << size);
        self.emit(0x4e00_0400 | (imm5 << 16) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    /// `umov rd, vn.<t>[lane]` (`size`: `0`=b, `1`=h, `2`=s(W), `3`=d(X)).
    pub fn umov(&mut self, size: u8, rd: Reg, vn: VReg, lane: u8) {
        let q = u32::from(size == 3) << 30;
        let imm5 = ((lane as u32) << (size + 1)) | (1u32 << size);
        self.emit(q | 0x0e00_3c00 | (imm5 << 16) | ((vn.0 as u32) << 5) | rd.0 as u32);
    }
    /// `smov rd, vn.<t>[lane]` (`size`: `0`=b, `1`=h, `2`=s; `is64`: target Xd vs Wd).
    pub fn smov(&mut self, is64: bool, size: u8, rd: Reg, vn: VReg, lane: u8) {
        let q = u32::from(is64) << 30;
        let imm5 = ((lane as u32) << (size + 1)) | (1u32 << size);
        self.emit(q | 0x0e00_2c00 | (imm5 << 16) | ((vn.0 as u32) << 5) | rd.0 as u32);
    }
    /// `ins vd.<t>[lane], rn` (`size`: `0`=b, `1`=h, `2`=s, `3`=d).
    pub fn ins_gp(&mut self, size: u8, vd: VReg, lane: u8, rn: Reg) {
        let imm5 = ((lane as u32) << (size + 1)) | (1u32 << size);
        self.emit(0x4e00_1c00 | (imm5 << 16) | ((rn.0 as u32) << 5) | vd.0 as u32);
    }
    /// `ins vd.<t>[dst_lane], vn.<t>[src_lane]` (`size`: `0`=b, `1`=h, `2`=s, `3`=d).
    pub fn ins_elt(&mut self, size: u8, vd: VReg, dst_lane: u8, vn: VReg, src_lane: u8) {
        let imm5 = ((dst_lane as u32) << (size + 1)) | (1u32 << size);
        let imm4 = (src_lane as u32) << size;
        self.emit(0x6e00_0400 | (imm5 << 16) | (imm4 << 11) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    /// `movi vd.16b, #0` (zero 128-bit vector register).
    pub fn movi_zero_16b(&mut self, vd: VReg) {
        self.emit(0x4f00_0400 | vd.0 as u32);
    }

    /// `tbl vd.16b, {vn.16b}, vm.16b`: 16-byte table lookup (indices >= 16 yield 0).
    pub fn tbl_16b(&mut self, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(0x4e00_0000 | ((vm.0 as u32) << 16) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }

    /// `saddlp vd.<T>, vn.<2T>` (`size`: `0`=16b->8h, `1`=8h->4s, `2`=4s->2d).
    pub fn saddlp_128(&mut self, size: u8, vd: VReg, vn: VReg) {
        self.emit(0x4e20_2800 | (((size & 3) as u32) << 22) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }

    /// `addp vd.<T>, vn.<T>, vm.<T>` (`size`: `0`=16b, `1`=8h, `2`=4s, `3`=2d).
    pub fn addp_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(
            0x4e20_bc00
                | (((size & 3) as u32) << 22)
                | ((vm.0 as u32) << 16)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }

    /// `uminv rd, vn.<T>` across-lanes unsigned min (`size`: `0`=16b, `1`=8h, `2`=4s).
    pub fn uminv_128(&mut self, size: u8, vd: VReg, vn: VReg) {
        self.emit(0x6e31_a800 | (((size & 3) as u32) << 22) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }

    /// `abs vd.<T>, vn.<T>` (`size`: `0`=16b, `1`=8h, `2`=4s, `3`=2d).
    pub fn abs_v128(&mut self, size: u8, vd: VReg, vn: VReg) {
        self.emit(0x4e20_b800 | (((size & 3) as u32) << 22) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }

    /// `neg vd.<T>, vn.<T>` (`size`: `0`=16b, `1`=8h, `2`=4s, `3`=2d).
    pub fn neg_v128(&mut self, size: u8, vd: VReg, vn: VReg) {
        self.emit(0x6e20_b800 | (((size & 3) as u32) << 22) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }

    /// Helper for AdvSIMD three-same 128-bit integer ops (`0|Q=1|U|01110|size|1|Rm|opcode|1|Rn|Rd`).
    fn neon_3same(&mut self, base: u32, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(
            base | (((size & 3) as u32) << 22)
                | ((vm.0 as u32) << 16)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }

    pub fn cmeq_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.neon_3same(0x6e20_8c00, size, vd, vn, vm);
    }
    pub fn cmgt_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.neon_3same(0x4e20_3400, size, vd, vn, vm);
    }
    pub fn cmge_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.neon_3same(0x4e20_3c00, size, vd, vn, vm);
    }
    pub fn cmhi_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.neon_3same(0x6e20_3400, size, vd, vn, vm);
    }
    pub fn cmhs_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.neon_3same(0x6e20_3c00, size, vd, vn, vm);
    }
    pub fn sqadd_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.neon_3same(0x4e20_0c00, size, vd, vn, vm);
    }
    pub fn uqadd_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.neon_3same(0x6e20_0c00, size, vd, vn, vm);
    }
    pub fn sqsub_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.neon_3same(0x4e20_2c00, size, vd, vn, vm);
    }
    pub fn uqsub_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.neon_3same(0x6e20_2c00, size, vd, vn, vm);
    }
    pub fn smax_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.neon_3same(0x4e20_6400, size, vd, vn, vm);
    }
    pub fn umax_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.neon_3same(0x6e20_6400, size, vd, vn, vm);
    }
    pub fn smin_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.neon_3same(0x4e20_6c00, size, vd, vn, vm);
    }
    pub fn umin_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.neon_3same(0x6e20_6c00, size, vd, vn, vm);
    }
    pub fn urhadd_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.neon_3same(0x6e20_1400, size, vd, vn, vm);
    }
    pub fn sqrdmulh_8h(&mut self, vd: VReg, vn: VReg, vm: VReg) {
        self.neon_3same(0x6e20_b400, 1, vd, vn, vm);
    }
    pub fn sshl_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.neon_3same(0x4e20_4400, size, vd, vn, vm);
    }
    pub fn ushl_v128(&mut self, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.neon_3same(0x6e20_4400, size, vd, vn, vm);
    }

    /// `sshr vd.<T>, vn.<T>, #shift` (`size`: `0`=16b, `1`=8h, `2`=4s, `3`=2d; `1 <= shift <= 8<<size`).
    pub fn sshr_imm_v128(&mut self, size: u8, vd: VReg, vn: VReg, shift: u8) {
        let enc = (16u32 << size) - u32::from(shift);
        self.emit(0x4f00_0400 | (enc << 16) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }

    /// `sxtl` / `sxtl2` / `uxtl` / `uxtl2` (`size`: `0`=8b->8h, `1`=4h->4s, `2`=2s->2d).
    pub fn xtl_128(&mut self, unsigned: bool, high: bool, size: u8, vd: VReg, vn: VReg) {
        let immh = 1u32 << size;
        self.emit(
            0x0f00_a400
                | (u32::from(high) << 30)
                | (u32::from(unsigned) << 29)
                | (immh << 19)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }

    /// `sqxtn` / `sqxtn2` (signed saturating narrow, `size`: `0`=8h->8b, `1`=4s->4h).
    pub fn sqxtn_128(&mut self, high: bool, size: u8, vd: VReg, vn: VReg) {
        self.emit(
            0x0e21_4800
                | (u32::from(high) << 30)
                | (((size & 3) as u32) << 22)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }

    /// `sqxtun` / `sqxtun2` (signed-to-unsigned saturating narrow, `size`: `0`=8h->8b, `1`=4s->4h).
    pub fn sqxtun_128(&mut self, high: bool, size: u8, vd: VReg, vn: VReg) {
        self.emit(
            0x2e21_2800
                | (u32::from(high) << 30)
                | (((size & 3) as u32) << 22)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }

    /// `smull` / `smull2` / `umull` / `umull2` (`size`: `0`=8b->8h, `1`=4h->4s, `2`=2s->2d).
    pub fn mull_128(&mut self, unsigned: bool, high: bool, size: u8, vd: VReg, vn: VReg, vm: VReg) {
        self.emit(
            0x0e20_c000
                | (u32::from(high) << 30)
                | (u32::from(unsigned) << 29)
                | (((size & 3) as u32) << 22)
                | ((vm.0 as u32) << 16)
                | ((vn.0 as u32) << 5)
                | vd.0 as u32,
        );
    }

    pub fn frintn_v128(&mut self, is64: bool, vd: VReg, vn: VReg) {
        self.emit(0x4e21_8800 | (u32::from(is64) << 22) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    pub fn frintm_v128(&mut self, is64: bool, vd: VReg, vn: VReg) {
        self.emit(0x4e21_9800 | (u32::from(is64) << 22) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    pub fn frintp_v128(&mut self, is64: bool, vd: VReg, vn: VReg) {
        self.emit(0x4ea1_8800 | (u32::from(is64) << 22) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    pub fn frintz_v128(&mut self, is64: bool, vd: VReg, vn: VReg) {
        self.emit(0x4ea1_9800 | (u32::from(is64) << 22) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    pub fn scvtf_v128(&mut self, is64: bool, vd: VReg, vn: VReg) {
        self.emit(0x4e21_d800 | (u32::from(is64) << 22) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    pub fn ucvtf_v128(&mut self, is64: bool, vd: VReg, vn: VReg) {
        self.emit(0x6e21_d800 | (u32::from(is64) << 22) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    pub fn fcvtzs_v128(&mut self, is64: bool, vd: VReg, vn: VReg) {
        self.emit(0x4ea1_b800 | (u32::from(is64) << 22) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    pub fn fcvtzu_v128(&mut self, is64: bool, vd: VReg, vn: VReg) {
        self.emit(0x6ea1_b800 | (u32::from(is64) << 22) | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    /// `fcvtn vd.2s, vn.2d` (demotes 2 f64s to 2 f32s and zeroes upper 64 bits!).
    pub fn fcvtn_2s_2d(&mut self, vd: VReg, vn: VReg) {
        self.emit(0x0e61_6800 | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
    /// `fcvtl vd.2d, vn.2s` (promotes low 2 f32s to 2 f64s).
    pub fn fcvtl_2d_2s(&mut self, vd: VReg, vn: VReg) {
        self.emit(0x0e61_7800 | ((vn.0 as u32) << 5) | vd.0 as u32);
    }
}

/// Patches all link-time relocations inside `code_slice` given resolved `.text` and `.rodata`
/// virtual addresses.
pub fn patch_a64_relocs(
    code: &mut [u8],
    text_vaddr: u64,
    fn_text_off: usize,
    relocs: &[A64Reloc],
    resolve: impl Fn(A64RelocTarget) -> Result<u64, String>,
) -> Result<(), String> {
    for r in relocs {
        let abs_off = fn_text_off + r.patch_off;
        let pc = text_vaddr + abs_off as u64;
        let target = resolve(r.target)?;
        match r.kind {
            A64RelocKind::Branch26 => {
                let diff = (target as i64) - (pc as i64);
                let imm = diff >> 2;
                if !(-(1i64 << 25)..(1i64 << 25)).contains(&imm) {
                    return Err(format!("aarch64: link b/bl offset {diff} out of ±128 MiB"));
                }
                let mut insn = u32::from_le_bytes(code[abs_off..abs_off + 4].try_into().unwrap());
                insn = (insn & 0xfc00_0000) | ((imm as u32) & 0x03ff_ffff);
                code[abs_off..abs_off + 4].copy_from_slice(&insn.to_le_bytes());
            }
            A64RelocKind::AdrpAdd => {
                let pc_page = pc & !0xfff;
                let tgt_page = target & !0xfff;
                let page_diff = ((tgt_page as i64) - (pc_page as i64)) >> 12;
                if !(-(1i64 << 20)..(1i64 << 20)).contains(&page_diff) {
                    return Err(format!(
                        "aarch64: adrp page delta {page_diff} out of ±4 GiB"
                    ));
                }
                let pd = page_diff as u32;
                let immlo = pd & 3;
                let immhi = (pd >> 2) & 0x7_ffff;
                let mut adrp = u32::from_le_bytes(code[abs_off..abs_off + 4].try_into().unwrap());
                adrp = (adrp & 0x9f00_001f) | (immlo << 29) | (immhi << 5);
                code[abs_off..abs_off + 4].copy_from_slice(&adrp.to_le_bytes());

                let lo12 = (target & 0xfff) as u32;
                let mut add =
                    u32::from_le_bytes(code[abs_off + 4..abs_off + 8].try_into().unwrap());
                add = (add & 0xffc0_03ff) | (lo12 << 10);
                code[abs_off + 4..abs_off + 8].copy_from_slice(&add.to_le_bytes());
            }
        }
    }
    Ok(())
}
