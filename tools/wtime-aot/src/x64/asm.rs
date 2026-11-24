//! Pure-Rust x86-64 machine-code assembler (`#![forbid(unsafe_code)]`).
//!
//! Emits position-independent x86-64 + SSE2 + SSE4.1 + POPCNT + BMI1/ABM
//! machine code with local labels, forward/backward branch fixups, and
//! cross-section PC-relative relocations (`Func`, `Rodata`, `Helper`).

#![allow(dead_code)]

use std::collections::HashMap;

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
    BaseDisp {
        base: Gp,
        disp: i32,
    },
    BaseIndexDisp {
        base: Gp,
        index: Gp,
        scale: u8,
        disp: i32,
    },
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
        assert!(matches!(scale, 1 | 2 | 4 | 8), "invalid x86-64 SIB scale");
        assert!(
            !matches!(index, Gp::Rsp),
            "x86-64 SIB cannot use RSP as index register"
        );
        Mem::BaseIndexDisp {
            base,
            index,
            scale,
            disp,
        }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FixupKind {
    Jmp,
    Jcc(u8),
    LeaRip,
}

#[derive(Clone, Copy, Debug)]
struct LocalFixup {
    inst_off: u32,
    patch_off: u32,
    next_ip_off: u32,
    label: Label,
    kind: FixupKind,
}

pub struct Asm {
    pub code: Vec<u8>,
    /// Pre-relaxation byte length of `code` charged to `CompileBudget` during emission
    /// (recorded by `finish_labels()` so warm incremental reuse charges identical code caps).
    pub pre_relax_code_len: u32,
    pub relocs: Vec<Reloc>,
    pub local_rodata: Vec<u8>,
    pub jt_patches: Vec<JumpTablePatch>,
    labels: Vec<Option<u32>>,
    align16_labels: Vec<bool>,
    fixups: Vec<LocalFixup>,
    f32_pool: HashMap<u32, u32>,
    f64_pool: HashMap<u64, u32>,
}

impl Asm {
    pub fn new() -> Self {
        Self {
            code: Vec::with_capacity(256),
            pre_relax_code_len: 0,
            relocs: Vec::new(),
            local_rodata: Vec::new(),
            jt_patches: Vec::new(),
            labels: Vec::new(),
            align16_labels: Vec::new(),
            fixups: Vec::new(),
            f32_pool: HashMap::new(),
            f64_pool: HashMap::new(),
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            code: Vec::with_capacity(cap),
            pre_relax_code_len: 0,
            relocs: Vec::new(),
            local_rodata: Vec::new(),
            jt_patches: Vec::new(),
            labels: Vec::new(),
            align16_labels: Vec::new(),
            fixups: Vec::new(),
            f32_pool: HashMap::new(),
            f64_pool: HashMap::new(),
        }
    }

    #[inline]
    pub fn intern_f32(&mut self, bits: u32) -> u32 {
        if let Some(&off) = self.f32_pool.get(&bits) {
            return off;
        }
        let off = self.add_local_rodata(4, &bits.to_le_bytes());
        self.f32_pool.insert(bits, off);
        off
    }

    #[inline]
    pub fn intern_f64(&mut self, bits: u64) -> u32 {
        if let Some(&off) = self.f64_pool.get(&bits) {
            return off;
        }
        let off = self.add_local_rodata(8, &bits.to_le_bytes());
        self.f64_pool.insert(bits, off);
        off
    }

    #[inline]
    pub fn pos(&self) -> u32 {
        u32::try_from(self.code.len()).expect("function machine code exceeded u32::MAX")
    }

    #[inline]
    pub fn new_label(&mut self) -> Label {
        let id = u32::try_from(self.labels.len()).expect("label count exceeded u32::MAX");
        self.labels.push(None);
        self.align16_labels.push(false);
        Label(id)
    }

    #[inline]
    pub fn mark_align16(&mut self, label: Label) {
        if let Some(slot) = self.align16_labels.get_mut(label.0 as usize) {
            *slot = true;
        }
    }

    #[inline]
    pub fn bind(&mut self, label: Label) {
        debug_assert!(self.labels[label.0 as usize].is_none(), "label bound twice");
        self.labels[label.0 as usize] = Some(self.pos());
    }

    /// Appends `n` bytes of canonical x86-64 multi-byte NOP padding (1..=15 bytes).
    fn emit_nops(buf: &mut Vec<u8>, mut n: usize) {
        const NOPS: [&[u8]; 9] = [
            &[0x90],
            &[0x66, 0x90],
            &[0x0f, 0x1f, 0x00],
            &[0x0f, 0x1f, 0x40, 0x00],
            &[0x0f, 0x1f, 0x44, 0x00, 0x00],
            &[0x66, 0x0f, 0x1f, 0x44, 0x00, 0x00],
            &[0x0f, 0x1f, 0x80, 0x00, 0x00, 0x00, 0x00],
            &[0x0f, 0x1f, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00],
            &[0x66, 0x0f, 0x1f, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00],
        ];
        while n > 0 {
            let chunk = n.min(9);
            buf.extend_from_slice(NOPS[chunk - 1]);
            n -= chunk;
        }
    }

