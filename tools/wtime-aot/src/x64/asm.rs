//! Pure-Rust x86-64 machine-code assembler (`#![forbid(unsafe_code)]`).
//!
//! Emits position-independent x86-64 + SSE2 + SSE4.1 + POPCNT + BMI1/ABM
//! machine code with local labels, forward/backward branch fixups, and
//! cross-section PC-relative relocations (`Func`, `Rodata`, `Helper`).

#![allow(dead_code)]

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Gp {
    Rax = 0,
    Rcx = 1,
    Rdx = 2,
    Rbx = 3,
    Rsp = 4,
    Rbp = 5,
    Rsi = 6,
    Rdi = 7,
    R8 = 8,
    R9 = 9,
    R10 = 10,
    R11 = 11,
    R12 = 12,
    R13 = 13,
    R14 = 14,
    R15 = 15,
}

impl Gp {
    #[inline]
    pub const fn id(self) -> u8 {
        self as u8
    }

    #[inline]
    pub const fn lo(self) -> u8 {
        (self as u8) & 7
    }

    #[inline]
    pub const fn hi(self) -> u8 {
        (self as u8) >> 3
    }

    pub const fn from_id(id: u8) -> Self {
        match id {
            0 => Gp::Rax,
            1 => Gp::Rcx,
            2 => Gp::Rdx,
            3 => Gp::Rbx,
            4 => Gp::Rsp,
            5 => Gp::Rbp,
            6 => Gp::Rsi,
            7 => Gp::Rdi,
            8 => Gp::R8,
            9 => Gp::R9,
            10 => Gp::R10,
            11 => Gp::R11,
            12 => Gp::R12,
            13 => Gp::R13,
            14 => Gp::R14,
            _ => Gp::R15,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Xmm {
    Xmm0 = 0,
    Xmm1 = 1,
    Xmm2 = 2,
    Xmm3 = 3,
    Xmm4 = 4,
    Xmm5 = 5,
    Xmm6 = 6,
    Xmm7 = 7,
    Xmm8 = 8,
    Xmm9 = 9,
    Xmm10 = 10,
    Xmm11 = 11,
    Xmm12 = 12,
    Xmm13 = 13,
    Xmm14 = 14,
    Xmm15 = 15,
}

impl Xmm {
    #[inline]
    pub const fn id(self) -> u8 {
        self as u8
    }

    #[inline]
    pub const fn lo(self) -> u8 {
        (self as u8) & 7
    }

    #[inline]
    pub const fn hi(self) -> u8 {
        (self as u8) >> 3
    }

    pub const fn from_id(id: u8) -> Self {
        match id {
            0 => Xmm::Xmm0,
            1 => Xmm::Xmm1,
            2 => Xmm::Xmm2,
            3 => Xmm::Xmm3,
            4 => Xmm::Xmm4,
            5 => Xmm::Xmm5,
            6 => Xmm::Xmm6,
            7 => Xmm::Xmm7,
            8 => Xmm::Xmm8,
            9 => Xmm::Xmm9,
            10 => Xmm::Xmm10,
            11 => Xmm::Xmm11,
            12 => Xmm::Xmm12,
            13 => Xmm::Xmm13,
            14 => Xmm::Xmm14,
            _ => Xmm::Xmm15,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Cond {
    O = 0x0,
    No = 0x1,
    B = 0x2,  // unsigned <
    Ae = 0x3, // unsigned >=
    E = 0x4,  // ==
    Ne = 0x5, // !=
    Be = 0x6, // unsigned <=
    A = 0x7,  // unsigned >
    S = 0x8,
    Ns = 0x9,
    P = 0xa,  // parity (NaN after ucomis*)
    Np = 0xb, // not parity
    L = 0xc,  // signed <
    Ge = 0xd, // signed >=
    Le = 0xe, // signed <=
    G = 0xf,  // signed >
}

impl Cond {
    #[inline]
    pub const fn invert(self) -> Self {
        match self {
            Cond::O => Cond::No,
            Cond::No => Cond::O,
            Cond::B => Cond::Ae,
            Cond::Ae => Cond::B,
            Cond::E => Cond::Ne,
            Cond::Ne => Cond::E,
            Cond::Be => Cond::A,
            Cond::A => Cond::Be,
            Cond::S => Cond::Ns,
            Cond::Ns => Cond::S,
            Cond::P => Cond::Np,
            Cond::Np => Cond::P,
            Cond::L => Cond::Ge,
            Cond::Ge => Cond::L,
            Cond::Le => Cond::G,
            Cond::G => Cond::Le,
        }
    }
}

/// Memory operand `[base + disp]` or `[base + index*scale + disp]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mem {
    BaseDisp { base: Gp, disp: i32 },
    BaseIndexDisp { base: Gp, index: Gp, scale: u8, disp: i32 },
}

impl Mem {
    #[inline]
    pub const fn base(base: Gp) -> Self {
        Mem::BaseDisp { base, disp: 0 }
    }

    #[inline]
    pub const fn base_disp(base: Gp, disp: i32) -> Self {
        Mem::BaseDisp { base, disp }
    }

    #[inline]
    pub const fn indexed(base: Gp, index: Gp, scale: u8, disp: i32) -> Self {
        debug_assert!(matches!(scale, 1 | 2 | 4 | 8));
        debug_assert!(!matches!(index, Gp::Rsp));
        Mem::BaseIndexDisp { base, index, scale, disp }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Label(pub u32);

/// Relocation target resolved when linking `.text` and `.rodata` into the ELF `.so`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelocTarget {
    /// Defined Wasm function (index into `Module::types`, i.e. `>= imports.len()` or import trampoline).
    Func(u32),
    /// Shared runtime helper function ID (resolved by `mod.rs`).
    Helper(u32),
    /// Byte offset inside `.rodata`.
    Rodata(u32),
    /// Byte offset inside local per-function `.rodata` blob (rebased when merging `.rodata`).
    LocalRodata(u32),
}

/// A PC-relative 32-bit displacement inside emitted code:
/// `*(i32*)&code[patch_off] = (target_vaddr - (fn_vaddr + next_ip_off)) as i32`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reloc {
    pub patch_off: u32,
    pub next_ip_off: u32,
    pub target: RelocTarget,
}

/// A jump-table entry stored in local `.rodata` that holds `(label_vaddr - table_base_vaddr) as i32`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JumpTablePatch {
    pub rodata_off: u32,
    pub table_rodata_off: u32,
    pub label: Label,
}

#[derive(Clone, Copy, Debug)]
struct LocalFixup {
    patch_off: u32,
    next_ip_off: u32,
    label: Label,
}

pub struct Asm {
    pub code: Vec<u8>,
    pub relocs: Vec<Reloc>,
    pub local_rodata: Vec<u8>,
    pub jt_patches: Vec<JumpTablePatch>,
    labels: Vec<Option<u32>>,
    fixups: Vec<LocalFixup>,
}

impl Asm {
    pub fn new() -> Self {
        Self {
            code: Vec::with_capacity(256),
            relocs: Vec::new(),
            local_rodata: Vec::new(),
            jt_patches: Vec::new(),
            labels: Vec::new(),
            fixups: Vec::new(),
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            code: Vec::with_capacity(cap),
            relocs: Vec::new(),
            local_rodata: Vec::new(),
            jt_patches: Vec::new(),
            labels: Vec::new(),
            fixups: Vec::new(),
        }
    }

    #[inline]
    pub fn pos(&self) -> u32 {
        self.code.len() as u32
    }

    #[inline]
    pub fn new_label(&mut self) -> Label {
        let id = self.labels.len() as u32;
        self.labels.push(None);
        Label(id)
    }

    #[inline]
    pub fn bind(&mut self, label: Label) {
        debug_assert!(self.labels[label.0 as usize].is_none(), "label bound twice");
        self.labels[label.0 as usize] = Some(self.code.len() as u32);
    }

    /// Resolves all local branch fixups. Must be called before linking.
    pub fn finish_labels(&mut self) -> Result<(), String> {
        for f in &self.fixups {
            let target = self.labels[f.label.0 as usize].ok_or_else(|| format!("unbound label {}", f.label.0))?;
            let rel = (target as i64) - (f.next_ip_off as i64);
            let rel32 = i32::try_from(rel).map_err(|_| "local branch displacement exceeds rel32".to_string())?;
            let p = f.patch_off as usize;
            self.code[p..p + 4].copy_from_slice(&rel32.to_le_bytes());
        }
        self.fixups.clear();
        Ok(())
    }

    /// Resolves jump-table entries in `local_rodata` once `fn_vaddr` and `local_rodata_vaddr` are known.
    pub fn patch_jump_tables(&mut self, fn_vaddr: u64, local_rodata_vaddr: u64) -> Result<(), String> {
        for jt in &self.jt_patches {
            let label_off = self.labels[jt.label.0 as usize].ok_or_else(|| format!("unbound jt label {}", jt.label.0))?;
            let target_vaddr = fn_vaddr + u64::from(label_off);
            let base_vaddr = local_rodata_vaddr + u64::from(jt.table_rodata_off);
            let rel = (target_vaddr as i64) - (base_vaddr as i64);
            let rel32 = i32::try_from(rel).map_err(|_| "jump table displacement exceeds rel32".to_string())?;
            let p = jt.rodata_off as usize;
            self.local_rodata[p..p + 4].copy_from_slice(&rel32.to_le_bytes());
        }
        Ok(())
    }

    /// Appends aligned raw bytes to `local_rodata`, returning the starting offset.
    pub fn add_local_rodata(&mut self, align: usize, bytes: &[u8]) -> u32 {
        let mask = align.saturating_sub(1);
        while (self.local_rodata.len() & mask) != 0 {
            self.local_rodata.push(0);
        }
        let off = self.local_rodata.len() as u32;
        self.local_rodata.extend_from_slice(bytes);
        off
    }

    /// Appends a rel32 jump table for `targets` to `local_rodata`, returning the table base offset.
    pub fn add_jump_table(&mut self, targets: &[Label]) -> u32 {
        while (self.local_rodata.len() & 3) != 0 {
            self.local_rodata.push(0);
        }
        let base = self.local_rodata.len() as u32;
        for &lbl in targets {
            let off = self.local_rodata.len() as u32;
            self.local_rodata.extend_from_slice(&0i32.to_le_bytes());
            self.jt_patches.push(JumpTablePatch { rodata_off: off, table_rodata_off: base, label: lbl });
        }
        base
    }

    #[inline]
    pub fn emit_u8(&mut self, b: u8) {
        self.code.push(b);
    }

    #[inline]
    pub fn emit_u32(&mut self, v: u32) {
        self.code.extend_from_slice(&v.to_le_bytes());
    }

    #[inline]
    pub fn emit_i32(&mut self, v: i32) {
        self.code.extend_from_slice(&v.to_le_bytes());
    }

    #[inline]
    pub fn emit_u64(&mut self, v: u64) {
        self.code.extend_from_slice(&v.to_le_bytes());
    }

    #[inline]
    fn rex(&mut self, w: bool, r: u8, x: u8, b: u8, force: bool) {
        let v = 0x40 | (u8::from(w) << 3) | ((r & 1) << 2) | ((x & 1) << 1) | (b & 1);
        if v != 0x40 || force {
            self.code.push(v);
        }
    }

    #[inline]
    fn modrm_reg(&mut self, reg_lo: u8, rm_lo: u8) {
        self.code.push(0xc0 | ((reg_lo & 7) << 3) | (rm_lo & 7));
    }

    fn rex_mem(&mut self, w: bool, reg_hi: u8, mem: Mem, force: bool) {
        match mem {
            Mem::BaseDisp { base, .. } => self.rex(w, reg_hi, 0, base.hi(), force),
            Mem::BaseIndexDisp { base, index, .. } => self.rex(w, reg_hi, index.hi(), base.hi(), force),
        }
    }

    fn modrm_mem(&mut self, reg_lo: u8, mem: Mem) {
        let reg_bits = (reg_lo & 7) << 3;
        match mem {
            Mem::BaseDisp { base, disp } => {
                let blo = base.lo();
                let need_sib = blo == 4; // RSP / R12
                let (m, dlen) = if disp == 0 && blo != 5 {
                    (0x00, 0)
                } else if i8::try_from(disp).is_ok() {
                    (0x40, 1)
                } else {
                    (0x80, 4)
                };
                self.code.push(m | reg_bits | blo);
                if need_sib {
                    self.code.push(0x24); // scale=1, index=none(4), base=4
                }
                match dlen {
                    1 => self.code.push(disp as i8 as u8),
                    4 => self.emit_i32(disp),
                    _ => {}
                }
            }
            Mem::BaseIndexDisp { base, index, scale, disp } => {
                debug_assert!(!matches!(index, Gp::Rsp));
                let blo = base.lo();
                let ilo = index.lo();
                let sbits = match scale {
                    1 => 0u8,
                    2 => 1u8,
                    4 => 2u8,
                    8 => 3u8,
                    _ => unreachable!(),
                };
                let (m, dlen) = if disp == 0 && blo != 5 {
                    (0x00, 0)
                } else if i8::try_from(disp).is_ok() {
                    (0x40, 1)
                } else {
                    (0x80, 4)
                };
                self.code.push(m | reg_bits | 4); // rm=4 => SIB
                self.code.push((sbits << 6) | (ilo << 3) | blo);
                match dlen {
                    1 => self.code.push(disp as i8 as u8),
                    4 => self.emit_i32(disp),
                    _ => {}
                }
            }
        }
    }

    // ---- Basic stack / control / misc ----

    pub fn nop(&mut self) {
        self.emit_u8(0x90);
    }

    pub fn ud2(&mut self) {
        self.emit_u8(0x0f);
        self.emit_u8(0x0b);
    }

    pub fn ret(&mut self) {
        self.emit_u8(0xc3);
    }

    pub fn syscall(&mut self) {
        self.emit_u8(0x0f);
        self.emit_u8(0x05);
    }

    pub fn push(&mut self, r: Gp) {
        if r.hi() != 0 {
            self.emit_u8(0x41);
        }
        self.emit_u8(0x50 + r.lo());
    }

    pub fn pop(&mut self, r: Gp) {
        if r.hi() != 0 {
            self.emit_u8(0x41);
        }
        self.emit_u8(0x58 + r.lo());
    }

    pub fn rep_movsb(&mut self) {
        self.emit_u8(0xf3);
        self.emit_u8(0xa4);
    }

    pub fn rep_stosb(&mut self) {
        self.emit_u8(0xf3);
        self.emit_u8(0xaa);
    }

    // ---- Branches & calls ----

    pub fn jmp(&mut self, label: Label) {
        self.emit_u8(0xe9);
        let patch_off = self.pos();
        self.emit_i32(0);
        let next_ip_off = self.pos();
        self.fixups.push(LocalFixup { patch_off, next_ip_off, label });
    }

    pub fn jcc(&mut self, cond: Cond, label: Label) {
        self.emit_u8(0x0f);
        self.emit_u8(0x80 + cond as u8);
        let patch_off = self.pos();
        self.emit_i32(0);
        let next_ip_off = self.pos();
        self.fixups.push(LocalFixup { patch_off, next_ip_off, label });
    }

    pub fn jmp_reg(&mut self, r: Gp) {
        self.rex(false, 0, 0, r.hi(), false);
        self.emit_u8(0xff);
        self.modrm_reg(4, r.lo());
    }

    pub fn call_reloc(&mut self, target: RelocTarget) {
        self.emit_u8(0xe8);
        let patch_off = self.pos();
        self.emit_i32(0);
        let next_ip_off = self.pos();
        self.relocs.push(Reloc { patch_off, next_ip_off, target });
    }

    pub fn jmp_reloc(&mut self, target: RelocTarget) {
        self.emit_u8(0xe9);
        let patch_off = self.pos();
        self.emit_i32(0);
        let next_ip_off = self.pos();
        self.relocs.push(Reloc { patch_off, next_ip_off, target });
    }

    pub fn call_reg(&mut self, r: Gp) {
        self.rex(false, 0, 0, r.hi(), false);
        self.emit_u8(0xff);
        self.modrm_reg(2, r.lo());
    }

    pub fn call_mem(&mut self, mem: Mem) {
        self.rex_mem(false, 0, mem, false);
        self.emit_u8(0xff);
        self.modrm_mem(2, mem);
    }

    // ---- RIP-relative loads / lea ----

    pub fn lea_rip(&mut self, dst: Gp, target: RelocTarget) {
        self.rex(true, dst.hi(), 0, 0, false);
        self.emit_u8(0x8d);
        self.emit_u8((dst.lo() << 3) | 5);
        let patch_off = self.pos();
        self.emit_i32(0);
        let next_ip_off = self.pos();
        self.relocs.push(Reloc { patch_off, next_ip_off, target });
    }

    pub fn movss_rip(&mut self, dst: Xmm, target: RelocTarget) {
        self.emit_u8(0xf3);
        self.rex(false, dst.hi(), 0, 0, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x10);
        self.emit_u8((dst.lo() << 3) | 5);
        let patch_off = self.pos();
        self.emit_i32(0);
        let next_ip_off = self.pos();
        self.relocs.push(Reloc { patch_off, next_ip_off, target });
    }

    pub fn movsd_rip(&mut self, dst: Xmm, target: RelocTarget) {
        self.emit_u8(0xf2);
        self.rex(false, dst.hi(), 0, 0, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x10);
        self.emit_u8((dst.lo() << 3) | 5);
        let patch_off = self.pos();
        self.emit_i32(0);
        let next_ip_off = self.pos();
        self.relocs.push(Reloc { patch_off, next_ip_off, target });
    }

    pub fn movdqu_rip(&mut self, dst: Xmm, target: RelocTarget) {
        self.emit_u8(0xf3);
        self.rex(false, dst.hi(), 0, 0, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x6f);
        self.emit_u8((dst.lo() << 3) | 5);
        let patch_off = self.pos();
        self.emit_i32(0);
        let next_ip_off = self.pos();
        self.relocs.push(Reloc { patch_off, next_ip_off, target });
    }

    pub fn xmm_rip_op(&mut self, prefix: Option<u8>, op: u8, dst: Xmm, target: RelocTarget) {
        if let Some(p) = prefix {
            self.emit_u8(p);
        }
        self.rex(false, dst.hi(), 0, 0, false);
        self.emit_u8(0x0f);
        self.emit_u8(op);
        self.emit_u8((dst.lo() << 3) | 5);
        let patch_off = self.pos();
        self.emit_i32(0);
        let next_ip_off = self.pos();
        self.relocs.push(Reloc { patch_off, next_ip_off, target });
    }

    // ---- GP moves, loads, stores, lea ----

    pub fn mov_rr(&mut self, is64: bool, dst: Gp, src: Gp) {
        if is64 && dst == src {
            return;
        }
        self.rex(is64, src.hi(), 0, dst.hi(), false);
        self.emit_u8(0x89);
        self.modrm_reg(src.lo(), dst.lo());
    }

    pub fn mov_ri32(&mut self, dst: Gp, imm: u32) {
        if imm == 0 {
            self.xor_rr(false, dst, dst);
            return;
        }
        self.rex(false, 0, 0, dst.hi(), false);
        self.emit_u8(0xb8 + dst.lo());
        self.emit_u32(imm);
    }

    pub fn mov_ri64(&mut self, dst: Gp, imm: u64) {
        if imm == 0 {
            self.xor_rr(false, dst, dst);
        } else if imm <= u32::MAX as u64 {
            self.mov_ri32(dst, imm as u32);
        } else if (imm as i64) >= i32::MIN as i64 && (imm as i64) <= i32::MAX as i64 {
            // Sign-extended 32-bit immediate (7 bytes vs 10).
            self.rex(true, 0, 0, dst.hi(), false);
            self.emit_u8(0xc7);
            self.modrm_reg(0, dst.lo());
            self.emit_i32(imm as i32);
        } else {
            self.rex(true, 0, 0, dst.hi(), false);
            self.emit_u8(0xb8 + dst.lo());
            self.emit_u64(imm);
        }
    }

    pub fn mov_rm(&mut self, is64: bool, dst: Gp, mem: Mem) {
        self.rex_mem(is64, dst.hi(), mem, false);
        self.emit_u8(0x8b);
        self.modrm_mem(dst.lo(), mem);
    }

    pub fn mov_mr(&mut self, is64: bool, mem: Mem, src: Gp) {
        self.rex_mem(is64, src.hi(), mem, false);
        self.emit_u8(0x89);
        self.modrm_mem(src.lo(), mem);
    }

    pub fn mov_mr8(&mut self, mem: Mem, src: Gp) {
        self.rex_mem(false, src.hi(), mem, src.id() >= 4);
        self.emit_u8(0x88);
        self.modrm_mem(src.lo(), mem);
    }

    pub fn mov_mr16(&mut self, mem: Mem, src: Gp) {
        self.emit_u8(0x66);
        self.rex_mem(false, src.hi(), mem, false);
        self.emit_u8(0x89);
        self.modrm_mem(src.lo(), mem);
    }

    pub fn mov_mi32(&mut self, is64: bool, mem: Mem, imm: i32) {
        self.rex_mem(is64, 0, mem, false);
        self.emit_u8(0xc7);
        self.modrm_mem(0, mem);
        self.emit_i32(imm);
    }

    pub fn mov_mi8(&mut self, mem: Mem, imm: u8) {
        self.rex_mem(false, 0, mem, false);
        self.emit_u8(0xc6);
        self.modrm_mem(0, mem);
        self.emit_u8(imm);
    }

    pub fn movzx_rm8(&mut self, is64: bool, dst: Gp, mem: Mem) {
        self.rex_mem(is64, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0xb6);
        self.modrm_mem(dst.lo(), mem);
    }

    pub fn movzx_rm16(&mut self, is64: bool, dst: Gp, mem: Mem) {
        self.rex_mem(is64, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0xb7);
        self.modrm_mem(dst.lo(), mem);
    }

    pub fn movsx_rm8(&mut self, is64: bool, dst: Gp, mem: Mem) {
        self.rex_mem(is64, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0xbe);
        self.modrm_mem(dst.lo(), mem);
    }

    pub fn movsx_rm16(&mut self, is64: bool, dst: Gp, mem: Mem) {
        self.rex_mem(is64, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0xbf);
        self.modrm_mem(dst.lo(), mem);
    }

    pub fn movsxd_rm(&mut self, dst: Gp, mem: Mem) {
        self.rex_mem(true, dst.hi(), mem, false);
        self.emit_u8(0x63);
        self.modrm_mem(dst.lo(), mem);
    }

    pub fn movzx_rr8(&mut self, dst: Gp, src: Gp) {
        self.rex(false, dst.hi(), 0, src.hi(), src.id() >= 4);
        self.emit_u8(0x0f);
        self.emit_u8(0xb6);
        self.modrm_reg(dst.lo(), src.lo());
    }

    pub fn movsx_rr8(&mut self, is64: bool, dst: Gp, src: Gp) {
        self.rex(is64, dst.hi(), 0, src.hi(), src.id() >= 4);
        self.emit_u8(0x0f);
        self.emit_u8(0xbe);
        self.modrm_reg(dst.lo(), src.lo());
    }

    pub fn movsx_rr16(&mut self, is64: bool, dst: Gp, src: Gp) {
        self.rex(is64, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0xbf);
        self.modrm_reg(dst.lo(), src.lo());
    }

    pub fn movsxd_rr(&mut self, dst: Gp, src: Gp) {
        self.rex(true, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x63);
        self.modrm_reg(dst.lo(), src.lo());
    }

    pub fn lea(&mut self, is64: bool, dst: Gp, mem: Mem) {
        self.rex_mem(is64, dst.hi(), mem, false);
        self.emit_u8(0x8d);
        self.modrm_mem(dst.lo(), mem);
    }

    // ---- GP ALU ----

    fn alu_rr(&mut self, op: u8, is64: bool, dst: Gp, src: Gp) {
        self.rex(is64, src.hi(), 0, dst.hi(), false);
        self.emit_u8(op);
        self.modrm_reg(src.lo(), dst.lo());
    }

    fn alu_ri(&mut self, ext: u8, is64: bool, dst: Gp, imm: i32) {
        self.rex(is64, 0, 0, dst.hi(), false);
        if let Ok(b) = i8::try_from(imm) {
            self.emit_u8(0x83);
            self.modrm_reg(ext, dst.lo());
            self.emit_u8(b as u8);
        } else {
            self.emit_u8(0x81);
            self.modrm_reg(ext, dst.lo());
            self.emit_i32(imm);
        }
    }

    pub fn add_rr(&mut self, is64: bool, dst: Gp, src: Gp) { self.alu_rr(0x01, is64, dst, src); }
    pub fn or_rr(&mut self, is64: bool, dst: Gp, src: Gp)  { self.alu_rr(0x09, is64, dst, src); }
    pub fn and_rr(&mut self, is64: bool, dst: Gp, src: Gp) { self.alu_rr(0x21, is64, dst, src); }
    pub fn sub_rr(&mut self, is64: bool, dst: Gp, src: Gp) { self.alu_rr(0x29, is64, dst, src); }
    pub fn xor_rr(&mut self, is64: bool, dst: Gp, src: Gp) { self.alu_rr(0x31, is64, dst, src); }
    pub fn cmp_rr(&mut self, is64: bool, a: Gp, b: Gp)     { self.alu_rr(0x39, is64, a, b); }

    pub fn add_ri(&mut self, is64: bool, dst: Gp, imm: i32) { self.alu_ri(0, is64, dst, imm); }
    pub fn or_ri(&mut self, is64: bool, dst: Gp, imm: i32)  { self.alu_ri(1, is64, dst, imm); }
    pub fn and_ri(&mut self, is64: bool, dst: Gp, imm: i32) { self.alu_ri(4, is64, dst, imm); }
    pub fn sub_ri(&mut self, is64: bool, dst: Gp, imm: i32) { self.alu_ri(5, is64, dst, imm); }
    pub fn xor_ri(&mut self, is64: bool, dst: Gp, imm: i32) { self.alu_ri(6, is64, dst, imm); }
    pub fn cmp_ri(&mut self, is64: bool, a: Gp, imm: i32)   { self.alu_ri(7, is64, a, imm); }

    pub fn cmp_rm(&mut self, is64: bool, a: Gp, mem: Mem) {
        self.rex_mem(is64, a.hi(), mem, false);
        self.emit_u8(0x3b);
        self.modrm_mem(a.lo(), mem);
    }

    pub fn test_rr(&mut self, is64: bool, a: Gp, b: Gp) {
        self.rex(is64, b.hi(), 0, a.hi(), false);
        self.emit_u8(0x85);
        self.modrm_reg(b.lo(), a.lo());
    }

    pub fn imul_rr(&mut self, is64: bool, dst: Gp, src: Gp) {
        self.rex(is64, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0xaf);
        self.modrm_reg(dst.lo(), src.lo());
    }

    pub fn imul_rri(&mut self, is64: bool, dst: Gp, src: Gp, imm: i32) {
        self.rex(is64, dst.hi(), 0, src.hi(), false);
        if let Ok(b) = i8::try_from(imm) {
            self.emit_u8(0x6b);
            self.modrm_reg(dst.lo(), src.lo());
            self.emit_u8(b as u8);
        } else {
            self.emit_u8(0x69);
            self.modrm_reg(dst.lo(), src.lo());
            self.emit_i32(imm);
        }
    }

    pub fn neg(&mut self, is64: bool, r: Gp) {
        self.rex(is64, 0, 0, r.hi(), false);
        self.emit_u8(0xf7);
        self.modrm_reg(3, r.lo());
    }

    pub fn not(&mut self, is64: bool, r: Gp) {
        self.rex(is64, 0, 0, r.hi(), false);
        self.emit_u8(0xf7);
        self.modrm_reg(2, r.lo());
    }

    pub fn cdq(&mut self) {
        self.emit_u8(0x99);
    }

    pub fn cqo(&mut self) {
        self.emit_u8(0x48);
        self.emit_u8(0x99);
    }

    pub fn idiv_r(&mut self, is64: bool, divisor: Gp) {
        self.rex(is64, 0, 0, divisor.hi(), false);
        self.emit_u8(0xf7);
        self.modrm_reg(7, divisor.lo());
    }

    pub fn div_r(&mut self, is64: bool, divisor: Gp) {
        self.rex(is64, 0, 0, divisor.hi(), false);
        self.emit_u8(0xf7);
        self.modrm_reg(6, divisor.lo());
    }

    fn shift_cl(&mut self, ext: u8, is64: bool, dst: Gp) {
        self.rex(is64, 0, 0, dst.hi(), false);
        self.emit_u8(0xd3);
        self.modrm_reg(ext, dst.lo());
    }

    fn shift_imm(&mut self, ext: u8, is64: bool, dst: Gp, imm: u8) {
        self.rex(is64, 0, 0, dst.hi(), false);
        if imm == 1 {
            self.emit_u8(0xd1);
            self.modrm_reg(ext, dst.lo());
        } else {
            self.emit_u8(0xc1);
            self.modrm_reg(ext, dst.lo());
            self.emit_u8(imm);
        }
    }

    pub fn rol_cl(&mut self, is64: bool, dst: Gp)  { self.shift_cl(0, is64, dst); }
    pub fn ror_cl(&mut self, is64: bool, dst: Gp)  { self.shift_cl(1, is64, dst); }
    pub fn shl_cl(&mut self, is64: bool, dst: Gp)  { self.shift_cl(4, is64, dst); }
    pub fn shr_cl(&mut self, is64: bool, dst: Gp)  { self.shift_cl(5, is64, dst); }
    pub fn sar_cl(&mut self, is64: bool, dst: Gp)  { self.shift_cl(7, is64, dst); }

    pub fn rol_imm(&mut self, is64: bool, dst: Gp, n: u8) { self.shift_imm(0, is64, dst, n); }
    pub fn ror_imm(&mut self, is64: bool, dst: Gp, n: u8) { self.shift_imm(1, is64, dst, n); }
    pub fn shl_imm(&mut self, is64: bool, dst: Gp, n: u8) { self.shift_imm(4, is64, dst, n); }
    pub fn shr_imm(&mut self, is64: bool, dst: Gp, n: u8) { self.shift_imm(5, is64, dst, n); }
    pub fn sar_imm(&mut self, is64: bool, dst: Gp, n: u8) { self.shift_imm(7, is64, dst, n); }

    pub fn lzcnt(&mut self, is64: bool, dst: Gp, src: Gp) {
        self.emit_u8(0xf3);
        self.rex(is64, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0xbd);
        self.modrm_reg(dst.lo(), src.lo());
    }

    pub fn tzcnt(&mut self, is64: bool, dst: Gp, src: Gp) {
        self.emit_u8(0xf3);
        self.rex(is64, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0xbc);
        self.modrm_reg(dst.lo(), src.lo());
    }

    pub fn popcnt(&mut self, is64: bool, dst: Gp, src: Gp) {
        self.emit_u8(0xf3);
        self.rex(is64, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0xb8);
        self.modrm_reg(dst.lo(), src.lo());
    }

    pub fn setcc(&mut self, cond: Cond, dst: Gp) {
        self.rex(false, 0, 0, dst.hi(), dst.id() >= 4);
        self.emit_u8(0x0f);
        self.emit_u8(0x90 + cond as u8);
        self.modrm_reg(0, dst.lo());
    }

    pub fn cmovcc(&mut self, cond: Cond, is64: bool, dst: Gp, src: Gp) {
        self.rex(is64, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x40 + cond as u8);
        self.modrm_reg(dst.lo(), src.lo());
    }

    // ---- SSE2 / SSE4.1 FP & V128 ----

    fn sse_rr(&mut self, prefix: Option<u8>, op: u8, dst: Xmm, src: Xmm) {
        if let Some(p) = prefix {
            self.emit_u8(p);
        }
        self.rex(false, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(op);
        self.modrm_reg(dst.lo(), src.lo());
    }

    pub fn movaps_rr(&mut self, dst: Xmm, src: Xmm) {
        if dst != src {
            self.sse_rr(None, 0x28, dst, src);
        }
    }

    pub fn movss_xm(&mut self, dst: Xmm, mem: Mem) {
        self.emit_u8(0xf3);
        self.rex_mem(false, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x10);
        self.modrm_mem(dst.lo(), mem);
    }

    pub fn movss_mx(&mut self, mem: Mem, src: Xmm) {
        self.emit_u8(0xf3);
        self.rex_mem(false, src.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x11);
        self.modrm_mem(src.lo(), mem);
    }

    pub fn movsd_xm(&mut self, dst: Xmm, mem: Mem) {
        self.emit_u8(0xf2);
        self.rex_mem(false, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x10);
        self.modrm_mem(dst.lo(), mem);
    }

    pub fn movsd_mx(&mut self, mem: Mem, src: Xmm) {
        self.emit_u8(0xf2);
        self.rex_mem(false, src.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x11);
        self.modrm_mem(src.lo(), mem);
    }

    pub fn movdqu_xm(&mut self, dst: Xmm, mem: Mem) {
        self.emit_u8(0xf3);
        self.rex_mem(false, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x6f);
        self.modrm_mem(dst.lo(), mem);
    }

    pub fn movdqu_mx(&mut self, mem: Mem, src: Xmm) {
        self.emit_u8(0xf3);
        self.rex_mem(false, src.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x7f);
        self.modrm_mem(src.lo(), mem);
    }

    pub fn movd_xr(&mut self, dst: Xmm, src: Gp) {
        self.emit_u8(0x66);
        self.rex(false, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x6e);
        self.modrm_reg(dst.lo(), src.lo());
    }

    pub fn movq_xr(&mut self, dst: Xmm, src: Gp) {
        self.emit_u8(0x66);
        self.rex(true, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x6e);
        self.modrm_reg(dst.lo(), src.lo());
    }

    pub fn movd_rx(&mut self, dst: Gp, src: Xmm) {
        self.emit_u8(0x66);
        self.rex(false, src.hi(), 0, dst.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x7e);
        self.modrm_reg(src.lo(), dst.lo());
    }

    pub fn movq_rx(&mut self, dst: Gp, src: Xmm) {
        self.emit_u8(0x66);
        self.rex(true, src.hi(), 0, dst.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x7e);
        self.modrm_reg(src.lo(), dst.lo());
    }

    pub fn addss(&mut self, d: Xmm, s: Xmm)   { self.sse_rr(Some(0xf3), 0x58, d, s); }
    pub fn subss(&mut self, d: Xmm, s: Xmm)   { self.sse_rr(Some(0xf3), 0x5c, d, s); }
    pub fn mulss(&mut self, d: Xmm, s: Xmm)   { self.sse_rr(Some(0xf3), 0x59, d, s); }
    pub fn divss(&mut self, d: Xmm, s: Xmm)   { self.sse_rr(Some(0xf3), 0x5e, d, s); }
    pub fn sqrtss(&mut self, d: Xmm, s: Xmm)  { self.sse_rr(Some(0xf3), 0x51, d, s); }
    pub fn minss(&mut self, d: Xmm, s: Xmm)   { self.sse_rr(Some(0xf3), 0x5d, d, s); }
    pub fn maxss(&mut self, d: Xmm, s: Xmm)   { self.sse_rr(Some(0xf3), 0x5f, d, s); }
    pub fn ucomiss(&mut self, a: Xmm, b: Xmm) { self.sse_rr(None, 0x2e, a, b); }

    pub fn addsd(&mut self, d: Xmm, s: Xmm)   { self.sse_rr(Some(0xf2), 0x58, d, s); }
    pub fn subsd(&mut self, d: Xmm, s: Xmm)   { self.sse_rr(Some(0xf2), 0x5c, d, s); }
    pub fn mulsd(&mut self, d: Xmm, s: Xmm)   { self.sse_rr(Some(0xf2), 0x59, d, s); }
    pub fn divsd(&mut self, d: Xmm, s: Xmm)   { self.sse_rr(Some(0xf2), 0x5e, d, s); }
    pub fn sqrtsd(&mut self, d: Xmm, s: Xmm)  { self.sse_rr(Some(0xf2), 0x51, d, s); }
    pub fn minsd(&mut self, d: Xmm, s: Xmm)   { self.sse_rr(Some(0xf2), 0x5d, d, s); }
    pub fn maxsd(&mut self, d: Xmm, s: Xmm)   { self.sse_rr(Some(0xf2), 0x5f, d, s); }
    pub fn ucomisd(&mut self, a: Xmm, b: Xmm) { self.sse_rr(Some(0x66), 0x2e, a, b); }

    pub fn andps(&mut self, d: Xmm, s: Xmm)   { self.sse_rr(None, 0x54, d, s); }
    pub fn andnps(&mut self, d: Xmm, s: Xmm)  { self.sse_rr(None, 0x55, d, s); }
    pub fn orps(&mut self, d: Xmm, s: Xmm)    { self.sse_rr(None, 0x56, d, s); }
    pub fn xorps(&mut self, d: Xmm, s: Xmm)   { self.sse_rr(None, 0x57, d, s); }

    pub fn cvtss2sd(&mut self, d: Xmm, s: Xmm) { self.sse_rr(Some(0xf3), 0x5a, d, s); }
    pub fn cvtsd2ss(&mut self, d: Xmm, s: Xmm) { self.sse_rr(Some(0xf2), 0x5a, d, s); }

    pub fn cvtsi2ss(&mut self, is64: bool, d: Xmm, s: Gp) {
        self.emit_u8(0xf3);
        self.rex(is64, d.hi(), 0, s.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x2a);
        self.modrm_reg(d.lo(), s.lo());
    }

    pub fn cvtsi2sd(&mut self, is64: bool, d: Xmm, s: Gp) {
        self.emit_u8(0xf2);
        self.rex(is64, d.hi(), 0, s.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x2a);
        self.modrm_reg(d.lo(), s.lo());
    }

    pub fn cvttss2si(&mut self, is64: bool, d: Gp, s: Xmm) {
        self.emit_u8(0xf3);
        self.rex(is64, d.hi(), 0, s.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x2c);
        self.modrm_reg(d.lo(), s.lo());
    }

    pub fn cvttsd2si(&mut self, is64: bool, d: Gp, s: Xmm) {
        self.emit_u8(0xf2);
        self.rex(is64, d.hi(), 0, s.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x2c);
        self.modrm_reg(d.lo(), s.lo());
    }

    /// SSE4.1 `roundss dst, src, imm8` (`0x08`=nearest, `0x09`=floor, `0x0a`=ceil, `0x0b`=trunc).
    pub fn roundss(&mut self, d: Xmm, s: Xmm, mode: u8) {
        self.emit_u8(0x66);
        self.rex(false, d.hi(), 0, s.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x0a);
        self.modrm_reg(d.lo(), s.lo());
        self.emit_u8(mode);
    }

    /// SSE4.1 `roundsd dst, src, imm8` (`0x08`=nearest, `0x09`=floor, `0x0a`=ceil, `0x0b`=trunc).
    pub fn roundsd(&mut self, d: Xmm, s: Xmm, mode: u8) {
        self.emit_u8(0x66);
        self.rex(false, d.hi(), 0, s.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x0b);
        self.modrm_reg(d.lo(), s.lo());
        self.emit_u8(mode);
    }

    // Minimal SIMD helpers for the V128 subset supported by `wtime-aot`.
    pub fn punpcklbw(&mut self, d: Xmm, s: Xmm) { self.sse_rr(Some(0x66), 0x60, d, s); }
    pub fn punpcklwd(&mut self, d: Xmm, s: Xmm) { self.sse_rr(Some(0x66), 0x61, d, s); }
    pub fn punpckldq(&mut self, d: Xmm, s: Xmm) { self.sse_rr(Some(0x66), 0x62, d, s); }
    pub fn punpcklqdq(&mut self, d: Xmm, s: Xmm) { self.sse_rr(Some(0x66), 0x6c, d, s); }

    pub fn pextrq(&mut self, dst: Gp, src: Xmm, lane: u8) {
        self.emit_u8(0x66);
        self.rex(true, src.hi(), 0, dst.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x16);
        self.modrm_reg(src.lo(), dst.lo());
        self.emit_u8(lane & 1);
    }

    pub fn pinsrq(&mut self, dst: Xmm, src: Gp, lane: u8) {
        self.emit_u8(0x66);
        self.rex(true, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x22);
        self.modrm_reg(dst.lo(), src.lo());
        self.emit_u8(lane & 1);
    }
}