    /// Resolves all local branch fixups, relaxing branches whose target lies within `rel8`
    /// (`[-128, 127]`) from 5/6-byte `rel32` encodings to 2-byte `rel8` encodings while
    /// keeping `labels` and `relocs` exact, then pads `align16`-marked labels (e.g. loop
    /// headers) to 16-byte boundaries using canonical multi-byte NOPs whenever all crossing
    /// `rel8` branches remain safely within `i8` range. Must be called before linking.
    pub fn finish_labels(&mut self) -> Result<(), String> {
        if self.code.len() > i32::MAX as usize {
            return Err(format!(
                "function machine code ({} B) exceeds rel32 range",
                self.code.len()
            ));
        }
        self.pre_relax_code_len = self.code.len() as u32;
        let has_align16 = self.align16_labels.iter().any(|&a| a);
        let n_fix = self.fixups.len();
        if n_fix == 0 && !has_align16 {
            return Ok(());
        }
        // Validate every fixup label upfront.
        let mut target_orig = Vec::with_capacity(n_fix);
        for f in &self.fixups {
            let t = self
                .labels
                .get(f.label.0 as usize)
                .and_then(|&o| o)
                .ok_or_else(|| format!("unbound label {}", f.label.0))?;
            target_orig.push(t);
        }

        // Monotonic branch relaxation (up to 4 passes catch cascading shrinks cleanly):
        // - `jmp` directly to the immediately following instruction (`rel == 0`): 5 B -> 0 B (saves 5 B)
        // - `jmp rel32` within `rel8`: 5 B -> 2 B (saves 3 B)
        // - `jcc rel32` within `rel8`: 6 B -> 2 B (saves 4 B)
        // Mode: 0 = rel32, 1 = rel8, 2 = elided 0-byte fallthrough jmp.
        let mut mode = vec![0u8; n_fix];
        let mut saved_prefix = vec![0u32; n_fix + 1];
        let mut any_changed = false;
        let save_for = |kind: FixupKind, m: u8| -> u32 {
            match (kind, m) {
                (FixupKind::Jmp, 2) => 5,
                (FixupKind::Jmp, 1) => 3,
                (FixupKind::Jcc(_), 1) => 4,
                _ => 0,
            }
        };
        for _pass in 0..4 {
            let mut changed = false;
            let shift_of = |orig_off: u32, fixups: &[LocalFixup], prefix: &[u32]| -> u32 {
                let idx = fixups.partition_point(|fx| fx.next_ip_off <= orig_off);
                prefix[idx]
            };
            for (i, f) in self.fixups.iter().enumerate() {
                let cur_m = mode[i];
                let cur_save = save_for(f.kind, cur_m);
                let t_cur = i64::from(target_orig[i])
                    - i64::from(shift_of(target_orig[i], &self.fixups, &saved_prefix));
                // Check if a forward `Jmp` lands directly on the byte immediately after itself
                // (0-byte fallthrough). Backward/self jumps (`target_orig[i] < f.next_ip_off`)
                // MUST never be elided or infinite loops (`L: jmp L`) and empty cycles vanish.
                if matches!(f.kind, FixupKind::Jmp) && cur_m < 2 && target_orig[i] >= f.next_ip_off
                {
                    let add_save = 5 - cur_save;
                    let ip_zero = i64::from(f.inst_off) - i64::from(saved_prefix[i]);
                    if (t_cur - i64::from(add_save)) == ip_zero {
                        mode[i] = 2;
                        any_changed = true;
                        changed = true;
                        continue;
                    }
                }
                if cur_m == 0 && !matches!(f.kind, FixupKind::LeaRip) {
                    let save = save_for(f.kind, 1);
                    let ip_cur = i64::from(f.inst_off) - i64::from(saved_prefix[i]) + 2;
                    let adj = if target_orig[i] >= f.next_ip_off {
                        i64::from(save)
                    } else {
                        0
                    };
                    let rel = (t_cur - adj) - ip_cur;
                    if i8::try_from(rel).is_ok() {
                        mode[i] = 1;
                        any_changed = true;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
            let mut acc = 0u32;
            for (i, f) in self.fixups.iter().enumerate() {
                saved_prefix[i] = acc;
                acc += save_for(f.kind, mode[i]);
            }
            saved_prefix[n_fix] = acc;
        }

        struct PostBranch {
            inst_off: u32,
            patch_off: u32,
            target_off: u32,
            cur_rel: i64,
            is_rel8: bool,
        }
        let mut post_branches: Vec<PostBranch> = if has_align16 {
            Vec::with_capacity(n_fix)
        } else {
            Vec::new()
        };

        if !any_changed {
            for (i, f) in self.fixups.iter().enumerate() {
                let rel = i128::from(target_orig[i]) - i128::from(f.next_ip_off);
                let rel32 = i32::try_from(rel)
                    .map_err(|_| "local branch displacement exceeds rel32".to_string())?;
                let p = f.patch_off as usize;
                let end = p
                    .checked_add(4)
                    .ok_or("local branch patch offset overflow")?;
                if end > self.code.len() {
                    return Err(format!(
                        "local branch patch offset {p} exceeds code len {}",
                        self.code.len()
                    ));
                }
                self.code[p..end].copy_from_slice(&rel32.to_le_bytes());
                if has_align16 {
                    post_branches.push(PostBranch {
                        inst_off: f.inst_off,
                        patch_off: f.patch_off,
                        target_off: target_orig[i],
                        cur_rel: i64::from(rel32),
                        is_rel8: false,
                    });
                }
            }
        } else {
            // Recompute final exact label offsets, reloc offsets, and compact machine code in O(code.len()).
            let shift_of = |orig_off: u32| -> u32 {
                let idx = self.fixups.partition_point(|fx| fx.next_ip_off <= orig_off);
                saved_prefix[idx]
            };
            for off in self.labels.iter_mut().flatten() {
                *off -= shift_of(*off);
            }
            for rel in &mut self.relocs {
                let sh = shift_of(rel.patch_off);
                rel.patch_off -= sh;
                rel.next_ip_off -= sh;
            }

            let total_saved = saved_prefix[n_fix] as usize;
            let mut new_code = Vec::with_capacity(self.code.len() - total_saved);
            let mut cursor = 0usize;
            for (i, f) in self.fixups.iter().enumerate() {
                let inst_start = f.inst_off as usize;
                let orig_end = f.next_ip_off as usize;
                new_code.extend_from_slice(&self.code[cursor..inst_start]);
                let target_new = self.labels[f.label.0 as usize].unwrap();
                match mode[i] {
                    2 => {
                        debug_assert_eq!(target_new, new_code.len() as u32);
                    }
                    1 => {
                        let new_inst_off = new_code.len() as u32;
                        let new_next_ip = new_inst_off + 2;
                        let rel = i64::from(target_new) - i64::from(new_next_ip);
                        let rel8 = i8::try_from(rel)
                            .map_err(|_| "short branch displacement overflow".to_string())?;
                        match f.kind {
                            FixupKind::Jmp => {
                                new_code.push(0xeb);
                                new_code.push(rel8 as u8);
                            }
                            FixupKind::Jcc(c) => {
                                new_code.push(0x70 + c);
                                new_code.push(rel8 as u8);
                            }
                            FixupKind::LeaRip => unreachable!(),
                        }
                        if has_align16 {
                            post_branches.push(PostBranch {
                                inst_off: new_inst_off,
                                patch_off: new_inst_off + 1,
                                target_off: target_new,
                                cur_rel: i64::from(rel8),
                                is_rel8: true,
                            });
                        }
                    }
                    _ => {
                        let new_inst_off = new_code.len() as u32;
                        let orig_len = (orig_end - inst_start) as u32;
                        let new_next_ip = new_inst_off + orig_len;
                        let rel = i128::from(target_new) - i128::from(new_next_ip);
                        let rel32 = i32::try_from(rel)
                            .map_err(|_| "local branch displacement exceeds rel32".to_string())?;
                        let patch_off = match f.kind {
                            FixupKind::Jmp => {
                                new_code.push(0xe9);
                                new_code.extend_from_slice(&rel32.to_le_bytes());
                                new_inst_off + 1
                            }
                            FixupKind::Jcc(c) => {
                                new_code.push(0x0f);
                                new_code.push(0x80 + c);
                                new_code.extend_from_slice(&rel32.to_le_bytes());
                                new_inst_off + 2
                            }
                            FixupKind::LeaRip => {
                                let p_rel = (f.patch_off as usize) - inst_start;
                                new_code
                                    .extend_from_slice(&self.code[inst_start..inst_start + p_rel]);
                                new_code.extend_from_slice(&rel32.to_le_bytes());
                                new_inst_off + (p_rel as u32)
                            }
                        };
                        if has_align16 {
                            post_branches.push(PostBranch {
                                inst_off: new_inst_off,
                                patch_off,
                                target_off: target_new,
                                cur_rel: i64::from(rel32),
                                is_rel8: false,
                            });
                        }
                    }
                }
                cursor = orig_end;
            }
            new_code.extend_from_slice(&self.code[cursor..]);
            debug_assert_eq!(new_code.len(), self.code.len() - total_saved);
            self.code = new_code;
        }
        self.fixups.clear();

        // Optional 16-byte loop header alignment via multi-byte NOPs (skipped cleanly on any
        // label whose padding would push a crossing `rel8` branch out of `[-128, 127]` or
        // exceed `MAX_FN_CODE_BYTES`).
        if has_align16 && !self.code.is_empty() {
            let code_len = self.code.len() as u32;
            let mut align_offs: Vec<u32> = self
                .labels
                .iter()
                .zip(&self.align16_labels)
                .filter_map(|(&off, &want)| want.then_some(off).flatten())
                .filter(|&off| off > 0 && off < code_len)
                .collect();
            align_offs.sort_unstable();
            align_offs.dedup();

            let mut pads: Vec<(u32, u32)> = Vec::new();
            let mut total_pad: u32 = 0;
            for orig_off in align_offs {
                let cur_off = orig_off.wrapping_add(total_pad);
                let pad = (16 - (cur_off & 15)) & 15;
                if pad == 0 {
                    continue;
                }
                if self
                    .code
                    .len()
                    .saturating_add(total_pad as usize + pad as usize)
                    > super::module::MAX_FN_CODE_BYTES
                {
                    continue;
                }
                let mut safe = true;
                for br in &post_branches {
                    let delta = if br.inst_off < orig_off && br.target_off >= orig_off {
                        i64::from(pad)
                    } else if br.inst_off >= orig_off && br.target_off < orig_off {
                        -i64::from(pad)
                    } else {
                        0
                    };
                    if delta != 0 {
                        let next_rel = br.cur_rel + delta;
                        let fits = if br.is_rel8 {
                            i8::try_from(next_rel).is_ok()
                        } else {
                            i32::try_from(next_rel).is_ok()
                        };
                        if !fits {
                            safe = false;
                            break;
                        }
                    }
                }
                if !safe {
                    continue;
                }
                for br in &mut post_branches {
                    if br.inst_off < orig_off && br.target_off >= orig_off {
                        br.cur_rel += i64::from(pad);
                    } else if br.inst_off >= orig_off && br.target_off < orig_off {
                        br.cur_rel -= i64::from(pad);
                    }
                }
                pads.push((orig_off, pad));
                total_pad += pad;
            }

            if !pads.is_empty() {
                let mut pad_prefix = Vec::with_capacity(pads.len() + 1);
                let mut acc = 0u32;
                pad_prefix.push(0u32);
                for &(_, p) in &pads {
                    acc += p;
                    pad_prefix.push(acc);
                }
                let pad_shift = |orig_off: u32| -> u32 {
                    let idx = pads.partition_point(|&(poff, _)| poff <= orig_off);
                    pad_prefix[idx]
                };

                let mut padded = Vec::with_capacity(self.code.len() + total_pad as usize);
                let mut cursor = 0usize;
                for &(poff, pad) in &pads {
                    let po = poff as usize;
                    padded.extend_from_slice(&self.code[cursor..po]);
                    Self::emit_nops(&mut padded, pad as usize);
                    cursor = po;
                }
                padded.extend_from_slice(&self.code[cursor..]);
                debug_assert_eq!(padded.len(), self.code.len() + total_pad as usize);

                // Patch all branch displacements at their shifted offsets.
                for br in &post_branches {
                    let p = (br.patch_off + pad_shift(br.inst_off)) as usize;
                    if br.is_rel8 {
                        padded[p] = (br.cur_rel as i8) as u8;
                    } else {
                        padded[p..p + 4].copy_from_slice(&(br.cur_rel as i32).to_le_bytes());
                    }
                }
                // Shift all bound labels (`>= poff`) and relocations (`patch_off >= poff`).
                for off in self.labels.iter_mut().flatten() {
                    *off += pad_shift(*off);
                }
                for rel in &mut self.relocs {
                    let sh = pad_shift(rel.patch_off);
                    rel.patch_off += sh;
                    rel.next_ip_off += sh;
                }
                self.code = padded;
                self.pre_relax_code_len = self.pre_relax_code_len.max(self.code.len() as u32);
            }
        }
        Ok(())
    }

    /// Fallibly extracts resolved `(rodata_off, table_rodata_off, label_code_off)` jump-table
    /// entries after `finish_labels()` for optional incremental function cache snapshotting.
    pub fn try_export_jt_entries(&self) -> Option<Vec<(u32, u32, u32)>> {
        let mut out = Vec::new();
        out.try_reserve_exact(self.jt_patches.len()).ok()?;
        for jt in &self.jt_patches {
            let off = self.labels.get(jt.label.0 as usize).and_then(|&o| o)?;
            out.push((jt.rodata_off, jt.table_rodata_off, off));
        }
        Some(out)
    }

    /// Reconstructs a link-ready `Asm` from cached buffers and pre-built jump-table metadata
    /// with zero heap allocations.
    pub fn from_cached_parts(
        code: Vec<u8>,
        pre_relax_code_len: u32,
        relocs: Vec<Reloc>,
        local_rodata: Vec<u8>,
        labels: Vec<Option<u32>>,
        jt_patches: Vec<JumpTablePatch>,
    ) -> Self {
        Self {
            pre_relax_code_len: pre_relax_code_len.max(code.len() as u32),
            code,
            relocs,
            local_rodata,
            jt_patches,
            labels,
            align16_labels: Vec::new(),
            fixups: Vec::new(),
            f32_pool: HashMap::new(),
            f64_pool: HashMap::new(),
        }
    }

    /// Fallibly builds `(labels, jt_patches)` from cached `(rodata_off, table_rodata_off, label_off)` tuples.
    pub fn try_build_cached_jt(
        jt_entries: &[(u32, u32, u32)],
    ) -> Option<(Vec<Option<u32>>, Vec<JumpTablePatch>)> {
        let mut labels = Vec::new();
        labels.try_reserve_exact(jt_entries.len()).ok()?;
        let mut jt_patches = Vec::new();
        jt_patches.try_reserve_exact(jt_entries.len()).ok()?;
        for (i, &(rodata_off, table_rodata_off, label_off)) in jt_entries.iter().enumerate() {
            labels.push(Some(label_off));
            jt_patches.push(JumpTablePatch {
                rodata_off,
                table_rodata_off,
                label: Label(i as u32),
            });
        }
        Some((labels, jt_patches))
    }

    /// Resolves jump-table entries in `local_rodata` once `fn_vaddr` and `local_rodata_vaddr` are known.
    pub fn patch_jump_tables(
        &mut self,
        fn_vaddr: u64,
        local_rodata_vaddr: u64,
    ) -> Result<(), String> {
        for jt in &self.jt_patches {
            let label_off = self
                .labels
                .get(jt.label.0 as usize)
                .and_then(|&o| o)
                .ok_or_else(|| format!("unbound jt label {}", jt.label.0))?;
            let target_vaddr = fn_vaddr
                .checked_add(u64::from(label_off))
                .ok_or("jump table target vaddr overflow")?;
            let base_vaddr = local_rodata_vaddr
                .checked_add(u64::from(jt.table_rodata_off))
                .ok_or("jump table base vaddr overflow")?;
            let rel = i128::from(target_vaddr) - i128::from(base_vaddr);
            let rel32 = i32::try_from(rel)
                .map_err(|_| "jump table displacement exceeds rel32".to_string())?;
            let p = jt.rodata_off as usize;
            let end = p.checked_add(4).ok_or("jump table patch offset overflow")?;
            if end > self.local_rodata.len() {
                return Err(format!(
                    "jump table patch offset {p} exceeds local_rodata len {}",
                    self.local_rodata.len()
                ));
            }
            self.local_rodata[p..end].copy_from_slice(&rel32.to_le_bytes());
        }
        Ok(())
    }

    /// Patches cached `(rodata_off, table_rodata_off, label_off)` jump-table entries directly into
    /// a destination `.rodata` slice (zero-copy linking from `Arc<IncCacheData>`).
    pub fn patch_cached_jump_tables_in_slice(
        dst_local_rodata: &mut [u8],
        jt_entries: &[(u32, u32, u32)],
        fn_vaddr: u64,
        local_rodata_vaddr: u64,
    ) -> Result<(), String> {
        for &(rodata_off, table_rodata_off, label_off) in jt_entries {
            let target_vaddr = fn_vaddr
                .checked_add(u64::from(label_off))
                .ok_or("jump table target vaddr overflow")?;
            let base_vaddr = local_rodata_vaddr
                .checked_add(u64::from(table_rodata_off))
                .ok_or("jump table base vaddr overflow")?;
            let rel = i128::from(target_vaddr) - i128::from(base_vaddr);
            let rel32 = i32::try_from(rel)
                .map_err(|_| "jump table displacement exceeds rel32".to_string())?;
            let p = rodata_off as usize;
            let end = p.checked_add(4).ok_or("jump table patch offset overflow")?;
            if end > dst_local_rodata.len() {
                return Err(format!(
                    "jump table patch offset {p} exceeds local_rodata len {}",
                    dst_local_rodata.len()
                ));
            }
            dst_local_rodata[p..end].copy_from_slice(&rel32.to_le_bytes());
        }
        Ok(())
    }

    /// Appends aligned raw bytes to `local_rodata`, returning the starting offset.
    pub fn add_local_rodata(&mut self, align: usize, bytes: &[u8]) -> u32 {
        assert!(
            align.is_power_of_two(),
            "local_rodata alignment must be a non-zero power of two"
        );
        let mask = align - 1;
        while (self.local_rodata.len() & mask) != 0 {
            self.local_rodata.push(0);
        }
        let off = u32::try_from(self.local_rodata.len()).expect("local_rodata exceeded u32::MAX");
        self.local_rodata.extend_from_slice(bytes);
        off
    }

    /// Appends a rel32 jump table for `targets` to `local_rodata`, returning the table base offset.
    pub fn add_jump_table(&mut self, targets: &[Label]) -> u32 {
        while (self.local_rodata.len() & 3) != 0 {
            self.local_rodata.push(0);
        }
        let base = u32::try_from(self.local_rodata.len()).expect("local_rodata exceeded u32::MAX");
        for &lbl in targets {
            let off =
                u32::try_from(self.local_rodata.len()).expect("local_rodata exceeded u32::MAX");
            self.local_rodata.extend_from_slice(&0i32.to_le_bytes());
            self.jt_patches.push(JumpTablePatch {
                rodata_off: off,
                table_rodata_off: base,
                label: lbl,
            });
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
            Mem::BaseIndexDisp { base, index, .. } => {
                self.rex(w, reg_hi, index.hi(), base.hi(), force)
            }
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
            Mem::BaseIndexDisp {
                base,
                index,
                scale,
                disp,
            } => {
                assert!(
                    !matches!(index, Gp::Rsp),
                    "x86-64 SIB cannot use RSP as index register"
                );
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
        let inst_off = self.pos();
        self.emit_u8(0xe9);
        let patch_off = self.pos();
        self.emit_i32(0);
        let next_ip_off = self.pos();
        self.fixups.push(LocalFixup {
            inst_off,
            patch_off,
            next_ip_off,
            label,
            kind: FixupKind::Jmp,
        });
    }

    pub fn jcc(&mut self, cond: Cond, label: Label) {
        let inst_off = self.pos();
        self.emit_u8(0x0f);
        self.emit_u8(0x80 + cond as u8);
        let patch_off = self.pos();
        self.emit_i32(0);
        let next_ip_off = self.pos();
        self.fixups.push(LocalFixup {
            inst_off,
            patch_off,
            next_ip_off,
            label,
            kind: FixupKind::Jcc(cond as u8),
        });
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
        self.relocs.push(Reloc {
            patch_off,
            next_ip_off,
            target,
        });
    }

    pub fn jmp_reloc(&mut self, target: RelocTarget) {
        self.emit_u8(0xe9);
        let patch_off = self.pos();
        self.emit_i32(0);
        let next_ip_off = self.pos();
        self.relocs.push(Reloc {
            patch_off,
            next_ip_off,
            target,
        });
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
        self.relocs.push(Reloc {
            patch_off,
            next_ip_off,
            target,
        });
    }

    /// `lea dst, [rip + label]` resolved by `finish_labels()`.
    pub fn lea_rip_label(&mut self, dst: Gp, label: Label) {
        let inst_off = self.pos();
        self.rex(true, dst.hi(), 0, 0, false);
        self.emit_u8(0x8d);
        self.emit_u8((dst.lo() << 3) | 5);
        let patch_off = self.pos();
        self.emit_i32(0);
        let next_ip_off = self.pos();
        self.fixups.push(LocalFixup {
            inst_off,
            patch_off,
            next_ip_off,
            label,
            kind: FixupKind::LeaRip,
        });
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
        self.relocs.push(Reloc {
            patch_off,
            next_ip_off,
            target,
        });
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
        self.relocs.push(Reloc {
            patch_off,
            next_ip_off,
            target,
        });
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
        self.relocs.push(Reloc {
            patch_off,
            next_ip_off,
            target,
        });
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
        self.relocs.push(Reloc {
            patch_off,
            next_ip_off,
            target,
        });
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

    /// Loads 32-bit zero-extended immediate into `dst` without modifying `RFLAGS` (even for `0`).
    pub fn mov_ri32_preserve_flags(&mut self, dst: Gp, imm: u32) {
        self.rex(false, 0, 0, dst.hi(), false);
        self.emit_u8(0xb8 + dst.lo());
        self.emit_u32(imm);
    }

    /// Loads 64-bit immediate into `dst` without modifying `RFLAGS` (even for `0`).
    pub fn mov_ri64_preserve_flags(&mut self, dst: Gp, imm: u64) {
        if imm <= u32::MAX as u64 {
            self.mov_ri32_preserve_flags(dst, imm as u32);
        } else if (imm as i64) >= i32::MIN as i64 && (imm as i64) <= i32::MAX as i64 {
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

    /// Loads 32-bit zero-extended immediate into `dst`.
    /// Note: `imm == 0` emits `xor dst32, dst32`, which modifies `RFLAGS` — do not
    /// call with `imm == 0` between a `cmp`/`test` and a dependent `jcc`/`cmovcc`/`setcc`.
    pub fn mov_ri32(&mut self, dst: Gp, imm: u32) {
        if imm == 0 {
            self.xor_rr(false, dst, dst);
            return;
        }
        self.mov_ri32_preserve_flags(dst, imm);
    }

    /// Loads 64-bit immediate into `dst`.
    /// Note: `imm == 0` emits `xor dst32, dst32`, which modifies `RFLAGS`.
    pub fn mov_ri64(&mut self, dst: Gp, imm: u64) {
        if imm == 0 {
            self.xor_rr(false, dst, dst);
        } else {
            self.mov_ri64_preserve_flags(dst, imm);
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

    pub fn add_rr(&mut self, is64: bool, dst: Gp, src: Gp) {
        self.alu_rr(0x01, is64, dst, src);
    }
    pub fn or_rr(&mut self, is64: bool, dst: Gp, src: Gp) {
        self.alu_rr(0x09, is64, dst, src);
    }
    pub fn and_rr(&mut self, is64: bool, dst: Gp, src: Gp) {
        self.alu_rr(0x21, is64, dst, src);
    }
    pub fn sub_rr(&mut self, is64: bool, dst: Gp, src: Gp) {
        self.alu_rr(0x29, is64, dst, src);
    }
    pub fn xor_rr(&mut self, is64: bool, dst: Gp, src: Gp) {
        self.alu_rr(0x31, is64, dst, src);
    }
    pub fn cmp_rr(&mut self, is64: bool, a: Gp, b: Gp) {
        self.alu_rr(0x39, is64, a, b);
    }

    pub fn add_ri(&mut self, is64: bool, dst: Gp, imm: i32) {
        self.alu_ri(0, is64, dst, imm);
    }
    pub fn or_ri(&mut self, is64: bool, dst: Gp, imm: i32) {
        self.alu_ri(1, is64, dst, imm);
    }
    pub fn and_ri(&mut self, is64: bool, dst: Gp, imm: i32) {
        self.alu_ri(4, is64, dst, imm);
    }
    pub fn sub_ri(&mut self, is64: bool, dst: Gp, imm: i32) {
        self.alu_ri(5, is64, dst, imm);
    }
    pub fn xor_ri(&mut self, is64: bool, dst: Gp, imm: i32) {
        self.alu_ri(6, is64, dst, imm);
    }
    pub fn cmp_ri(&mut self, is64: bool, a: Gp, imm: i32) {
        self.alu_ri(7, is64, a, imm);
    }

    fn alu_rm(&mut self, op: u8, is64: bool, dst: Gp, mem: Mem) {
        self.rex_mem(is64, dst.hi(), mem, false);
        self.emit_u8(op);
        self.modrm_mem(dst.lo(), mem);
    }

    pub fn add_rm(&mut self, is64: bool, dst: Gp, mem: Mem) {
        self.alu_rm(0x03, is64, dst, mem);
    }
    pub fn or_rm(&mut self, is64: bool, dst: Gp, mem: Mem) {
        self.alu_rm(0x0b, is64, dst, mem);
    }
    pub fn and_rm(&mut self, is64: bool, dst: Gp, mem: Mem) {
        self.alu_rm(0x23, is64, dst, mem);
    }
    pub fn sub_rm(&mut self, is64: bool, dst: Gp, mem: Mem) {
        self.alu_rm(0x2b, is64, dst, mem);
    }
    pub fn xor_rm(&mut self, is64: bool, dst: Gp, mem: Mem) {
        self.alu_rm(0x33, is64, dst, mem);
    }
    pub fn cmp_rm(&mut self, is64: bool, a: Gp, mem: Mem) {
        self.alu_rm(0x3b, is64, a, mem);
    }

    pub fn imul_rm(&mut self, is64: bool, dst: Gp, mem: Mem) {
        self.rex_mem(is64, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0xaf);
        self.modrm_mem(dst.lo(), mem);
    }

    pub fn test_rr(&mut self, is64: bool, a: Gp, b: Gp) {
        self.rex(is64, b.hi(), 0, a.hi(), false);
        self.emit_u8(0x85);
        self.modrm_reg(b.lo(), a.lo());
    }

    /// `test r/m32/64, imm32` (`[REX.W+B] F7 /0 id`).
    pub fn test_ri(&mut self, is64: bool, a: Gp, imm: i32) {
        self.rex(is64, 0, 0, a.hi(), false);
        self.emit_u8(0xf7);
        self.modrm_reg(0, a.lo());
        self.emit_i32(imm);
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

    pub fn mul_r(&mut self, is64: bool, factor: Gp) {
        self.rex(is64, 0, 0, factor.hi(), false);
        self.emit_u8(0xf7);
        self.modrm_reg(4, factor.lo());
    }

    pub fn imul1_r(&mut self, is64: bool, factor: Gp) {
        self.rex(is64, 0, 0, factor.hi(), false);
        self.emit_u8(0xf7);
        self.modrm_reg(5, factor.lo());
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

    pub fn rol_cl(&mut self, is64: bool, dst: Gp) {
        self.shift_cl(0, is64, dst);
    }
    pub fn ror_cl(&mut self, is64: bool, dst: Gp) {
        self.shift_cl(1, is64, dst);
    }
    pub fn shl_cl(&mut self, is64: bool, dst: Gp) {
        self.shift_cl(4, is64, dst);
    }
    pub fn shr_cl(&mut self, is64: bool, dst: Gp) {
        self.shift_cl(5, is64, dst);
    }
    pub fn sar_cl(&mut self, is64: bool, dst: Gp) {
        self.shift_cl(7, is64, dst);
    }

    /// `adc dst, imm8` (`[REX.W+B] 83 /2 ib`).
    pub fn adc_ri8(&mut self, is64: bool, dst: Gp, imm: i8) {
        self.rex(is64, 0, 0, dst.hi(), false);
        self.emit_u8(0x83);
        self.modrm_reg(2, dst.lo());
        self.emit_u8(imm as u8);
    }

    /// `shrd dst, src, cl` (`[REX.W+R+B] 0F AD /r`).
    pub fn shrd_cl(&mut self, is64: bool, dst: Gp, src: Gp) {
        self.rex(is64, src.hi(), 0, dst.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0xad);
        self.modrm_reg(src.lo(), dst.lo());
    }

    pub fn rol_imm(&mut self, is64: bool, dst: Gp, n: u8) {
        self.shift_imm(0, is64, dst, n);
    }
    pub fn ror_imm(&mut self, is64: bool, dst: Gp, n: u8) {
        self.shift_imm(1, is64, dst, n);
    }
    pub fn shl_imm(&mut self, is64: bool, dst: Gp, n: u8) {
        self.shift_imm(4, is64, dst, n);
    }
    pub fn shr_imm(&mut self, is64: bool, dst: Gp, n: u8) {
        self.shift_imm(5, is64, dst, n);
    }
    pub fn sar_imm(&mut self, is64: bool, dst: Gp, n: u8) {
        self.shift_imm(7, is64, dst, n);
    }

    // ---- BMI2 non-flag-modifying 3-operand shifts & rotate ----

    #[allow(clippy::too_many_arguments)]
    fn vex3_rr(&mut self, is64: bool, map: u8, pp: u8, dst: Gp, vvvv_id: u8, rm: Gp, op: u8) {
        let r_inv = (!dst.hi()) & 1;
        let b_inv = (!rm.hi()) & 1;
        self.emit_u8(0xc4);
        self.emit_u8((r_inv << 7) | (1 << 6) | (b_inv << 5) | (map & 0x1f));
        self.emit_u8((u8::from(is64) << 7) | (((!vvvv_id) & 0x0f) << 3) | (pp & 3));
        self.emit_u8(op);
        self.modrm_reg(dst.lo(), rm.lo());
    }

    /// BMI2 `shlx dst, src, cnt` (`VEX.LZ.66.0F38.W0/W1 F7 /r`): `dst = src << (cnt & (w-1))`.
    pub fn shlx(&mut self, is64: bool, dst: Gp, src: Gp, cnt: Gp) {
        self.vex3_rr(is64, 2, 1, dst, cnt.id(), src, 0xf7);
    }

    /// BMI2 `sarx dst, src, cnt` (`VEX.LZ.F3.0F38.W0/W1 F7 /r`): `dst = src >>s (cnt & (w-1))`.
    pub fn sarx(&mut self, is64: bool, dst: Gp, src: Gp, cnt: Gp) {
        self.vex3_rr(is64, 2, 2, dst, cnt.id(), src, 0xf7);
    }

    /// BMI2 `shrx dst, src, cnt` (`VEX.LZ.F2.0F38.W0/W1 F7 /r`): `dst = src >>u (cnt & (w-1))`.
    pub fn shrx(&mut self, is64: bool, dst: Gp, src: Gp, cnt: Gp) {
        self.vex3_rr(is64, 2, 3, dst, cnt.id(), src, 0xf7);
    }

    /// BMI2 `rorx dst, src, imm8` (`VEX.LZ.F2.0F3A.W0/W1 F0 /r ib`): non-destructive rotate right.
    pub fn rorx(&mut self, is64: bool, dst: Gp, src: Gp, imm: u8) {
        self.vex3_rr(is64, 3, 3, dst, 0, src, 0xf0);
        self.emit_u8(imm);
    }

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

    fn sse_xm(&mut self, prefix: Option<u8>, op: u8, dst: Xmm, mem: Mem) {
        if let Some(p) = prefix {
            self.emit_u8(p);
        }
        self.rex_mem(false, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(op);
        self.modrm_mem(dst.lo(), mem);
    }

    pub fn addss_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(Some(0xf3), 0x58, d, m);
    }
    pub fn subss_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(Some(0xf3), 0x5c, d, m);
    }
    pub fn mulss_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(Some(0xf3), 0x59, d, m);
    }
    pub fn divss_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(Some(0xf3), 0x5e, d, m);
    }
    pub fn minss_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(Some(0xf3), 0x5d, d, m);
    }
    pub fn maxss_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(Some(0xf3), 0x5f, d, m);
    }
    pub fn ucomiss_xm(&mut self, a: Xmm, m: Mem) {
        self.sse_xm(None, 0x2e, a, m);
    }

    pub fn addsd_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(Some(0xf2), 0x58, d, m);
    }
    pub fn subsd_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(Some(0xf2), 0x5c, d, m);
    }
    pub fn mulsd_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(Some(0xf2), 0x59, d, m);
    }
    pub fn divsd_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(Some(0xf2), 0x5e, d, m);
    }
    pub fn minsd_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(Some(0xf2), 0x5d, d, m);
    }
    pub fn maxsd_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(Some(0xf2), 0x5f, d, m);
    }
    pub fn ucomisd_xm(&mut self, a: Xmm, m: Mem) {
        self.sse_xm(Some(0x66), 0x2e, a, m);
    }

    pub fn addss(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0xf3), 0x58, d, s);
    }
    pub fn subss(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0xf3), 0x5c, d, s);
    }
    pub fn mulss(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0xf3), 0x59, d, s);
    }
    pub fn divss(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0xf3), 0x5e, d, s);
    }
    pub fn sqrtss(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0xf3), 0x51, d, s);
    }
    pub fn minss(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0xf3), 0x5d, d, s);
    }
    pub fn maxss(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0xf3), 0x5f, d, s);
    }
    pub fn ucomiss(&mut self, a: Xmm, b: Xmm) {
        self.sse_rr(None, 0x2e, a, b);
    }

    pub fn addsd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0xf2), 0x58, d, s);
    }
    pub fn subsd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0xf2), 0x5c, d, s);
    }
    pub fn mulsd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0xf2), 0x59, d, s);
    }
    pub fn divsd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0xf2), 0x5e, d, s);
    }
    pub fn sqrtsd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0xf2), 0x51, d, s);
    }
    pub fn minsd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0xf2), 0x5d, d, s);
    }
    pub fn maxsd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0xf2), 0x5f, d, s);
    }
    pub fn ucomisd(&mut self, a: Xmm, b: Xmm) {
        self.sse_rr(Some(0x66), 0x2e, a, b);
    }

    pub fn andps(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(None, 0x54, d, s);
    }
    pub fn andnps(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(None, 0x55, d, s);
    }
    pub fn orps(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(None, 0x56, d, s);
    }
    pub fn xorps(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(None, 0x57, d, s);
    }

    pub fn cvtss2sd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0xf3), 0x5a, d, s);
    }
    pub fn cvtsd2ss(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0xf2), 0x5a, d, s);
    }

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

    // ---- Complete SSE2 / SSSE3 / SSE4.1 / SSE4.2 SIMD encoders ----

    fn sse_38_rr(&mut self, prefix: Option<u8>, op: u8, dst: Xmm, src: Xmm) {
        if let Some(p) = prefix {
            self.emit_u8(p);
        }
        self.rex(false, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x38);
        self.emit_u8(op);
        self.modrm_reg(dst.lo(), src.lo());
    }

    fn sse_38_xm(&mut self, prefix: Option<u8>, op: u8, dst: Xmm, mem: Mem) {
        if let Some(p) = prefix {
            self.emit_u8(p);
        }
        self.rex_mem(false, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x38);
        self.emit_u8(op);
        self.modrm_mem(dst.lo(), mem);
    }

    fn sse_rri(&mut self, prefix: Option<u8>, op: u8, dst: Xmm, src: Xmm, imm: u8) {
        self.sse_rr(prefix, op, dst, src);
        self.emit_u8(imm);
    }

    fn sse_3a_rri(&mut self, prefix: Option<u8>, op: u8, dst: Xmm, src: Xmm, imm: u8) {
        if let Some(p) = prefix {
            self.emit_u8(p);
        }
        self.rex(false, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(op);
        self.modrm_reg(dst.lo(), src.lo());
        self.emit_u8(imm);
    }

    fn sse_shift_imm(&mut self, op: u8, ext: u8, dst: Xmm, imm: u8) {
        self.emit_u8(0x66);
        self.rex(false, 0, 0, dst.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(op);
        self.modrm_reg(ext, dst.lo());
        self.emit_u8(imm);
    }

    pub fn punpcklbw(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x60, d, s);
    }
    pub fn punpcklwd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x61, d, s);
    }
    pub fn punpckldq(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x62, d, s);
    }
    pub fn punpcklqdq(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x6c, d, s);
    }
    pub fn punpckhqdq(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x6d, d, s);
    }
    pub fn unpcklps(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(None, 0x14, d, s);
    }
    pub fn unpckhps(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(None, 0x15, d, s);
    }
    pub fn unpcklpd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x14, d, s);
    }
    pub fn unpckhpd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x15, d, s);
    }
    pub fn movddup_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(Some(0xf2), 0x12, d, m);
    }
    pub fn movhlps(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(None, 0x12, d, s);
    }
    pub fn movlhps(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(None, 0x16, d, s);
    }
    pub fn shufps(&mut self, d: Xmm, s: Xmm, imm: u8) {
        self.sse_rri(None, 0xc6, d, s, imm);
    }
    pub fn shufpd(&mut self, d: Xmm, s: Xmm, imm: u8) {
        self.sse_rri(Some(0x66), 0xc6, d, s, imm);
    }
    pub fn pshufd(&mut self, d: Xmm, s: Xmm, imm: u8) {
        self.sse_rri(Some(0x66), 0x70, d, s, imm);
    }
    pub fn pshuflw(&mut self, d: Xmm, s: Xmm, imm: u8) {
        self.sse_rri(Some(0xf2), 0x70, d, s, imm);
    }
    pub fn pshufhw(&mut self, d: Xmm, s: Xmm, imm: u8) {
        self.sse_rri(Some(0xf3), 0x70, d, s, imm);
    }
    pub fn pshufb(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x00, d, s);
    }
    pub fn pshufb_rip(&mut self, dst: Xmm, target: RelocTarget) {
        self.emit_u8(0x66);
        self.rex(false, dst.hi(), 0, 0, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x38);
        self.emit_u8(0x00);
        self.emit_u8((dst.lo() << 3) | 5);
        let patch_off = self.pos();
        self.emit_i32(0);
        let next_ip_off = self.pos();
        self.relocs.push(Reloc {
            patch_off,
            next_ip_off,
            target,
        });
    }

    // Bitwise integer SIMD
    pub fn pand(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xdb, d, s);
    }
    pub fn pandn(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xdf, d, s);
    }
    pub fn por(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xeb, d, s);
    }
    pub fn pxor(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xef, d, s);
    }
    pub fn andpd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x54, d, s);
    }
    pub fn andnpd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x55, d, s);
    }
    pub fn orpd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x56, d, s);
    }
    pub fn xorpd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x57, d, s);
    }
    pub fn ptest(&mut self, a: Xmm, b: Xmm) {
        self.sse_38_rr(Some(0x66), 0x17, a, b);
    }
    pub fn pmovmskb(&mut self, dst: Gp, src: Xmm) {
        self.emit_u8(0x66);
        self.rex(false, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0xd7);
        self.modrm_reg(dst.lo(), src.lo());
    }
    pub fn movmskps(&mut self, dst: Gp, src: Xmm) {
        self.rex(false, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x50);
        self.modrm_reg(dst.lo(), src.lo());
    }
    pub fn movmskpd(&mut self, dst: Gp, src: Xmm) {
        self.emit_u8(0x66);
        self.rex(false, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x50);
        self.modrm_reg(dst.lo(), src.lo());
    }

    // Integer comparisons
    pub fn pcmpeqb(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x74, d, s);
    }
    pub fn pcmpeqw(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x75, d, s);
    }
    pub fn pcmpeqd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x76, d, s);
    }
    pub fn pcmpeqq(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x29, d, s);
    }
    pub fn pcmpgtb(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x64, d, s);
    }
    pub fn pcmpgtw(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x65, d, s);
    }
    pub fn pcmpgtd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x66, d, s);
    }
    pub fn pcmpgtq(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x37, d, s);
    }

    // Integer arithmetic / sat / min / max / abs / avg / mul / dot
    pub fn paddb(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xfc, d, s);
    }
    pub fn paddw(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xfd, d, s);
    }
    pub fn paddd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xfe, d, s);
    }
    pub fn paddq(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xd4, d, s);
    }
    pub fn psubb(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xf8, d, s);
    }
    pub fn psubw(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xf9, d, s);
    }
    pub fn psubd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xfa, d, s);
    }
    pub fn psubq(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xfb, d, s);
    }
    pub fn paddsb(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xec, d, s);
    }
    pub fn paddsw(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xed, d, s);
    }
    pub fn paddusb(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xdc, d, s);
    }
    pub fn paddusw(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xdd, d, s);
    }
    pub fn psubsb(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xe8, d, s);
    }
    pub fn psubsw(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xe9, d, s);
    }
    pub fn psubusb(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xd8, d, s);
    }
    pub fn psubusw(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xd9, d, s);
    }
    pub fn pminsb(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x38, d, s);
    }
    pub fn pminub(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xda, d, s);
    }
    pub fn pminsw(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xea, d, s);
    }
    pub fn pminuw(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x3a, d, s);
    }
    pub fn pminsd(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x39, d, s);
    }
    pub fn pminud(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x3b, d, s);
    }
    pub fn pmaxsb(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x3c, d, s);
    }
    pub fn pmaxub(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xde, d, s);
    }
    pub fn pmaxsw(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xee, d, s);
    }
    pub fn pmaxuw(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x3e, d, s);
    }
    pub fn pmaxsd(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x3d, d, s);
    }
    pub fn pmaxud(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x3f, d, s);
    }
    pub fn pabsb(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x1c, d, s);
    }
    pub fn pabsw(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x1d, d, s);
    }
    pub fn pabsd(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x1e, d, s);
    }
    pub fn pavgb(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xe0, d, s);
    }
    pub fn pavgw(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xe3, d, s);
    }
    pub fn pmullw(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xd5, d, s);
    }
    pub fn pmulld(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x40, d, s);
    }
    pub fn pmuldq(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x28, d, s);
    }
    pub fn pmuludq(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xf4, d, s);
    }
    pub fn pmulhrsw(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x0b, d, s);
    }
    pub fn pmaddwd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xf5, d, s);
    }
    pub fn pmaddubsw(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x04, d, s);
    }
    pub fn psadbw(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xf6, d, s);
    }

    // Narrowing / extending
    pub fn packsswb(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x63, d, s);
    }
    pub fn packuswb(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x67, d, s);
    }
    pub fn packssdw(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x6b, d, s);
    }
    pub fn packusdw(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x2b, d, s);
    }
    pub fn pmovsxbw(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x20, d, s);
    }
    pub fn pmovzxbw(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x30, d, s);
    }
    pub fn pmovsxwd(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x23, d, s);
    }
    pub fn pmovzxwd(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x33, d, s);
    }
    pub fn pmovsxdq(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x25, d, s);
    }
    pub fn pmovzxdq(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x35, d, s);
    }
    pub fn pmovsxbw_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_38_xm(Some(0x66), 0x20, d, m);
    }
    pub fn pmovzxbw_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_38_xm(Some(0x66), 0x30, d, m);
    }
    pub fn pmovsxwd_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_38_xm(Some(0x66), 0x23, d, m);
    }
    pub fn pmovzxwd_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_38_xm(Some(0x66), 0x33, d, m);
    }
    pub fn pmovsxdq_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_38_xm(Some(0x66), 0x25, d, m);
    }
    pub fn pmovzxdq_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_38_xm(Some(0x66), 0x35, d, m);
    }

    // Shifts (register count in xmm + immediate count)
    pub fn psllw(&mut self, d: Xmm, cnt: Xmm) {
        self.sse_rr(Some(0x66), 0xf1, d, cnt);
    }
    pub fn psrlw(&mut self, d: Xmm, cnt: Xmm) {
        self.sse_rr(Some(0x66), 0xd1, d, cnt);
    }
    pub fn psraw(&mut self, d: Xmm, cnt: Xmm) {
        self.sse_rr(Some(0x66), 0xe1, d, cnt);
    }
    pub fn pslld(&mut self, d: Xmm, cnt: Xmm) {
        self.sse_rr(Some(0x66), 0xf2, d, cnt);
    }
    pub fn psrld(&mut self, d: Xmm, cnt: Xmm) {
        self.sse_rr(Some(0x66), 0xd2, d, cnt);
    }
    pub fn psrad(&mut self, d: Xmm, cnt: Xmm) {
        self.sse_rr(Some(0x66), 0xe2, d, cnt);
    }
    pub fn psllq(&mut self, d: Xmm, cnt: Xmm) {
        self.sse_rr(Some(0x66), 0xf3, d, cnt);
    }
    pub fn psrlq(&mut self, d: Xmm, cnt: Xmm) {
        self.sse_rr(Some(0x66), 0xd3, d, cnt);
    }
    pub fn psllw_imm(&mut self, d: Xmm, imm: u8) {
        self.sse_shift_imm(0x71, 6, d, imm);
    }
    pub fn psrlw_imm(&mut self, d: Xmm, imm: u8) {
        self.sse_shift_imm(0x71, 2, d, imm);
    }
    pub fn psraw_imm(&mut self, d: Xmm, imm: u8) {
        self.sse_shift_imm(0x71, 4, d, imm);
    }
    pub fn pslld_imm(&mut self, d: Xmm, imm: u8) {
        self.sse_shift_imm(0x72, 6, d, imm);
    }
    pub fn psrld_imm(&mut self, d: Xmm, imm: u8) {
        self.sse_shift_imm(0x72, 2, d, imm);
    }
    pub fn psrad_imm(&mut self, d: Xmm, imm: u8) {
        self.sse_shift_imm(0x72, 4, d, imm);
    }
    pub fn psllq_imm(&mut self, d: Xmm, imm: u8) {
        self.sse_shift_imm(0x73, 6, d, imm);
    }
    pub fn psrlq_imm(&mut self, d: Xmm, imm: u8) {
        self.sse_shift_imm(0x73, 2, d, imm);
    }
    pub fn psrldq_imm(&mut self, d: Xmm, imm_bytes: u8) {
        self.sse_shift_imm(0x73, 3, d, imm_bytes);
    }
    pub fn pslldq_imm(&mut self, d: Xmm, imm_bytes: u8) {
        self.sse_shift_imm(0x73, 7, d, imm_bytes);
    }

    // Packed float arithmetic / comparisons / rounding / conversions
    pub fn addps(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(None, 0x58, d, s);
    }
    pub fn subps(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(None, 0x5c, d, s);
    }
    pub fn mulps(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(None, 0x59, d, s);
    }
    pub fn divps(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(None, 0x5e, d, s);
    }
    pub fn sqrtps(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(None, 0x51, d, s);
    }
    pub fn minps(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(None, 0x5d, d, s);
    }
    pub fn maxps(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(None, 0x5f, d, s);
    }
    pub fn minps_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(None, 0x5d, d, m);
    }
    pub fn maxps_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(None, 0x5f, d, m);
    }
    pub fn andps_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(None, 0x54, d, m);
    }
    pub fn andnps_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(None, 0x55, d, m);
    }
    pub fn orps_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(None, 0x56, d, m);
    }
    pub fn cmpps(&mut self, d: Xmm, s: Xmm, pred: u8) {
        self.sse_rri(None, 0xc2, d, s, pred);
    }
    pub fn roundps(&mut self, d: Xmm, s: Xmm, mode: u8) {
        self.sse_3a_rri(Some(0x66), 0x08, d, s, mode);
    }
    pub fn addpd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x58, d, s);
    }
    pub fn subpd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x5c, d, s);
    }
    pub fn mulpd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x59, d, s);
    }
    pub fn divpd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x5e, d, s);
    }
    pub fn sqrtpd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x51, d, s);
    }
    pub fn minpd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x5d, d, s);
    }
    pub fn maxpd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x5f, d, s);
    }
    pub fn minpd_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(Some(0x66), 0x5d, d, m);
    }
    pub fn maxpd_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(Some(0x66), 0x5f, d, m);
    }
    pub fn andpd_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(Some(0x66), 0x54, d, m);
    }
    pub fn andnpd_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(Some(0x66), 0x55, d, m);
    }
    pub fn orpd_xm(&mut self, d: Xmm, m: Mem) {
        self.sse_xm(Some(0x66), 0x56, d, m);
    }
    pub fn cmppd(&mut self, d: Xmm, s: Xmm, pred: u8) {
        self.sse_rri(Some(0x66), 0xc2, d, s, pred);
    }
    pub fn roundpd(&mut self, d: Xmm, s: Xmm, mode: u8) {
        self.sse_3a_rri(Some(0x66), 0x09, d, s, mode);
    }
    pub fn cvtdq2ps(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(None, 0x5b, d, s);
    }
    pub fn cvttps2dq(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0xf3), 0x5b, d, s);
    }
    pub fn cvtdq2pd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0xf3), 0xe6, d, s);
    }
    pub fn cvttpd2dq(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xe6, d, s);
    }
    pub fn cvtps2pd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(None, 0x5a, d, s);
    }
    pub fn cvtpd2ps(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x5a, d, s);
    }

    // Lane extract / insert (SSE2/SSE4.1)
    pub fn pextrb(&mut self, dst: Gp, src: Xmm, lane: u8) {
        self.emit_u8(0x66);
        self.rex(false, src.hi(), 0, dst.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x14);
        self.modrm_reg(src.lo(), dst.lo());
        self.emit_u8(lane & 15);
    }
    pub fn pextrw(&mut self, dst: Gp, src: Xmm, lane: u8) {
        self.emit_u8(0x66);
        self.rex(false, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0xc5);
        self.modrm_reg(dst.lo(), src.lo());
        self.emit_u8(lane & 7);
    }
    pub fn pextrd(&mut self, dst: Gp, src: Xmm, lane: u8) {
        self.emit_u8(0x66);
        self.rex(false, src.hi(), 0, dst.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x16);
        self.modrm_reg(src.lo(), dst.lo());
        self.emit_u8(lane & 3);
    }
    pub fn pextrq(&mut self, dst: Gp, src: Xmm, lane: u8) {
        self.emit_u8(0x66);
        self.rex(true, src.hi(), 0, dst.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x16);
        self.modrm_reg(src.lo(), dst.lo());
        self.emit_u8(lane & 1);
    }
    pub fn pinsrb(&mut self, dst: Xmm, src: Gp, lane: u8) {
        self.emit_u8(0x66);
        self.rex(false, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x20);
        self.modrm_reg(dst.lo(), src.lo());
        self.emit_u8(lane & 15);
    }
    pub fn pinsrw(&mut self, dst: Xmm, src: Gp, lane: u8) {
        self.emit_u8(0x66);
        self.rex(false, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0xc4);
        self.modrm_reg(dst.lo(), src.lo());
        self.emit_u8(lane & 7);
    }
    pub fn pinsrd(&mut self, dst: Xmm, src: Gp, lane: u8) {
        self.emit_u8(0x66);
        self.rex(false, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x22);
        self.modrm_reg(dst.lo(), src.lo());
        self.emit_u8(lane & 3);
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

    // Memory-lane extract/insert (SSE4.1)
    pub fn pinsrb_xm(&mut self, dst: Xmm, mem: Mem, lane: u8) {
        self.emit_u8(0x66);
        self.rex_mem(false, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x20);
        self.modrm_mem(dst.lo(), mem);
        self.emit_u8(lane & 15);
    }
    pub fn pinsrw_xm(&mut self, dst: Xmm, mem: Mem, lane: u8) {
        self.emit_u8(0x66);
        self.rex_mem(false, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0xc4);
        self.modrm_mem(dst.lo(), mem);
        self.emit_u8(lane & 7);
    }
    pub fn pinsrd_xm(&mut self, dst: Xmm, mem: Mem, lane: u8) {
        self.emit_u8(0x66);
        self.rex_mem(false, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x22);
        self.modrm_mem(dst.lo(), mem);
        self.emit_u8(lane & 3);
    }
    pub fn pinsrq_xm(&mut self, dst: Xmm, mem: Mem, lane: u8) {
        self.emit_u8(0x66);
        self.rex_mem(true, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x22);
        self.modrm_mem(dst.lo(), mem);
        self.emit_u8(lane & 1);
    }
    pub fn pextrb_mx(&mut self, mem: Mem, src: Xmm, lane: u8) {
        self.emit_u8(0x66);
        self.rex_mem(false, src.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x14);
        self.modrm_mem(src.lo(), mem);
        self.emit_u8(lane & 15);
    }
    pub fn pextrw_mx(&mut self, mem: Mem, src: Xmm, lane: u8) {
        self.emit_u8(0x66);
        self.rex_mem(false, src.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x15);
        self.modrm_mem(src.lo(), mem);
        self.emit_u8(lane & 7);
    }
    pub fn pextrd_mx(&mut self, mem: Mem, src: Xmm, lane: u8) {
        self.emit_u8(0x66);
        self.rex_mem(false, src.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x16);
        self.modrm_mem(src.lo(), mem);
        self.emit_u8(lane & 3);
    }
    pub fn pextrq_mx(&mut self, mem: Mem, src: Xmm, lane: u8) {
        self.emit_u8(0x66);
        self.rex_mem(true, src.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x16);
        self.modrm_mem(src.lo(), mem);
        self.emit_u8(lane & 1);
    }

    /// 3-byte VEX FMA3 (`x64-v3`): `vfmadd213{ss,sd,ps,pd} dst, vsrc, src` (`dst = dst * vsrc + src`).
    fn vex3_fma(&mut self, is64: bool, op: u8, dst: Xmm, vsrc: Xmm, src: Xmm) {
        let r_inv = if dst.hi() != 0 { 0 } else { 1 };
        let b_inv = if src.hi() != 0 { 0 } else { 1 };
        let w = u8::from(is64);
        let vvvv = (!vsrc.id()) & 0x0f;
        self.emit_u8(0xc4);
        self.emit_u8((r_inv << 7) | (1 << 6) | (b_inv << 5) | 0x02); // map 0F38
        self.emit_u8((w << 7) | (vvvv << 3) | 0x01); // L=0 (128-bit), pp=01 (66)
        self.emit_u8(op);
        self.modrm_reg(dst.lo(), src.lo());
    }
    pub fn vfmadd213ss(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_fma(false, 0xa9, d, a, b);
    }
    pub fn vfmadd213sd(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_fma(true, 0xa9, d, a, b);
    }
    pub fn vfmadd213ps(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_fma(false, 0xa8, d, a, b);
    }
    pub fn vfmadd213pd(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_fma(true, 0xa8, d, a, b);
    }

    // ---- AVX2 (256-bit YMM) VEX Encoders + XGETBV + VZEROUPPER ----

    /// Emits a 3-byte VEX prefix (`0xC4`) for 256-bit (`L=1`) register-register instructions.
    #[allow(clippy::too_many_arguments)]
    fn vex3_ymm_rr(
        &mut self,
        map: u8,
        pp: u8,
        w: bool,
        reg: Xmm,
        vvvv: Option<Xmm>,
        rm: Xmm,
        op: u8,
    ) {
        let r_inv = u8::from(reg.hi() == 0);
        let b_inv = u8::from(rm.hi() == 0);
        let v = vvvv.map_or(0x0f, |x| (!x.id()) & 0x0f);
        self.emit_u8(0xc4);
        self.emit_u8((r_inv << 7) | (1 << 6) | (b_inv << 5) | (map & 0x1f));
        self.emit_u8((u8::from(w) << 7) | (v << 3) | (1 << 2) | (pp & 0x03));
        self.emit_u8(op);
        self.modrm_reg(reg.lo(), rm.lo());
    }

    /// Emits a 3-byte VEX prefix (`0xC4`) for 256-bit (`L=1`) register-memory instructions.
    #[allow(clippy::too_many_arguments)]
    fn vex3_ymm_mem(
        &mut self,
        map: u8,
        pp: u8,
        w: bool,
        reg: Xmm,
        vvvv: Option<Xmm>,
        mem: Mem,
        op: u8,
    ) {
        let (x_hi, b_hi) = match mem {
            Mem::BaseDisp { base, .. } => (0, base.hi()),
            Mem::BaseIndexDisp { base, index, .. } => (index.hi(), base.hi()),
        };
        let r_inv = u8::from(reg.hi() == 0);
        let x_inv = u8::from(x_hi == 0);
        let b_inv = u8::from(b_hi == 0);
        let v = vvvv.map_or(0x0f, |x| (!x.id()) & 0x0f);
        self.emit_u8(0xc4);
        self.emit_u8((r_inv << 7) | (x_inv << 6) | (b_inv << 5) | (map & 0x1f));
        self.emit_u8((u8::from(w) << 7) | (v << 3) | (1 << 2) | (pp & 0x03));
        self.emit_u8(op);
        self.modrm_mem(reg.lo(), mem);
    }

    /// `vmovdqu ymm, m256` (`VEX.256.F3.0F.WIG 6F /r`).
    pub fn vmovdqu_ym(&mut self, dst: Xmm, mem: Mem) {
        self.vex3_ymm_mem(1, 2, false, dst, None, mem, 0x6f);
    }

    /// `vmovdqu m256, ymm` (`VEX.256.F3.0F.WIG 7F /r`).
    pub fn vmovdqu_my(&mut self, mem: Mem, src: Xmm) {
        self.vex3_ymm_mem(1, 2, false, src, None, mem, 0x7f);
    }

    /// `vpbroadcastb ymm, xmm` (`VEX.256.66.0F38.W0 78 /r`).
    pub fn vpbroadcastb_yx(&mut self, dst: Xmm, src: Xmm) {
        self.vex3_ymm_rr(2, 1, false, dst, None, src, 0x78);
    }

    /// `vpxor ymm, ymm, ymm` (`VEX.256.66.0F.WIG EF /r`).
    pub fn vpxor_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 1, false, dst, Some(a), b, 0xef);
    }

    /// `vpsadbw ymm, ymm, m256` (`VEX.256.66.0F.WIG F6 /r`).
    pub fn vpsadbw_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(1, 1, false, dst, Some(a), mem, 0xf6);
    }

    /// `vpaddq ymm, ymm, ymm` (`VEX.256.66.0F.WIG D4 /r`).
    pub fn vpaddq_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 1, false, dst, Some(a), b, 0xd4);
    }

    /// `vextracti128 xmm, ymm, imm8` (`VEX.256.66.0F3A.W0 39 /r ib`; `reg = src`, `rm = dst`).
    pub fn vextracti128_xy(&mut self, dst: Xmm, src: Xmm, imm: u8) {
        self.vex3_ymm_rr(3, 1, false, src, None, dst, 0x39);
        self.emit_u8(imm & 1);
    }

    /// `vzeroupper` (`C5 F8 77`): clears upper 128 bits of all YMM0..YMM15, eliminating AVX-SSE stalls.
    pub fn vzeroupper(&mut self) {
        self.emit_u8(0xc5);
        self.emit_u8(0xf8);
        self.emit_u8(0x77);
    }

    // ---- CPUID, XGETBV, MXCSR, and portable BSR/BSF ----

    pub fn cpuid(&mut self) {
        self.emit_u8(0x0f);
        self.emit_u8(0xa2);
    }

    /// `xgetbv` (`0F 01 D0`): reads extended control register `XCR[ecx]` into `EDX:EAX`.
    pub fn xgetbv(&mut self) {
        self.emit_u8(0x0f);
        self.emit_u8(0x01);
        self.emit_u8(0xd0);
    }

    /// `rdsspq dst` (`F3 [REX.W+B] 0F 1E /1`): architectural multi-byte NOP when CET shadow stack
    /// is inactive; writes the calling thread's non-zero shadow-stack pointer into `dst` when active.
    pub fn rdsspq(&mut self, dst: Gp) {
        self.emit_u8(0xf3);
        self.rex(true, 0, 0, dst.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x1e);
        self.modrm_reg(1, dst.lo());
    }

    pub fn stmxcsr(&mut self, mem: Mem) {
        self.rex_mem(false, 0, mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0xae);
        self.modrm_mem(3, mem);
    }

    pub fn ldmxcsr(&mut self, mem: Mem) {
        self.rex_mem(false, 0, mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0xae);
        self.modrm_mem(2, mem);
    }

    pub fn ldmxcsr_rip(&mut self, target: RelocTarget) {
        self.emit_u8(0x0f);
        self.emit_u8(0xae);
        self.emit_u8((2 << 3) | 5);
        let patch_off = self.pos();
        self.emit_i32(0);
        let next_ip_off = self.pos();
        self.relocs.push(Reloc {
            patch_off,
            next_ip_off,
            target,
        });
    }

    pub fn bsr(&mut self, is64: bool, dst: Gp, src: Gp) {
        self.rex(is64, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0xbd);
        self.modrm_reg(dst.lo(), src.lo());
    }

    pub fn bsf(&mut self, is64: bool, dst: Gp, src: Gp) {
        self.rex(is64, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0xbc);
        self.modrm_reg(dst.lo(), src.lo());
    }
}

impl Default for Asm {
    fn default() -> Self {
        Self::new()
    }
}
