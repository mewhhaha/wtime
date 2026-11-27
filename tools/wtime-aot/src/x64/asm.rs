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
    Xmm16 = 16,
    Xmm17 = 17,
    Xmm18 = 18,
    Xmm19 = 19,
    Xmm20 = 20,
    Xmm21 = 21,
    Xmm22 = 22,
    Xmm23 = 23,
    Xmm24 = 24,
    Xmm25 = 25,
    Xmm26 = 26,
    Xmm27 = 27,
    Xmm28 = 28,
    Xmm29 = 29,
    Xmm30 = 30,
    Xmm31 = 31,
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
        ((self as u8) >> 3) & 1
    }

    #[inline]
    pub const fn hi4(self) -> u8 {
        (self as u8) >> 4
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
            15 => Xmm::Xmm15,
            16 => Xmm::Xmm16,
            17 => Xmm::Xmm17,
            18 => Xmm::Xmm18,
            19 => Xmm::Xmm19,
            20 => Xmm::Xmm20,
            21 => Xmm::Xmm21,
            22 => Xmm::Xmm22,
            23 => Xmm::Xmm23,
            24 => Xmm::Xmm24,
            25 => Xmm::Xmm25,
            26 => Xmm::Xmm26,
            27 => Xmm::Xmm27,
            28 => Xmm::Xmm28,
            29 => Xmm::Xmm29,
            30 => Xmm::Xmm30,
            _ => Xmm::Xmm31,
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
    /// Start virtual address of compiled guest Wasm functions inside `.text`.
    GuestCodeStart,
    /// End virtual address of compiled guest Wasm functions inside `.text`.
    GuestCodeEnd,
    /// Virtual address of the 64-byte writable `GuardState` tail in `PT_LOAD 2` (`--guard-pages`).
    GuardState,
    /// Virtual address of the post-link sorted GC `SafepointHeader` table in `.rodata` (GC modules only).
    GcSafepointHeader,
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

/// Machine-code safepoint record emitted immediately after a GC-safepoint `call`.
///
/// `code_off` is the function-relative return address offset (adjusted automatically by
/// `finish_labels()` across both branch relaxation and loop-header alignment NOP padding).
/// `frame_caller_ret_off` equals `frame_size + used_callee_gp.len() * 8`: when unwinding from
/// this return address (`frame_rsp = ret_addr_rsp + 8`), the caller's return address sits at
/// `[frame_rsp + frame_caller_ret_off]`, and live GC/extern roots sit at `[frame_rsp + slot]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SafepointRecord {
    pub code_off: u32,
    pub frame_caller_ret_off: u32,
    pub gc_slots: Vec<i32>,
    pub ext_slots: Vec<i32>,
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
    /// GC safepoint records emitted after safepoint `call`s (empty for non-GC modules).
    pub safepoints: Vec<SafepointRecord>,
    labels: Vec<Option<u32>>,
    /// Optional per-label code alignment (`0` = none, `16` = loop header, `32` = hot AVX2 loop).
    align_labels: Vec<u8>,
    /// True when at least one label requested 32-byte alignment (so the linker aligns the function to 32B).
    pub has_align32: bool,
    /// When true (`x64-v3`/`x64-v4`), basic FP/V128 moves, loads, stores, and RIP ops emit VEX encodings
    /// automatically so functions never mix legacy SSE and VEX instructions.
    pub use_vex: bool,
    /// When true, the function emits wide 256/512-bit YMM/ZMM instructions without per-op
    /// `vzeroupper`, so `vzeroupper` is inserted automatically before `ret`, calls, tail-calls,
    /// and any legacy non-VEX SSE instruction.
    pub guard_legacy_sse_vzeroupper: bool,
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
            safepoints: Vec::new(),
            labels: Vec::new(),
            align_labels: Vec::new(),
            has_align32: false,
            use_vex: false,
            guard_legacy_sse_vzeroupper: false,
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
            safepoints: Vec::new(),
            labels: Vec::new(),
            align_labels: Vec::new(),
            has_align32: false,
            use_vex: false,
            guard_legacy_sse_vzeroupper: false,
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

    /// Records a precise GC safepoint at the current code offset (the return address immediately
    /// following a `call` instruction).
    pub fn record_safepoint(
        &mut self,
        frame_caller_ret_off: u32,
        gc_slots: Vec<i32>,
        ext_slots: Vec<i32>,
    ) {
        self.safepoints.push(SafepointRecord {
            code_off: self.pos(),
            frame_caller_ret_off,
            gc_slots,
            ext_slots,
        });
    }

    #[inline]
    pub fn new_label(&mut self) -> Label {
        let id = u32::try_from(self.labels.len()).expect("label count exceeded u32::MAX");
        self.labels.push(None);
        self.align_labels.push(0);
        Label(id)
    }

    #[inline]
    pub fn mark_align16(&mut self, label: Label) {
        if let Some(slot) = self.align_labels.get_mut(label.0 as usize)
            && *slot < 16
        {
            *slot = 16;
        }
    }

    #[inline]
    pub fn mark_align32(&mut self, label: Label) {
        if let Some(slot) = self.align_labels.get_mut(label.0 as usize) {
            *slot = 32;
            self.has_align32 = true;
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
        let has_align16 = self.align_labels.iter().any(|&a| a != 0);
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
            for sp in &mut self.safepoints {
                sp.code_off -= shift_of(sp.code_off);
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
            let mut align_offs: Vec<(u32, u8)> = self
                .labels
                .iter()
                .zip(&self.align_labels)
                .filter_map(|(&off, &want)| (want != 0).then_some(off).flatten().map(|o| (o, want)))
                .filter(|&(off, _)| off > 0 && off < code_len)
                .collect();
            align_offs.sort_unstable_by_key(|&(off, _)| off);
            // Deduplicate identical offsets, keeping the stricter alignment requirement.
            let mut deduped: Vec<(u32, u8)> = Vec::with_capacity(align_offs.len());
            for (off, req) in align_offs {
                if let Some(last) = deduped.last_mut()
                    && last.0 == off
                {
                    last.1 = last.1.max(req);
                } else {
                    deduped.push((off, req));
                }
            }

            let mut pads: Vec<(u32, u32)> = Vec::new();
            let mut total_pad: u32 = 0;
            for (orig_off, req) in deduped {
                let align = u32::from(req);
                let mask = align - 1;
                let cur_off = orig_off.wrapping_add(total_pad);
                let pad = (align - (cur_off & mask)) & mask;
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
                // A safepoint's `code_off` is the return address pushed by the preceding `call`.
                // Padding inserted AT `poff == sp.code_off` sits AFTER the `call` (before the next
                // aligned label), so only padding strictly BEFORE `sp.code_off` shifts it.
                for sp in &mut self.safepoints {
                    let idx = pads.partition_point(|&(poff, _)| poff < sp.code_off);
                    sp.code_off += pad_prefix[idx];
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
            safepoints: Vec::new(),
            labels,
            align_labels: Vec::new(),
            has_align32: false,
            use_vex: false,
            guard_legacy_sse_vzeroupper: false,
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

    pub fn lfence(&mut self) {
        self.code.extend_from_slice(&[0x0f, 0xae, 0xe8]);
    }

    pub fn mfence(&mut self) {
        self.code.extend_from_slice(&[0x0f, 0xae, 0xf0]);
    }

    pub fn nop(&mut self) {
        self.emit_u8(0x90);
    }

    pub fn ud2(&mut self) {
        self.emit_u8(0x0f);
        self.emit_u8(0x0b);
    }

    #[inline]
    fn maybe_vzeroupper(&mut self) {
        if self.guard_legacy_sse_vzeroupper {
            self.vzeroupper();
        }
    }

    pub fn ret(&mut self) {
        self.maybe_vzeroupper();
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

    pub fn cld(&mut self) {
        self.emit_u8(0xfc);
    }

    pub fn std_dir(&mut self) {
        self.emit_u8(0xfd);
    }

    pub fn rep_movsb(&mut self) {
        self.emit_u8(0xf3);
        self.emit_u8(0xa4);
    }

    pub fn rep_movsq(&mut self) {
        self.emit_u8(0xf3);
        self.emit_u8(0x48);
        self.emit_u8(0xa5);
    }

    pub fn rep_stosb(&mut self) {
        self.emit_u8(0xf3);
        self.emit_u8(0xaa);
    }

    pub fn rep_stosq(&mut self) {
        self.emit_u8(0xf3);
        self.emit_u8(0x48);
        self.emit_u8(0xab);
    }

    // ---- Branches & calls ----

    pub fn call_label(&mut self, label: Label) {
        self.maybe_vzeroupper();
        let inst_off = self.pos();
        self.emit_u8(0xe8);
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
        self.maybe_vzeroupper();
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
        self.maybe_vzeroupper();
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
        self.maybe_vzeroupper();
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
        if self.use_vex {
            self.vmovss_rip(dst, target);
            return;
        }
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
        if self.use_vex {
            self.vmovsd_rip(dst, target);
            return;
        }
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
        if self.use_vex {
            self.vmovdqu_rip(dst, target);
            return;
        }
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
        if self.use_vex {
            let (pp, evex_w) = match prefix {
                None => (0, false),
                Some(0x66) => (1, true),
                Some(0xf3) => (2, false),
                Some(0xf2) => (3, true),
                _ => unreachable!(),
            };
            // 2-operand VEX/EVEX ops (`vucomiss/d`, `vcomiss/d`, `vmovss/d`) require `vvvv = 1111` (`None`);
            // binary ops (`vand/or/xor`, `vadd/sub/mul/div/min/max`) take `vvvv = Some(dst)`.
            let vvvv = if matches!(op, 0x10 | 0x2e | 0x2f) {
                None
            } else {
                Some(dst)
            };
            self.vex_or_evex_rip(1, pp, false, evex_w, dst, vvvv, op, target);
            return;
        }
        assert!(dst.id() < 16, "SSE xmm_rip_op cannot encode Xmm16..31");
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

    /// `lock cmpxchg [mem], src`: compares `RAX/EAX` with `[mem]`; if equal, stores `src` into
    /// `[mem]` and sets `ZF=1`; otherwise loads `[mem]` into `RAX/EAX` and clears `ZF`.
    pub fn lock_cmpxchg_mr(&mut self, is64: bool, mem: Mem, src: Gp) {
        self.emit_u8(0xf0); // LOCK prefix
        self.rex_mem(is64, src.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0xb1);
        self.modrm_mem(src.lo(), mem);
    }

    /// `lock cmpxchg byte ptr [mem], src8` (compares `AL` with `[mem]`).
    pub fn lock_cmpxchg_mr8(&mut self, mem: Mem, src: Gp) {
        self.emit_u8(0xf0);
        self.rex_mem(false, src.hi(), mem, src.id() >= 4);
        self.emit_u8(0x0f);
        self.emit_u8(0xb0);
        self.modrm_mem(src.lo(), mem);
    }

    /// `lock cmpxchg word ptr [mem], src16` (compares `AX` with `[mem]`).
    pub fn lock_cmpxchg_mr16(&mut self, mem: Mem, src: Gp) {
        self.emit_u8(0xf0);
        self.emit_u8(0x66);
        self.rex_mem(false, src.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0xb1);
        self.modrm_mem(src.lo(), mem);
    }

    /// `lock xadd [mem], src`: atomically adds `src` into `[mem]` and returns old `[mem]` in `src`.
    pub fn lock_xadd_mr(&mut self, is64: bool, mem: Mem, src: Gp) {
        self.emit_u8(0xf0);
        self.rex_mem(is64, src.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0xc1);
        self.modrm_mem(src.lo(), mem);
    }

    /// `lock xadd byte ptr [mem], src8`.
    pub fn lock_xadd_mr8(&mut self, mem: Mem, src: Gp) {
        self.emit_u8(0xf0);
        self.rex_mem(false, src.hi(), mem, src.id() >= 4);
        self.emit_u8(0x0f);
        self.emit_u8(0xc0);
        self.modrm_mem(src.lo(), mem);
    }

    /// `lock xadd word ptr [mem], src16`.
    pub fn lock_xadd_mr16(&mut self, mem: Mem, src: Gp) {
        self.emit_u8(0xf0);
        self.emit_u8(0x66);
        self.rex_mem(false, src.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0xc1);
        self.modrm_mem(src.lo(), mem);
    }

    /// `xchg reg, [mem]` (implicit LOCK on x86-64: full sequential consistency).
    pub fn xchg_rm(&mut self, is64: bool, reg: Gp, mem: Mem) {
        self.rex_mem(is64, reg.hi(), mem, false);
        self.emit_u8(0x87);
        self.modrm_mem(reg.lo(), mem);
    }

    /// `xchg reg8, byte ptr [mem]` (implicit LOCK on x86-64).
    pub fn xchg_rm8(&mut self, reg: Gp, mem: Mem) {
        self.rex_mem(false, reg.hi(), mem, reg.id() >= 4);
        self.emit_u8(0x86);
        self.modrm_mem(reg.lo(), mem);
    }

    /// `xchg reg16, word ptr [mem]` (implicit LOCK on x86-64).
    pub fn xchg_rm16(&mut self, reg: Gp, mem: Mem) {
        self.emit_u8(0x66);
        self.rex_mem(false, reg.hi(), mem, false);
        self.emit_u8(0x87);
        self.modrm_mem(reg.lo(), mem);
    }

    /// `pause` (`F3 90`): spin-wait hint.
    pub fn pause(&mut self) {
        self.emit_u8(0xf3);
        self.emit_u8(0x90);
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
    pub fn sbb_rr(&mut self, is64: bool, dst: Gp, src: Gp) {
        self.alu_rr(0x19, is64, dst, src);
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
        self.maybe_vzeroupper();
        assert!(
            dst.id() < 16 && src.id() < 16,
            "legacy SSE encoder cannot encode Xmm16..31: dst={dst:?} src={src:?} op={op:#x}"
        );
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
            if self.use_vex {
                self.vmovaps_rr(dst, src);
            } else {
                self.sse_rr(None, 0x28, dst, src);
            }
        }
    }

    pub fn movss_xm(&mut self, dst: Xmm, mem: Mem) {
        if self.use_vex {
            self.vmovss_xm(dst, mem);
            return;
        }
        self.emit_u8(0xf3);
        self.rex_mem(false, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x10);
        self.modrm_mem(dst.lo(), mem);
    }

    pub fn movss_mx(&mut self, mem: Mem, src: Xmm) {
        if self.use_vex {
            self.vmovss_mx(mem, src);
            return;
        }
        self.emit_u8(0xf3);
        self.rex_mem(false, src.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x11);
        self.modrm_mem(src.lo(), mem);
    }

    pub fn movsd_xm(&mut self, dst: Xmm, mem: Mem) {
        if self.use_vex {
            self.vmovsd_xm(dst, mem);
            return;
        }
        self.emit_u8(0xf2);
        self.rex_mem(false, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x10);
        self.modrm_mem(dst.lo(), mem);
    }

    pub fn movsd_mx(&mut self, mem: Mem, src: Xmm) {
        if self.use_vex {
            self.vmovsd_mx(mem, src);
            return;
        }
        self.emit_u8(0xf2);
        self.rex_mem(false, src.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x11);
        self.modrm_mem(src.lo(), mem);
    }

    pub fn movdqu_xm(&mut self, dst: Xmm, mem: Mem) {
        if self.use_vex {
            self.vmovdqu_xm(dst, mem);
            return;
        }
        self.emit_u8(0xf3);
        self.rex_mem(false, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x6f);
        self.modrm_mem(dst.lo(), mem);
    }

    pub fn movdqu_mx(&mut self, mem: Mem, src: Xmm) {
        if self.use_vex {
            self.vmovdqu_mx(mem, src);
            return;
        }
        self.emit_u8(0xf3);
        self.rex_mem(false, src.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x7f);
        self.modrm_mem(src.lo(), mem);
    }

    pub fn movd_xr(&mut self, dst: Xmm, src: Gp) {
        if self.use_vex {
            self.vmovd_xr(dst, src);
            return;
        }
        self.emit_u8(0x66);
        self.rex(false, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x6e);
        self.modrm_reg(dst.lo(), src.lo());
    }

    pub fn movq_xr(&mut self, dst: Xmm, src: Gp) {
        if self.use_vex {
            self.vmovq_xr(dst, src);
            return;
        }
        self.emit_u8(0x66);
        self.rex(true, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x6e);
        self.modrm_reg(dst.lo(), src.lo());
    }

    pub fn movd_rx(&mut self, dst: Gp, src: Xmm) {
        if self.use_vex {
            self.vmovd_rx(dst, src);
            return;
        }
        self.emit_u8(0x66);
        self.rex(false, src.hi(), 0, dst.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x7e);
        self.modrm_reg(src.lo(), dst.lo());
    }

    pub fn movq_rx(&mut self, dst: Gp, src: Xmm) {
        if self.use_vex {
            self.vmovq_rx(dst, src);
            return;
        }
        self.emit_u8(0x66);
        self.rex(true, src.hi(), 0, dst.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x7e);
        self.modrm_reg(src.lo(), dst.lo());
    }

    fn sse_xm(&mut self, prefix: Option<u8>, op: u8, dst: Xmm, mem: Mem) {
        self.maybe_vzeroupper();
        assert!(
            dst.id() < 16,
            "legacy SSE memory encoder cannot encode Xmm16..31: dst={dst:?} op={op:#x}"
        );
        if let Some(p) = prefix {
            self.emit_u8(p);
        }
        self.rex_mem(false, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(op);
        self.modrm_mem(dst.lo(), mem);
    }

    pub fn addss_xm(&mut self, d: Xmm, m: Mem) {
        if self.use_vex {
            self.vex_or_evex_mem(1, 2, false, false, 4, d, Some(d), m, 0x58);
            return;
        }
        self.sse_xm(Some(0xf3), 0x58, d, m);
    }
    pub fn subss_xm(&mut self, d: Xmm, m: Mem) {
        if self.use_vex {
            self.vex_or_evex_mem(1, 2, false, false, 4, d, Some(d), m, 0x5c);
            return;
        }
        self.sse_xm(Some(0xf3), 0x5c, d, m);
    }
    pub fn mulss_xm(&mut self, d: Xmm, m: Mem) {
        if self.use_vex {
            self.vex_or_evex_mem(1, 2, false, false, 4, d, Some(d), m, 0x59);
            return;
        }
        self.sse_xm(Some(0xf3), 0x59, d, m);
    }
    pub fn divss_xm(&mut self, d: Xmm, m: Mem) {
        if self.use_vex {
            self.vex_or_evex_mem(1, 2, false, false, 4, d, Some(d), m, 0x5e);
            return;
        }
        self.sse_xm(Some(0xf3), 0x5e, d, m);
    }
    pub fn minss_xm(&mut self, d: Xmm, m: Mem) {
        if self.use_vex {
            self.vex_or_evex_mem(1, 2, false, false, 4, d, Some(d), m, 0x5d);
            return;
        }
        self.sse_xm(Some(0xf3), 0x5d, d, m);
    }
    pub fn maxss_xm(&mut self, d: Xmm, m: Mem) {
        if self.use_vex {
            self.vex_or_evex_mem(1, 2, false, false, 4, d, Some(d), m, 0x5f);
            return;
        }
        self.sse_xm(Some(0xf3), 0x5f, d, m);
    }
    pub fn ucomiss_xm(&mut self, a: Xmm, m: Mem) {
        if self.use_vex {
            self.vex_or_evex_mem(1, 0, false, false, 4, a, None, m, 0x2e);
            return;
        }
        self.sse_xm(None, 0x2e, a, m);
    }

    pub fn addsd_xm(&mut self, d: Xmm, m: Mem) {
        if self.use_vex {
            self.vex_or_evex_mem(1, 3, false, true, 8, d, Some(d), m, 0x58);
            return;
        }
        self.sse_xm(Some(0xf2), 0x58, d, m);
    }
    pub fn subsd_xm(&mut self, d: Xmm, m: Mem) {
        if self.use_vex {
            self.vex_or_evex_mem(1, 3, false, true, 8, d, Some(d), m, 0x5c);
            return;
        }
        self.sse_xm(Some(0xf2), 0x5c, d, m);
    }
    pub fn mulsd_xm(&mut self, d: Xmm, m: Mem) {
        if self.use_vex {
            self.vex_or_evex_mem(1, 3, false, true, 8, d, Some(d), m, 0x59);
            return;
        }
        self.sse_xm(Some(0xf2), 0x59, d, m);
    }
    pub fn divsd_xm(&mut self, d: Xmm, m: Mem) {
        if self.use_vex {
            self.vex_or_evex_mem(1, 3, false, true, 8, d, Some(d), m, 0x5e);
            return;
        }
        self.sse_xm(Some(0xf2), 0x5e, d, m);
    }
    pub fn minsd_xm(&mut self, d: Xmm, m: Mem) {
        if self.use_vex {
            self.vex_or_evex_mem(1, 3, false, true, 8, d, Some(d), m, 0x5d);
            return;
        }
        self.sse_xm(Some(0xf2), 0x5d, d, m);
    }
    pub fn maxsd_xm(&mut self, d: Xmm, m: Mem) {
        if self.use_vex {
            self.vex_or_evex_mem(1, 3, false, true, 8, d, Some(d), m, 0x5f);
            return;
        }
        self.sse_xm(Some(0xf2), 0x5f, d, m);
    }
    pub fn ucomisd_xm(&mut self, a: Xmm, m: Mem) {
        if self.use_vex {
            self.vex_or_evex_mem(1, 1, false, true, 8, a, None, m, 0x2e);
            return;
        }
        self.sse_xm(Some(0x66), 0x2e, a, m);
    }

    pub fn addss(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vex_or_evex_rr(1, 2, false, false, d, Some(d), s, 0x58);
            return;
        }
        self.sse_rr(Some(0xf3), 0x58, d, s);
    }
    pub fn subss(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vex_or_evex_rr(1, 2, false, false, d, Some(d), s, 0x5c);
            return;
        }
        self.sse_rr(Some(0xf3), 0x5c, d, s);
    }
    pub fn mulss(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vex_or_evex_rr(1, 2, false, false, d, Some(d), s, 0x59);
            return;
        }
        self.sse_rr(Some(0xf3), 0x59, d, s);
    }
    pub fn divss(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vex_or_evex_rr(1, 2, false, false, d, Some(d), s, 0x5e);
            return;
        }
        self.sse_rr(Some(0xf3), 0x5e, d, s);
    }
    pub fn sqrtss(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vex_or_evex_rr(1, 2, false, false, d, Some(d), s, 0x51);
            return;
        }
        self.sse_rr(Some(0xf3), 0x51, d, s);
    }
    pub fn minss(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vex_or_evex_rr(1, 2, false, false, d, Some(d), s, 0x5d);
            return;
        }
        self.sse_rr(Some(0xf3), 0x5d, d, s);
    }
    pub fn maxss(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vex_or_evex_rr(1, 2, false, false, d, Some(d), s, 0x5f);
            return;
        }
        self.sse_rr(Some(0xf3), 0x5f, d, s);
    }
    pub fn ucomiss(&mut self, a: Xmm, b: Xmm) {
        if self.use_vex {
            self.vex_or_evex_rr(1, 0, false, false, a, None, b, 0x2e);
            return;
        }
        self.sse_rr(None, 0x2e, a, b);
    }

    pub fn addsd(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vex_or_evex_rr(1, 3, false, true, d, Some(d), s, 0x58);
            return;
        }
        self.sse_rr(Some(0xf2), 0x58, d, s);
    }
    pub fn subsd(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vex_or_evex_rr(1, 3, false, true, d, Some(d), s, 0x5c);
            return;
        }
        self.sse_rr(Some(0xf2), 0x5c, d, s);
    }
    pub fn mulsd(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vex_or_evex_rr(1, 3, false, true, d, Some(d), s, 0x59);
            return;
        }
        self.sse_rr(Some(0xf2), 0x59, d, s);
    }
    pub fn divsd(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vex_or_evex_rr(1, 3, false, true, d, Some(d), s, 0x5e);
            return;
        }
        self.sse_rr(Some(0xf2), 0x5e, d, s);
    }
    pub fn sqrtsd(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vex_or_evex_rr(1, 3, false, true, d, Some(d), s, 0x51);
            return;
        }
        self.sse_rr(Some(0xf2), 0x51, d, s);
    }
    pub fn minsd(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vex_or_evex_rr(1, 3, false, true, d, Some(d), s, 0x5d);
            return;
        }
        self.sse_rr(Some(0xf2), 0x5d, d, s);
    }
    pub fn maxsd(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vex_or_evex_rr(1, 3, false, true, d, Some(d), s, 0x5f);
            return;
        }
        self.sse_rr(Some(0xf2), 0x5f, d, s);
    }
    pub fn ucomisd(&mut self, a: Xmm, b: Xmm) {
        if self.use_vex {
            self.vex_or_evex_rr(1, 1, false, true, a, None, b, 0x2e);
            return;
        }
        self.sse_rr(Some(0x66), 0x2e, a, b);
    }

    pub fn andps(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vandps_xxx(d, d, s);
            return;
        }
        self.sse_rr(None, 0x54, d, s);
    }
    pub fn andnps(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vandnps_xxx(d, d, s);
            return;
        }
        self.sse_rr(None, 0x55, d, s);
    }
    pub fn orps(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vorps_xxx(d, d, s);
            return;
        }
        self.sse_rr(None, 0x56, d, s);
    }
    pub fn xorps(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vxorps_xxx(d, d, s);
            return;
        }
        self.sse_rr(None, 0x57, d, s);
    }

    pub fn cvtss2sd(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vex_or_evex_rr(1, 2, false, false, d, Some(d), s, 0x5a);
            return;
        }
        self.sse_rr(Some(0xf3), 0x5a, d, s);
    }
    pub fn cvtsd2ss(&mut self, d: Xmm, s: Xmm) {
        if self.use_vex {
            self.vex_or_evex_rr(1, 3, false, true, d, Some(d), s, 0x5a);
            return;
        }
        self.sse_rr(Some(0xf2), 0x5a, d, s);
    }

    pub fn cvtsi2ss(&mut self, is64: bool, d: Xmm, s: Gp) {
        if self.use_vex {
            self.vex_or_evex_xmm_gp(1, 2, is64, d, Some(d), s, 0x2a);
            return;
        }
        assert!(d.id() < 16, "SSE cvtsi2ss cannot encode Xmm16..31");
        self.emit_u8(0xf3);
        self.rex(is64, d.hi(), 0, s.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x2a);
        self.modrm_reg(d.lo(), s.lo());
    }

    pub fn cvtsi2sd(&mut self, is64: bool, d: Xmm, s: Gp) {
        if self.use_vex {
            self.vex_or_evex_xmm_gp(1, 3, is64, d, Some(d), s, 0x2a);
            return;
        }
        assert!(d.id() < 16, "SSE cvtsi2sd cannot encode Xmm16..31");
        self.emit_u8(0xf2);
        self.rex(is64, d.hi(), 0, s.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x2a);
        self.modrm_reg(d.lo(), s.lo());
    }

    pub fn cvttss2si(&mut self, is64: bool, d: Gp, s: Xmm) {
        if self.use_vex {
            self.vex_or_evex_gp_xmm(1, 2, is64, d, s, 0x2c);
            return;
        }
        assert!(s.id() < 16, "SSE cvttss2si cannot encode Xmm16..31");
        self.emit_u8(0xf3);
        self.rex(is64, d.hi(), 0, s.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x2c);
        self.modrm_reg(d.lo(), s.lo());
    }

    pub fn cvttsd2si(&mut self, is64: bool, d: Gp, s: Xmm) {
        if self.use_vex {
            self.vex_or_evex_gp_xmm(1, 3, is64, d, s, 0x2c);
            return;
        }
        assert!(s.id() < 16, "SSE cvttsd2si cannot encode Xmm16..31");
        self.emit_u8(0xf2);
        self.rex(is64, d.hi(), 0, s.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x2c);
        self.modrm_reg(d.lo(), s.lo());
    }

    /// SSE4.1 `roundss dst, src, imm8` (`0x08`=nearest, `0x09`=floor, `0x0a`=ceil, `0x0b`=trunc).
    pub fn roundss(&mut self, d: Xmm, s: Xmm, mode: u8) {
        self.maybe_vzeroupper();
        assert!(
            d.id() < 16 && s.id() < 16,
            "roundss cannot encode Xmm16..31"
        );
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
        self.maybe_vzeroupper();
        assert!(
            d.id() < 16 && s.id() < 16,
            "roundsd cannot encode Xmm16..31"
        );
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
        self.maybe_vzeroupper();
        assert!(
            dst.id() < 16 && src.id() < 16,
            "legacy SSE 0F38 encoder cannot encode Xmm16..31: dst={dst:?} src={src:?} op={op:#x}"
        );
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
        self.maybe_vzeroupper();
        assert!(
            dst.id() < 16,
            "legacy SSE 0F38 mem encoder cannot encode Xmm16..31: dst={dst:?} op={op:#x}"
        );
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
        self.maybe_vzeroupper();
        assert!(
            dst.id() < 16 && src.id() < 16,
            "legacy SSE 0F3A encoder cannot encode Xmm16..31: dst={dst:?} src={src:?} op={op:#x}"
        );
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
        self.maybe_vzeroupper();
        assert!(
            dst.id() < 16,
            "legacy SSE shift_imm cannot encode Xmm16..31"
        );
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
    pub fn punpckhbw(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x68, d, s);
    }
    pub fn punpcklwd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x61, d, s);
    }
    pub fn punpckhwd(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x69, d, s);
    }
    pub fn punpckldq(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x62, d, s);
    }
    pub fn punpckhdq(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x6a, d, s);
    }
    pub fn punpcklqdq(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x6c, d, s);
    }
    pub fn punpckhqdq(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0x6d, d, s);
    }
    pub fn palignr(&mut self, d: Xmm, s: Xmm, imm: u8) {
        self.sse_3a_rri(Some(0x66), 0x0f, d, s, imm);
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
        if self.use_vex {
            self.vex_or_evex_rip(2, 1, false, false, dst, Some(dst), 0x00, target);
            return;
        }
        self.maybe_vzeroupper();
        assert!(dst.id() < 16, "SSE pshufb_rip cannot encode Xmm16..31");
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
        self.maybe_vzeroupper();
        self.emit_u8(0x66);
        self.rex(false, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0xd7);
        self.modrm_reg(dst.lo(), src.lo());
    }
    pub fn movmskps(&mut self, dst: Gp, src: Xmm) {
        self.maybe_vzeroupper();
        self.rex(false, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x50);
        self.modrm_reg(dst.lo(), src.lo());
    }
    pub fn movmskpd(&mut self, dst: Gp, src: Xmm) {
        self.maybe_vzeroupper();
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
    pub fn pmulhuw(&mut self, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), 0xe4, d, s);
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
    pub fn pmovzxbd(&mut self, d: Xmm, s: Xmm) {
        self.sse_38_rr(Some(0x66), 0x31, d, s);
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
        self.maybe_vzeroupper();
        assert!(src.id() < 16, "SSE pextrb cannot encode Xmm16..31");
        self.emit_u8(0x66);
        self.rex(false, src.hi(), 0, dst.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x14);
        self.modrm_reg(src.lo(), dst.lo());
        self.emit_u8(lane & 15);
    }
    pub fn pextrw(&mut self, dst: Gp, src: Xmm, lane: u8) {
        self.maybe_vzeroupper();
        assert!(src.id() < 16, "SSE pextrw cannot encode Xmm16..31");
        self.emit_u8(0x66);
        self.rex(false, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0xc5);
        self.modrm_reg(dst.lo(), src.lo());
        self.emit_u8(lane & 7);
    }
    pub fn pextrd(&mut self, dst: Gp, src: Xmm, lane: u8) {
        self.maybe_vzeroupper();
        assert!(src.id() < 16, "SSE pextrd cannot encode Xmm16..31");
        self.emit_u8(0x66);
        self.rex(false, src.hi(), 0, dst.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x16);
        self.modrm_reg(src.lo(), dst.lo());
        self.emit_u8(lane & 3);
    }
    pub fn pextrq(&mut self, dst: Gp, src: Xmm, lane: u8) {
        self.maybe_vzeroupper();
        assert!(src.id() < 16, "SSE pextrq cannot encode Xmm16..31");
        self.emit_u8(0x66);
        self.rex(true, src.hi(), 0, dst.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x16);
        self.modrm_reg(src.lo(), dst.lo());
        self.emit_u8(lane & 1);
    }
    pub fn pinsrb(&mut self, dst: Xmm, src: Gp, lane: u8) {
        self.maybe_vzeroupper();
        assert!(dst.id() < 16, "SSE pinsrb cannot encode Xmm16..31");
        self.emit_u8(0x66);
        self.rex(false, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x20);
        self.modrm_reg(dst.lo(), src.lo());
        self.emit_u8(lane & 15);
    }
    pub fn pinsrw(&mut self, dst: Xmm, src: Gp, lane: u8) {
        self.maybe_vzeroupper();
        assert!(dst.id() < 16, "SSE pinsrw cannot encode Xmm16..31");
        self.emit_u8(0x66);
        self.rex(false, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0xc4);
        self.modrm_reg(dst.lo(), src.lo());
        self.emit_u8(lane & 7);
    }
    pub fn pinsrd(&mut self, dst: Xmm, src: Gp, lane: u8) {
        self.maybe_vzeroupper();
        assert!(dst.id() < 16, "SSE pinsrd cannot encode Xmm16..31");
        self.emit_u8(0x66);
        self.rex(false, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x22);
        self.modrm_reg(dst.lo(), src.lo());
        self.emit_u8(lane & 3);
    }
    pub fn pinsrq(&mut self, dst: Xmm, src: Gp, lane: u8) {
        self.maybe_vzeroupper();
        assert!(dst.id() < 16, "SSE pinsrq cannot encode Xmm16..31");
        self.emit_u8(0x66);
        self.rex(true, dst.hi(), 0, src.hi(), false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x22);
        self.modrm_reg(dst.lo(), src.lo());
        self.emit_u8(lane & 1);
    }

    // Memory-lane extract/insert (SSE4.1 / AVX / AVX-512VL+BW+DQ)
    pub fn pinsrb_xm(&mut self, dst: Xmm, mem: Mem, lane: u8) {
        if self.use_vex {
            self.vex_or_evex_mem(3, 1, false, false, 1, dst, Some(dst), mem, 0x20);
            self.emit_u8(lane & 15);
            return;
        }
        self.maybe_vzeroupper();
        assert!(dst.id() < 16, "SSE pinsrb_xm cannot encode Xmm16..31");
        self.emit_u8(0x66);
        self.rex_mem(false, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x20);
        self.modrm_mem(dst.lo(), mem);
        self.emit_u8(lane & 15);
    }
    pub fn pinsrw_xm(&mut self, dst: Xmm, mem: Mem, lane: u8) {
        if self.use_vex {
            self.vex_or_evex_mem(1, 1, false, false, 2, dst, Some(dst), mem, 0xc4);
            self.emit_u8(lane & 7);
            return;
        }
        self.maybe_vzeroupper();
        assert!(dst.id() < 16, "SSE pinsrw_xm cannot encode Xmm16..31");
        self.emit_u8(0x66);
        self.rex_mem(false, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0xc4);
        self.modrm_mem(dst.lo(), mem);
        self.emit_u8(lane & 7);
    }
    pub fn pinsrd_xm(&mut self, dst: Xmm, mem: Mem, lane: u8) {
        if self.use_vex {
            self.vex_or_evex_mem(3, 1, false, false, 4, dst, Some(dst), mem, 0x22);
            self.emit_u8(lane & 3);
            return;
        }
        self.maybe_vzeroupper();
        assert!(dst.id() < 16, "SSE pinsrd_xm cannot encode Xmm16..31");
        self.emit_u8(0x66);
        self.rex_mem(false, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x22);
        self.modrm_mem(dst.lo(), mem);
        self.emit_u8(lane & 3);
    }
    pub fn pinsrq_xm(&mut self, dst: Xmm, mem: Mem, lane: u8) {
        if self.use_vex {
            self.vex_or_evex_mem(3, 1, true, true, 8, dst, Some(dst), mem, 0x22);
            self.emit_u8(lane & 1);
            return;
        }
        self.maybe_vzeroupper();
        assert!(dst.id() < 16, "SSE pinsrq_xm cannot encode Xmm16..31");
        self.emit_u8(0x66);
        self.rex_mem(true, dst.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x22);
        self.modrm_mem(dst.lo(), mem);
        self.emit_u8(lane & 1);
    }
    pub fn pextrb_mx(&mut self, mem: Mem, src: Xmm, lane: u8) {
        if self.use_vex {
            self.vex_or_evex_mem(3, 1, false, false, 1, src, None, mem, 0x14);
            self.emit_u8(lane & 15);
            return;
        }
        self.maybe_vzeroupper();
        assert!(src.id() < 16, "SSE pextrb_mx cannot encode Xmm16..31");
        self.emit_u8(0x66);
        self.rex_mem(false, src.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x14);
        self.modrm_mem(src.lo(), mem);
        self.emit_u8(lane & 15);
    }
    pub fn pextrw_mx(&mut self, mem: Mem, src: Xmm, lane: u8) {
        if self.use_vex {
            self.vex_or_evex_mem(3, 1, false, false, 2, src, None, mem, 0x15);
            self.emit_u8(lane & 7);
            return;
        }
        self.maybe_vzeroupper();
        assert!(src.id() < 16, "SSE pextrw_mx cannot encode Xmm16..31");
        self.emit_u8(0x66);
        self.rex_mem(false, src.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x15);
        self.modrm_mem(src.lo(), mem);
        self.emit_u8(lane & 7);
    }
    pub fn pextrd_mx(&mut self, mem: Mem, src: Xmm, lane: u8) {
        if self.use_vex {
            self.vex_or_evex_mem(3, 1, false, false, 4, src, None, mem, 0x16);
            self.emit_u8(lane & 3);
            return;
        }
        self.maybe_vzeroupper();
        assert!(src.id() < 16, "SSE pextrd_mx cannot encode Xmm16..31");
        self.emit_u8(0x66);
        self.rex_mem(false, src.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x16);
        self.modrm_mem(src.lo(), mem);
        self.emit_u8(lane & 3);
    }
    pub fn pextrq_mx(&mut self, mem: Mem, src: Xmm, lane: u8) {
        if self.use_vex {
            self.vex_or_evex_mem(3, 1, true, true, 8, src, None, mem, 0x16);
            self.emit_u8(lane & 1);
            return;
        }
        self.maybe_vzeroupper();
        assert!(src.id() < 16, "SSE pextrq_mx cannot encode Xmm16..31");
        self.emit_u8(0x66);
        self.rex_mem(true, src.hi(), mem, false);
        self.emit_u8(0x0f);
        self.emit_u8(0x3a);
        self.emit_u8(0x16);
        self.modrm_mem(src.lo(), mem);
        self.emit_u8(lane & 1);
    }

    /// 3-byte VEX (or 4-byte EVEX for `Xmm16..31`) FMA3: `vfmadd213{ss,sd,ps,pd} dst, vsrc, src`.
    fn vex3_fma(&mut self, is64: bool, op: u8, dst: Xmm, vsrc: Xmm, src: Xmm) {
        self.vex_or_evex_rr(2, 1, is64, is64, dst, Some(vsrc), src, op);
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
    pub fn vfnmadd213ss(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_fma(false, 0xad, d, a, b);
    }
    pub fn vfnmadd213sd(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_fma(true, 0xad, d, a, b);
    }
    pub fn vfnmadd213ps(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_fma(false, 0xac, d, a, b);
    }
    pub fn vfnmadd213pd(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_fma(true, 0xac, d, a, b);
    }
    pub fn vfmadd231ps(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_fma(false, 0xb8, d, a, b);
    }
    pub fn vfmadd231pd(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_fma(true, 0xb8, d, a, b);
    }
    pub fn vfnmadd231ps(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_fma(false, 0xbc, d, a, b);
    }
    pub fn vfnmadd231pd(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_fma(true, 0xbc, d, a, b);
    }
    pub fn vfmadd213ps_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(2, 1, false, false, 16, d, Some(a), mem, 0xa8);
    }
    pub fn vfnmadd213ps_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(2, 1, false, false, 16, d, Some(a), mem, 0xac);
    }
    pub fn vfmadd213pd_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(2, 1, true, true, 16, d, Some(a), mem, 0xa8);
    }
    pub fn vfnmadd213pd_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(2, 1, true, true, 16, d, Some(a), mem, 0xac);
    }
    pub fn vfmadd231ps_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(2, 1, false, false, 16, d, Some(a), mem, 0xb8);
    }
    pub fn vfnmadd231ps_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(2, 1, false, false, 16, d, Some(a), mem, 0xbc);
    }
    pub fn vfmadd231pd_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(2, 1, true, true, 16, d, Some(a), mem, 0xb8);
    }
    pub fn vfnmadd231pd_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(2, 1, true, true, 16, d, Some(a), mem, 0xbc);
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
        assert!(
            reg.id() < 16 && rm.id() < 16 && vvvv.is_none_or(|x| x.id() < 16),
            "VEX 256-bit rr cannot encode Xmm16..31"
        );
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
        assert!(
            reg.id() < 16 && vvvv.is_none_or(|x| x.id() < 16),
            "VEX 256-bit mem cannot encode Xmm16..31"
        );
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

    /// Emits a 3-byte VEX prefix (`0xC4`) for 128-bit (`L=0`) register-register instructions.
    #[allow(clippy::too_many_arguments)]
    fn vex3_xmm_rr(
        &mut self,
        map: u8,
        pp: u8,
        w: bool,
        reg: Xmm,
        vvvv: Option<Xmm>,
        rm: Xmm,
        op: u8,
    ) {
        assert!(
            reg.id() < 16 && rm.id() < 16 && vvvv.is_none_or(|x| x.id() < 16),
            "VEX 128-bit rr cannot encode Xmm16..31: reg={reg:?} vvvv={vvvv:?} rm={rm:?} op={op:#x}"
        );
        let r_inv = u8::from(reg.hi() == 0);
        let b_inv = u8::from(rm.hi() == 0);
        let v = vvvv.map_or(0x0f, |x| (!x.id()) & 0x0f);
        self.emit_u8(0xc4);
        self.emit_u8((r_inv << 7) | (1 << 6) | (b_inv << 5) | (map & 0x1f));
        self.emit_u8((u8::from(w) << 7) | (v << 3) | (pp & 0x03));
        self.emit_u8(op);
        self.modrm_reg(reg.lo(), rm.lo());
    }

    /// Emits a 3-byte VEX prefix (`0xC4`) for 128-bit (`L=0`) register-memory instructions.
    #[allow(clippy::too_many_arguments)]
    fn vex3_xmm_mem(
        &mut self,
        map: u8,
        pp: u8,
        w: bool,
        reg: Xmm,
        vvvv: Option<Xmm>,
        mem: Mem,
        op: u8,
    ) {
        assert!(
            reg.id() < 16 && vvvv.is_none_or(|x| x.id() < 16),
            "VEX 128-bit mem cannot encode Xmm16..31: reg={reg:?} vvvv={vvvv:?} op={op:#x}"
        );
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
        self.emit_u8((u8::from(w) << 7) | (v << 3) | (pp & 0x03));
        self.emit_u8(op);
        self.modrm_mem(reg.lo(), mem);
    }

    /// Emits a 3-byte VEX prefix (`0xC4`) for 128-bit (`L=0`) RIP-relative memory instructions.
    #[allow(clippy::too_many_arguments)]
    fn vex3_xmm_rip(
        &mut self,
        map: u8,
        pp: u8,
        w: bool,
        reg: Xmm,
        vvvv: Option<Xmm>,
        op: u8,
        target: RelocTarget,
    ) {
        assert!(
            reg.id() < 16 && vvvv.is_none_or(|x| x.id() < 16),
            "VEX 128-bit rip cannot encode Xmm16..31: reg={reg:?} vvvv={vvvv:?} op={op:#x}"
        );
        let r_inv = u8::from(reg.hi() == 0);
        let v = vvvv.map_or(0x0f, |x| (!x.id()) & 0x0f);
        self.emit_u8(0xc4);
        self.emit_u8((r_inv << 7) | (1 << 6) | (1 << 5) | (map & 0x1f));
        self.emit_u8((u8::from(w) << 7) | (v << 3) | (pp & 0x03));
        self.emit_u8(op);
        self.emit_u8((reg.lo() << 3) | 5);
        let patch_off = self.pos();
        self.emit_i32(0);
        let next_ip_off = self.pos();
        self.relocs.push(Reloc {
            patch_off,
            next_ip_off,
            target,
        });
    }

    /// Emits a 3-byte VEX 128-bit instruction with `reg = Xmm`, optional `vvvv = Xmm`, `rm = Gp`.
    #[allow(clippy::too_many_arguments)]
    fn vex3_xmm_gp(
        &mut self,
        map: u8,
        pp: u8,
        w: bool,
        reg: Xmm,
        vvvv: Option<Xmm>,
        rm_gp: Gp,
        op: u8,
    ) {
        assert!(
            reg.id() < 16 && vvvv.is_none_or(|x| x.id() < 16),
            "VEX 128-bit xmm_gp cannot encode Xmm16..31"
        );
        let r_inv = u8::from(reg.hi() == 0);
        let b_inv = u8::from(rm_gp.hi() == 0);
        let v = vvvv.map_or(0x0f, |x| (!x.id()) & 0x0f);
        self.emit_u8(0xc4);
        self.emit_u8((r_inv << 7) | (1 << 6) | (b_inv << 5) | (map & 0x1f));
        self.emit_u8((u8::from(w) << 7) | (v << 3) | (pp & 0x03));
        self.emit_u8(op);
        self.modrm_reg(reg.lo(), rm_gp.lo());
    }

    /// Emits a 3-byte VEX 128-bit instruction with `reg = Gp`, `rm = Xmm` (e.g. `vpmovmskb`/`vmovmskps`/`vpextrw`).
    fn vex3_gp_xmm(&mut self, map: u8, pp: u8, w: bool, reg_gp: Gp, rm: Xmm, op: u8) {
        assert!(rm.id() < 16, "VEX 128-bit gp_xmm cannot encode Xmm16..31");
        let r_inv = u8::from(reg_gp.hi() == 0);
        let b_inv = u8::from(rm.hi() == 0);
        self.emit_u8(0xc4);
        self.emit_u8((r_inv << 7) | (1 << 6) | (b_inv << 5) | (map & 0x1f));
        self.emit_u8((u8::from(w) << 7) | (0x0f << 3) | (pp & 0x03));
        self.emit_u8(op);
        self.modrm_reg(reg_gp.lo(), rm.lo());
    }

    /// Emits a 3-byte VEX 128-bit immediate shift (`VEX.128.66.0F.WIG op /reg_ext ib`).
    fn vex3_xmm_shift_imm(&mut self, op: u8, reg_ext: u8, dst: Xmm, src: Xmm, imm: u8) {
        assert!(
            dst.id() < 16 && src.id() < 16,
            "VEX 128-bit shift_imm cannot encode Xmm16..31"
        );
        let b_inv = u8::from(src.hi() == 0);
        let v = (!dst.id()) & 0x0f;
        self.emit_u8(0xc4);
        self.emit_u8((1 << 7) | (1 << 6) | (b_inv << 5) | 1);
        self.emit_u8((v << 3) | 1);
        self.emit_u8(op);
        self.modrm_reg(reg_ext, src.lo());
        self.emit_u8(imm);
    }

    /// Emits a 3-byte VEX 256-bit immediate shift (`VEX.256.66.0F.WIG op /reg_ext ib`).
    fn vex3_ymm_shift_imm(&mut self, op: u8, reg_ext: u8, dst: Xmm, src: Xmm, imm: u8) {
        assert!(
            dst.id() < 16 && src.id() < 16,
            "VEX 256-bit shift_imm cannot encode Xmm16..31"
        );
        let b_inv = u8::from(src.hi() == 0);
        let v = (!dst.id()) & 0x0f;
        self.emit_u8(0xc4);
        self.emit_u8((1 << 7) | (1 << 6) | (b_inv << 5) | 1);
        self.emit_u8((v << 3) | (1 << 2) | 1);
        self.emit_u8(op);
        self.modrm_reg(reg_ext, src.lo());
        self.emit_u8(imm);
    }

    /// Emits 128-bit VEX when all XMM operands are `< 16`, or 128-bit EVEX (`AVX-512VL`) when any operand is `Xmm16..Xmm31`.
    #[allow(clippy::too_many_arguments)]
    fn vex_or_evex_rr(
        &mut self,
        map: u8,
        pp: u8,
        vex_w: bool,
        evex_w: bool,
        reg: Xmm,
        vvvv: Option<Xmm>,
        rm: Xmm,
        op: u8,
    ) {
        if reg.id() >= 16 || rm.id() >= 16 || vvvv.is_some_and(|x| x.id() >= 16) {
            self.evex_rr(map, pp, evex_w, 0, reg, vvvv, rm, op);
        } else {
            self.vex3_xmm_rr(map, pp, vex_w, reg, vvvv, rm, op);
        }
    }

    /// Emits 128-bit VEX memory op when all XMM operands are `< 16`, or 128-bit EVEX when any operand is `Xmm16..Xmm31`.
    #[allow(clippy::too_many_arguments)]
    fn vex_or_evex_mem(
        &mut self,
        map: u8,
        pp: u8,
        vex_w: bool,
        evex_w: bool,
        disp8_scale: i32,
        reg: Xmm,
        vvvv: Option<Xmm>,
        mem: Mem,
        op: u8,
    ) {
        if reg.id() >= 16 || vvvv.is_some_and(|x| x.id() >= 16) {
            self.evex_mem(map, pp, evex_w, 0, disp8_scale, reg, vvvv, mem, op);
        } else {
            self.vex3_xmm_mem(map, pp, vex_w, reg, vvvv, mem, op);
        }
    }

    /// Emits 128-bit VEX RIP-relative op when all XMM operands are `< 16`, or 128-bit EVEX when any operand is `Xmm16..Xmm31`.
    #[allow(clippy::too_many_arguments)]
    fn vex_or_evex_rip(
        &mut self,
        map: u8,
        pp: u8,
        vex_w: bool,
        evex_w: bool,
        reg: Xmm,
        vvvv: Option<Xmm>,
        op: u8,
        target: RelocTarget,
    ) {
        if reg.id() >= 16 || vvvv.is_some_and(|x| x.id() >= 16) {
            self.evex_rip(map, pp, evex_w, 0, reg, vvvv, op, target);
        } else {
            self.vex3_xmm_rip(map, pp, vex_w, reg, vvvv, op, target);
        }
    }

    /// Emits 128-bit VEX `reg=Xmm, rm=Gp` when all XMM operands are `< 16`, or 128-bit EVEX for `Xmm16..Xmm31`.
    #[allow(clippy::too_many_arguments)]
    fn vex_or_evex_xmm_gp(
        &mut self,
        map: u8,
        pp: u8,
        w: bool,
        reg: Xmm,
        vvvv: Option<Xmm>,
        rm_gp: Gp,
        op: u8,
    ) {
        if reg.id() >= 16 || vvvv.is_some_and(|x| x.id() >= 16) {
            self.evex_xmm_gp(map, pp, w, reg, vvvv, rm_gp, op);
        } else {
            self.vex3_xmm_gp(map, pp, w, reg, vvvv, rm_gp, op);
        }
    }

    /// Emits 128-bit VEX `reg=Gp, rm=Xmm` when `rm < 16`, or 128-bit EVEX for `Xmm16..Xmm31`.
    fn vex_or_evex_gp_xmm(&mut self, map: u8, pp: u8, w: bool, reg_gp: Gp, rm: Xmm, op: u8) {
        if rm.id() >= 16 {
            self.evex_gp_xmm(map, pp, w, reg_gp, rm, op);
        } else {
            self.vex3_gp_xmm(map, pp, w, reg_gp, rm, op);
        }
    }

    /// Emits 128-bit VEX immediate shift when both XMM operands are `< 16`, or 128-bit EVEX for `Xmm16..Xmm31`.
    fn vex_or_evex_xmm_shift_imm(
        &mut self,
        evex_w: bool,
        op: u8,
        reg_ext: u8,
        dst: Xmm,
        src: Xmm,
        imm: u8,
    ) {
        if dst.id() >= 16 || src.id() >= 16 {
            self.evex_xmm_shift_imm(evex_w, op, reg_ext, dst, src, imm);
        } else {
            self.vex3_xmm_shift_imm(op, reg_ext, dst, src, imm);
        }
    }

    // ---- AVX 128-bit 3-operand FP / SIMD instructions (L=0, auto-EVEX for Xmm16..Xmm31) ----

    pub fn vmovaps_rr(&mut self, dst: Xmm, src: Xmm) {
        if dst != src {
            self.vex_or_evex_rr(1, 0, false, false, dst, None, src, 0x28);
        }
    }
    pub fn vmovss_xm(&mut self, dst: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 2, false, false, 4, dst, None, mem, 0x10);
    }
    pub fn vmovss_mx(&mut self, mem: Mem, src: Xmm) {
        self.vex_or_evex_mem(1, 2, false, false, 4, src, None, mem, 0x11);
    }
    pub fn vmovsd_xm(&mut self, dst: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 3, false, true, 8, dst, None, mem, 0x10);
    }
    pub fn vmovsd_mx(&mut self, mem: Mem, src: Xmm) {
        self.vex_or_evex_mem(1, 3, false, true, 8, src, None, mem, 0x11);
    }
    pub fn vmovss_rip(&mut self, dst: Xmm, target: RelocTarget) {
        self.vex_or_evex_rip(1, 2, false, false, dst, None, 0x10, target);
    }
    pub fn vmovsd_rip(&mut self, dst: Xmm, target: RelocTarget) {
        self.vex_or_evex_rip(1, 3, false, true, dst, None, 0x10, target);
    }
    pub fn vmovdqu_rip(&mut self, dst: Xmm, target: RelocTarget) {
        self.vex_or_evex_rip(1, 2, false, false, dst, None, 0x6f, target);
    }
    pub fn vmovd_xr(&mut self, dst: Xmm, src: Gp) {
        self.vex_or_evex_xmm_gp(1, 1, false, dst, None, src, 0x6e);
    }
    pub fn vmovq_xr(&mut self, dst: Xmm, src: Gp) {
        self.vex_or_evex_xmm_gp(1, 1, true, dst, None, src, 0x6e);
    }
    pub fn vmovd_rx(&mut self, dst: Gp, src: Xmm) {
        self.vex_or_evex_xmm_gp(1, 1, false, src, None, dst, 0x7e);
    }
    pub fn vmovq_rx(&mut self, dst: Gp, src: Xmm) {
        self.vex_or_evex_xmm_gp(1, 1, true, src, None, dst, 0x7e);
    }
    pub fn vmovdqu_xm(&mut self, dst: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 2, false, false, 16, dst, None, mem, 0x6f);
    }
    pub fn vmovdqu_mx(&mut self, mem: Mem, src: Xmm) {
        self.vex_or_evex_mem(1, 2, false, false, 16, src, None, mem, 0x7f);
    }
    /// `vpshufd xmm, xmm, imm8` (`VEX.128.66.0F.WIG 70 /r ib` or `EVEX.128.66.0F.W0 70 /r ib`).
    pub fn vpshufd(&mut self, d: Xmm, s: Xmm, imm: u8) {
        self.vex_or_evex_rr(1, 1, false, false, d, None, s, 0x70);
        self.emit_u8(imm);
    }
    pub fn vaddps_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 0, false, false, d, Some(a), b, 0x58);
    }
    pub fn vsubps_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 0, false, false, d, Some(a), b, 0x5c);
    }
    pub fn vmulps_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 0, false, false, d, Some(a), b, 0x59);
    }
    pub fn vdivps_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 0, false, false, d, Some(a), b, 0x5e);
    }
    pub fn vminps_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 0, false, false, d, Some(a), b, 0x5d);
    }
    pub fn vmaxps_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 0, false, false, d, Some(a), b, 0x5f);
    }
    pub fn vaddps_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 0, false, false, 16, d, Some(a), mem, 0x58);
    }
    pub fn vsubps_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 0, false, false, 16, d, Some(a), mem, 0x5c);
    }
    pub fn vmulps_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 0, false, false, 16, d, Some(a), mem, 0x59);
    }
    pub fn vdivps_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 0, false, false, 16, d, Some(a), mem, 0x5e);
    }
    pub fn vminps_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 0, false, false, 16, d, Some(a), mem, 0x5d);
    }
    pub fn vmaxps_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 0, false, false, 16, d, Some(a), mem, 0x5f);
    }
    pub fn vsqrtps_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(1, 0, false, false, d, None, s, 0x51);
    }
    pub fn vsqrtpd_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(1, 1, false, true, d, None, s, 0x51);
    }
    pub fn vcmpps_xxxi(&mut self, d: Xmm, a: Xmm, b: Xmm, pred: u8) {
        self.vex3_xmm_rr(1, 0, false, d, Some(a), b, 0xc2);
        self.emit_u8(pred);
    }
    pub fn vcmppd_xxxi(&mut self, d: Xmm, a: Xmm, b: Xmm, pred: u8) {
        self.vex3_xmm_rr(1, 1, false, d, Some(a), b, 0xc2);
        self.emit_u8(pred);
    }
    pub fn vandps_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 0, false, false, d, Some(a), b, 0x54);
    }
    pub fn vandnps_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 0, false, false, d, Some(a), b, 0x55);
    }
    pub fn vorps_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 0, false, false, d, Some(a), b, 0x56);
    }
    pub fn vxorps_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 0, false, false, d, Some(a), b, 0x57);
    }
    pub fn vandps_xx_rip(&mut self, d: Xmm, a: Xmm, target: RelocTarget) {
        self.vex_or_evex_rip(1, 0, false, false, d, Some(a), 0x54, target);
    }
    pub fn vxorps_xx_rip(&mut self, d: Xmm, a: Xmm, target: RelocTarget) {
        self.vex_or_evex_rip(1, 0, false, false, d, Some(a), 0x57, target);
    }
    pub fn vandpd_xx_rip(&mut self, d: Xmm, a: Xmm, target: RelocTarget) {
        self.vex_or_evex_rip(1, 1, false, true, d, Some(a), 0x54, target);
    }
    pub fn vxorpd_xx_rip(&mut self, d: Xmm, a: Xmm, target: RelocTarget) {
        self.vex_or_evex_rip(1, 1, false, true, d, Some(a), 0x57, target);
    }
    pub fn vroundps_xxi(&mut self, d: Xmm, s: Xmm, mode: u8) {
        self.vex3_xmm_rr(3, 1, false, d, None, s, 0x08);
        self.emit_u8(mode);
    }
    pub fn vroundpd_xxi(&mut self, d: Xmm, s: Xmm, mode: u8) {
        self.vex3_xmm_rr(3, 1, false, d, None, s, 0x09);
        self.emit_u8(mode);
    }
    pub fn vcvtdq2ps_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(1, 0, false, false, d, None, s, 0x5b);
    }
    pub fn vcvttps2dq_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(1, 2, false, false, d, None, s, 0x5b);
    }
    pub fn vcvtdq2pd_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(1, 2, false, false, d, None, s, 0xe6);
    }
    pub fn vcvttpd2dq_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(1, 1, false, true, d, None, s, 0xe6);
    }
    pub fn vcvtps2pd_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(1, 0, false, false, d, None, s, 0x5a);
    }
    pub fn vcvtpd2ps_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(1, 1, false, true, d, None, s, 0x5a);
    }
    pub fn vaddpd_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 1, false, true, 16, d, Some(a), mem, 0x58);
    }
    pub fn vsubpd_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 1, false, true, 16, d, Some(a), mem, 0x5c);
    }
    pub fn vmulpd_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 1, false, true, 16, d, Some(a), mem, 0x59);
    }
    pub fn vdivpd_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 1, false, true, 16, d, Some(a), mem, 0x5e);
    }
    pub fn vminpd_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 1, false, true, 16, d, Some(a), mem, 0x5d);
    }
    pub fn vmaxpd_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 1, false, true, 16, d, Some(a), mem, 0x5f);
    }
    // VEX/EVEX 128-bit integer arithmetic / bitwise / cmp / shuffle / broadcast / shift
    pub fn vpand_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xdb);
    }
    pub fn vpandn_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xdf);
    }
    pub fn vpor_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xeb);
    }
    pub fn vpxor_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xef);
    }
    pub fn vpand_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 1, false, false, 16, d, Some(a), mem, 0xdb);
    }
    pub fn vpandn_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 1, false, false, 16, d, Some(a), mem, 0xdf);
    }
    pub fn vpor_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 1, false, false, 16, d, Some(a), mem, 0xeb);
    }
    pub fn vpxor_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 1, false, false, 16, d, Some(a), mem, 0xef);
    }
    pub fn vpaddb_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xfc);
    }
    pub fn vpaddw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xfd);
    }
    pub fn vpaddd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xfe);
    }
    pub fn vpaddq_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, true, d, Some(a), b, 0xd4);
    }
    pub fn vpaddb_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 1, false, false, 16, d, Some(a), mem, 0xfc);
    }
    pub fn vpaddw_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 1, false, false, 16, d, Some(a), mem, 0xfd);
    }
    pub fn vpaddd_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 1, false, false, 16, d, Some(a), mem, 0xfe);
    }
    pub fn vpaddq_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 1, false, true, 16, d, Some(a), mem, 0xd4);
    }
    pub fn vpsubb_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xf8);
    }
    pub fn vpsubw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xf9);
    }
    pub fn vpsubd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xfa);
    }
    pub fn vpsubq_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, true, d, Some(a), b, 0xfb);
    }
    pub fn vpsubb_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 1, false, false, 16, d, Some(a), mem, 0xf8);
    }
    pub fn vpsubw_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 1, false, false, 16, d, Some(a), mem, 0xf9);
    }
    pub fn vpsubd_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 1, false, false, 16, d, Some(a), mem, 0xfa);
    }
    pub fn vpsubq_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 1, false, true, 16, d, Some(a), mem, 0xfb);
    }
    pub fn vpaddsb_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xec);
    }
    pub fn vpaddsw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xed);
    }
    pub fn vpaddusb_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xdc);
    }
    pub fn vpaddusw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xdd);
    }
    pub fn vpsubsb_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xe8);
    }
    pub fn vpsubsw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xe9);
    }
    pub fn vpsubusb_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xd8);
    }
    pub fn vpsubusw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xd9);
    }
    pub fn vpmullw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xd5);
    }
    pub fn vpmullw_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 1, false, false, 16, d, Some(a), mem, 0xd5);
    }
    pub fn vpmulld_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, Some(a), b, 0x40);
    }
    pub fn vpmulld_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(2, 1, false, false, 16, d, Some(a), mem, 0x40);
    }
    pub fn vpmuldq_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(2, 1, false, true, d, Some(a), b, 0x28);
    }
    pub fn vpmuludq_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, true, d, Some(a), b, 0xf4);
    }
    pub fn vpmulhrsw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, Some(a), b, 0x0b);
    }
    pub fn vpmaddwd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xf5);
    }
    pub fn vpmaddubsw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, Some(a), b, 0x04);
    }
    pub fn vpsadbw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xf6);
    }
    pub fn vpminsb_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, Some(a), b, 0x38);
    }
    pub fn vpminub_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xda);
    }
    pub fn vpminsw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xea);
    }
    pub fn vpminuw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, Some(a), b, 0x3a);
    }
    pub fn vpminsd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, Some(a), b, 0x39);
    }
    pub fn vpminud_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, Some(a), b, 0x3b);
    }
    pub fn vpmaxsb_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, Some(a), b, 0x3c);
    }
    pub fn vpmaxub_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xde);
    }
    pub fn vpmaxsw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xee);
    }
    pub fn vpmaxuw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, Some(a), b, 0x3e);
    }
    pub fn vpmaxsd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, Some(a), b, 0x3d);
    }
    pub fn vpmaxud_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, Some(a), b, 0x3f);
    }
    pub fn vpavgb_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xe0);
    }
    pub fn vpavgw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0xe3);
    }
    pub fn vpabsb_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, None, s, 0x1c);
    }
    pub fn vpabsw_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, None, s, 0x1d);
    }
    pub fn vpabsd_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, None, s, 0x1e);
    }
    pub fn vpcmpeqb_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_xmm_rr(1, 1, false, d, Some(a), b, 0x74);
    }
    pub fn vpcmpeqw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_xmm_rr(1, 1, false, d, Some(a), b, 0x75);
    }
    pub fn vpcmpeqd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_xmm_rr(1, 1, false, d, Some(a), b, 0x76);
    }
    pub fn vpcmpeqq_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_xmm_rr(2, 1, false, d, Some(a), b, 0x29);
    }
    pub fn vpcmpgtb_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_xmm_rr(1, 1, false, d, Some(a), b, 0x64);
    }
    pub fn vpcmpgtw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_xmm_rr(1, 1, false, d, Some(a), b, 0x65);
    }
    pub fn vpcmpgtd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_xmm_rr(1, 1, false, d, Some(a), b, 0x66);
    }
    pub fn vpcmpgtq_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex3_xmm_rr(2, 1, false, d, Some(a), b, 0x37);
    }
    pub fn vpacksswb_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0x63);
    }
    pub fn vpackuswb_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0x67);
    }
    pub fn vpackssdw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0x6b);
    }
    pub fn vpackusdw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, Some(a), b, 0x2b);
    }
    pub fn vpmovsxbw_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, None, s, 0x20);
    }
    pub fn vpmovzxbw_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, None, s, 0x30);
    }
    pub fn vpmovzxbd_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, None, s, 0x31);
    }
    pub fn vpmovsxwd_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, None, s, 0x23);
    }
    pub fn vpmovzxwd_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, None, s, 0x33);
    }
    pub fn vpmovsxdq_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, None, s, 0x25);
    }
    pub fn vpmovzxdq_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, None, s, 0x35);
    }
    pub fn vpmovsxbw_xm(&mut self, d: Xmm, mem: Mem) {
        self.vex_or_evex_mem(2, 1, false, false, 8, d, None, mem, 0x20);
    }
    pub fn vpmovzxbw_xm(&mut self, d: Xmm, mem: Mem) {
        self.vex_or_evex_mem(2, 1, false, false, 8, d, None, mem, 0x30);
    }
    pub fn vpmovsxwd_xm(&mut self, d: Xmm, mem: Mem) {
        self.vex_or_evex_mem(2, 1, false, false, 8, d, None, mem, 0x23);
    }
    pub fn vpmovzxwd_xm(&mut self, d: Xmm, mem: Mem) {
        self.vex_or_evex_mem(2, 1, false, false, 8, d, None, mem, 0x33);
    }
    pub fn vpmovsxdq_xm(&mut self, d: Xmm, mem: Mem) {
        self.vex_or_evex_mem(2, 1, false, false, 8, d, None, mem, 0x25);
    }
    pub fn vpmovzxdq_xm(&mut self, d: Xmm, mem: Mem) {
        self.vex_or_evex_mem(2, 1, false, false, 8, d, None, mem, 0x35);
    }
    pub fn vunpcklps_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 0, false, false, d, Some(a), b, 0x14);
    }
    pub fn vunpckhps_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 0, false, false, d, Some(a), b, 0x15);
    }
    pub fn vunpcklpd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, true, d, Some(a), b, 0x14);
    }
    pub fn vunpckhpd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, true, d, Some(a), b, 0x15);
    }
    pub fn vshufps_xxxi(&mut self, d: Xmm, a: Xmm, b: Xmm, imm: u8) {
        self.vex_or_evex_rr(1, 0, false, false, d, Some(a), b, 0xc6);
        self.emit_u8(imm);
    }
    pub fn vshufpd_xxxi(&mut self, d: Xmm, a: Xmm, b: Xmm, imm: u8) {
        self.vex_or_evex_rr(1, 1, false, true, d, Some(a), b, 0xc6);
        self.emit_u8(imm);
    }
    pub fn vpunpcklbw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0x60);
    }
    pub fn vpunpckhbw_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0x68);
    }
    pub fn vpunpcklwd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0x61);
    }
    pub fn vpunpckhwd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0x69);
    }
    pub fn vpunpckldq_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0x62);
    }
    pub fn vpunpckhdq_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), b, 0x6a);
    }
    pub fn vpunpcklqdq_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, true, d, Some(a), b, 0x6c);
    }
    pub fn vpunpckhqdq_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, true, d, Some(a), b, 0x6d);
    }
    pub fn vpalignr_xxxi(&mut self, d: Xmm, hi: Xmm, lo: Xmm, imm: u8) {
        self.vex_or_evex_rr(3, 1, false, false, d, Some(hi), lo, 0x0f);
        self.emit_u8(imm);
    }
    pub fn vpshufb_xxx(&mut self, d: Xmm, a: Xmm, mask: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, Some(a), mask, 0x00);
    }
    pub fn vpshufb_xx_rip(&mut self, d: Xmm, a: Xmm, target: RelocTarget) {
        self.vex_or_evex_rip(2, 1, false, false, d, Some(a), 0x00, target);
    }
    pub fn vpaddusb_xx_rip(&mut self, d: Xmm, a: Xmm, target: RelocTarget) {
        self.vex_or_evex_rip(1, 1, false, false, d, Some(a), 0xdc, target);
    }
    pub fn vpand_xx_rip(&mut self, d: Xmm, a: Xmm, target: RelocTarget) {
        self.vex_or_evex_rip(1, 1, false, false, d, Some(a), 0xdb, target);
    }
    pub fn vmulps_xx_rip(&mut self, d: Xmm, a: Xmm, target: RelocTarget) {
        self.vex_or_evex_rip(1, 0, false, false, d, Some(a), 0x59, target);
    }
    pub fn vbroadcastss_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, None, s, 0x18);
    }
    pub fn vbroadcastss_xm(&mut self, d: Xmm, mem: Mem) {
        self.vex_or_evex_mem(2, 1, false, false, 4, d, None, mem, 0x18);
    }
    pub fn vpbroadcastb_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, None, s, 0x78);
    }
    pub fn vpbroadcastb_xm(&mut self, d: Xmm, mem: Mem) {
        self.vex_or_evex_mem(2, 1, false, false, 1, d, None, mem, 0x78);
    }
    pub fn vpbroadcastw_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, None, s, 0x79);
    }
    pub fn vpbroadcastw_xm(&mut self, d: Xmm, mem: Mem) {
        self.vex_or_evex_mem(2, 1, false, false, 2, d, None, mem, 0x79);
    }
    pub fn vpbroadcastd_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(2, 1, false, false, d, None, s, 0x58);
    }
    pub fn vpbroadcastd_xm(&mut self, d: Xmm, mem: Mem) {
        self.vex_or_evex_mem(2, 1, false, false, 4, d, None, mem, 0x58);
    }
    pub fn vpbroadcastq_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(2, 1, false, true, d, None, s, 0x59);
    }
    pub fn vmovddup_xx(&mut self, d: Xmm, s: Xmm) {
        self.vex_or_evex_rr(1, 3, false, true, d, None, s, 0x12);
    }
    pub fn vmovddup_xm(&mut self, d: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 3, false, true, 8, d, None, mem, 0x12);
    }
    pub fn vpsllw_xxx(&mut self, d: Xmm, a: Xmm, cnt: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), cnt, 0xf1);
    }
    pub fn vpsrlw_xxx(&mut self, d: Xmm, a: Xmm, cnt: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), cnt, 0xd1);
    }
    pub fn vpsraw_xxx(&mut self, d: Xmm, a: Xmm, cnt: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), cnt, 0xe1);
    }
    pub fn vpslld_xxx(&mut self, d: Xmm, a: Xmm, cnt: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), cnt, 0xf2);
    }
    pub fn vpsrld_xxx(&mut self, d: Xmm, a: Xmm, cnt: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), cnt, 0xd2);
    }
    pub fn vpsrad_xxx(&mut self, d: Xmm, a: Xmm, cnt: Xmm) {
        self.vex_or_evex_rr(1, 1, false, false, d, Some(a), cnt, 0xe2);
    }
    pub fn vpsllq_xxx(&mut self, d: Xmm, a: Xmm, cnt: Xmm) {
        self.vex_or_evex_rr(1, 1, false, true, d, Some(a), cnt, 0xf3);
    }
    pub fn vpsrlq_xxx(&mut self, d: Xmm, a: Xmm, cnt: Xmm) {
        self.vex_or_evex_rr(1, 1, false, true, d, Some(a), cnt, 0xd3);
    }
    pub fn vpsllw_xxi(&mut self, d: Xmm, s: Xmm, imm: u8) {
        self.vex_or_evex_xmm_shift_imm(false, 0x71, 6, d, s, imm);
    }
    pub fn vpsrlw_xxi(&mut self, d: Xmm, s: Xmm, imm: u8) {
        self.vex_or_evex_xmm_shift_imm(false, 0x71, 2, d, s, imm);
    }
    pub fn vpsraw_xxi(&mut self, d: Xmm, s: Xmm, imm: u8) {
        self.vex_or_evex_xmm_shift_imm(false, 0x71, 4, d, s, imm);
    }
    pub fn vpslld_xxi(&mut self, d: Xmm, s: Xmm, imm: u8) {
        self.vex_or_evex_xmm_shift_imm(false, 0x72, 6, d, s, imm);
    }
    pub fn vpsrld_xxi(&mut self, d: Xmm, s: Xmm, imm: u8) {
        self.vex_or_evex_xmm_shift_imm(false, 0x72, 2, d, s, imm);
    }
    pub fn vpsrad_xxi(&mut self, d: Xmm, s: Xmm, imm: u8) {
        self.vex_or_evex_xmm_shift_imm(false, 0x72, 4, d, s, imm);
    }
    pub fn vpsllq_xxi(&mut self, d: Xmm, s: Xmm, imm: u8) {
        self.vex_or_evex_xmm_shift_imm(true, 0x73, 6, d, s, imm);
    }
    pub fn vpsrlq_xxi(&mut self, d: Xmm, s: Xmm, imm: u8) {
        self.vex_or_evex_xmm_shift_imm(true, 0x73, 2, d, s, imm);
    }
    pub fn vpsrldq_xxi(&mut self, d: Xmm, s: Xmm, imm_bytes: u8) {
        self.vex_or_evex_xmm_shift_imm(false, 0x73, 3, d, s, imm_bytes);
    }
    pub fn vptest(&mut self, a: Xmm, b: Xmm) {
        self.vex3_xmm_rr(2, 1, false, a, None, b, 0x17);
    }
    pub fn vpmovmskb(&mut self, dst: Gp, src: Xmm) {
        self.vex3_gp_xmm(1, 1, false, dst, src, 0xd7);
    }
    pub fn vmovmskps(&mut self, dst: Gp, src: Xmm) {
        self.vex3_gp_xmm(1, 0, false, dst, src, 0x50);
    }
    pub fn vmovmskpd(&mut self, dst: Gp, src: Xmm) {
        self.vex3_gp_xmm(1, 1, false, dst, src, 0x50);
    }
    pub fn vpinsrb(&mut self, d: Xmm, v: Xmm, src: Gp, lane: u8) {
        self.vex_or_evex_xmm_gp(3, 1, false, d, Some(v), src, 0x20);
        self.emit_u8(lane & 15);
    }
    pub fn vpinsrw(&mut self, d: Xmm, v: Xmm, src: Gp, lane: u8) {
        self.vex_or_evex_xmm_gp(1, 1, false, d, Some(v), src, 0xc4);
        self.emit_u8(lane & 7);
    }
    pub fn vpinsrd(&mut self, d: Xmm, v: Xmm, src: Gp, lane: u8) {
        self.vex_or_evex_xmm_gp(3, 1, false, d, Some(v), src, 0x22);
        self.emit_u8(lane & 3);
    }
    pub fn vpinsrq(&mut self, d: Xmm, v: Xmm, src: Gp, lane: u8) {
        self.vex_or_evex_xmm_gp(3, 1, true, d, Some(v), src, 0x22);
        self.emit_u8(lane & 1);
    }
    pub fn vpextrb(&mut self, dst: Gp, src: Xmm, lane: u8) {
        self.vex_or_evex_xmm_gp(3, 1, false, src, None, dst, 0x14);
        self.emit_u8(lane & 15);
    }
    pub fn vpextrw(&mut self, dst: Gp, src: Xmm, lane: u8) {
        self.vex_or_evex_gp_xmm(1, 1, false, dst, src, 0xc5);
        self.emit_u8(lane & 7);
    }
    pub fn vpextrd(&mut self, dst: Gp, src: Xmm, lane: u8) {
        self.vex_or_evex_xmm_gp(3, 1, false, src, None, dst, 0x16);
        self.emit_u8(lane & 3);
    }
    pub fn vpextrq(&mut self, dst: Gp, src: Xmm, lane: u8) {
        self.vex_or_evex_xmm_gp(3, 1, true, src, None, dst, 0x16);
        self.emit_u8(lane & 1);
    }
    pub fn vaddss_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 2, false, false, d, Some(a), b, 0x58);
    }
    pub fn vsubss_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 2, false, false, d, Some(a), b, 0x5c);
    }
    pub fn vmulss_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 2, false, false, d, Some(a), b, 0x59);
    }
    pub fn vdivss_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 2, false, false, d, Some(a), b, 0x5e);
    }
    pub fn vaddss_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 2, false, false, 4, d, Some(a), mem, 0x58);
    }
    pub fn vsubss_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 2, false, false, 4, d, Some(a), mem, 0x5c);
    }
    pub fn vmulss_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 2, false, false, 4, d, Some(a), mem, 0x59);
    }
    pub fn vdivss_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 2, false, false, 4, d, Some(a), mem, 0x5e);
    }
    pub fn vaddsd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 3, false, true, d, Some(a), b, 0x58);
    }
    pub fn vsubsd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 3, false, true, d, Some(a), b, 0x5c);
    }
    pub fn vmulsd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 3, false, true, d, Some(a), b, 0x59);
    }
    pub fn vdivsd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 3, false, true, d, Some(a), b, 0x5e);
    }
    pub fn vaddsd_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 3, false, true, 8, d, Some(a), mem, 0x58);
    }
    pub fn vsubsd_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 3, false, true, 8, d, Some(a), mem, 0x5c);
    }
    pub fn vmulsd_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 3, false, true, 8, d, Some(a), mem, 0x59);
    }
    pub fn vdivsd_xxm(&mut self, d: Xmm, a: Xmm, mem: Mem) {
        self.vex_or_evex_mem(1, 3, false, true, 8, d, Some(a), mem, 0x5e);
    }
    pub fn vaddpd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, true, d, Some(a), b, 0x58);
    }
    pub fn vsubpd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, true, d, Some(a), b, 0x5c);
    }
    pub fn vmulpd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, true, d, Some(a), b, 0x59);
    }
    pub fn vdivpd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, true, d, Some(a), b, 0x5e);
    }
    pub fn vminpd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, true, d, Some(a), b, 0x5d);
    }
    pub fn vmaxpd_xxx(&mut self, d: Xmm, a: Xmm, b: Xmm) {
        self.vex_or_evex_rr(1, 1, false, true, d, Some(a), b, 0x5f);
    }

    // ---- AVX2 256-bit YMM byte-stencil & f64x4 instructions ----

    /// `vpbroadcastw ymm, xmm` (`VEX.256.66.0F38.W0 79 /r`).
    pub fn vpbroadcastw_yx(&mut self, dst: Xmm, src: Xmm) {
        self.vex3_ymm_rr(2, 1, false, dst, None, src, 0x79);
    }
    /// `vpmovzxbw ymm, m128` (`VEX.256.66.0F38.WIG 30 /r`).
    pub fn vpmovzxbw_ym(&mut self, dst: Xmm, mem: Mem) {
        self.vex3_ymm_mem(2, 1, false, dst, None, mem, 0x30);
    }
    /// `vpaddw ymm, ymm, ymm` (`VEX.256.66.0F.WIG FD /r`).
    pub fn vpaddw_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 1, false, dst, Some(a), b, 0xfd);
    }
    /// `vpmulhuw ymm, ymm, ymm` (`VEX.256.66.0F.WIG E4 /r`).
    pub fn vpmulhuw_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 1, false, dst, Some(a), b, 0xe4);
    }
    /// `vpsrlw ymm, ymm, imm8` (`VEX.256.66.0F.WIG 71 /2 ib`).
    pub fn vpsrlw_yyi(&mut self, dst: Xmm, src: Xmm, imm: u8) {
        self.vex3_ymm_shift_imm(0x71, 2, dst, src, imm);
    }
    /// `vpackuswb ymm, ymm, ymm` (`VEX.256.66.0F.WIG 67 /r`).
    pub fn vpackuswb_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 1, false, dst, Some(a), b, 0x67);
    }
    /// `vpermq ymm, ymm, imm8` (`VEX.256.66.0F3A.W1 00 /r ib`).
    pub fn vpermq_yyi(&mut self, dst: Xmm, src: Xmm, imm: u8) {
        self.vex3_ymm_rr(3, 1, true, dst, None, src, 0x00);
        self.emit_u8(imm);
    }
    /// `vbroadcastsd ymm, xmm` (`VEX.256.66.0F38.W0 19 /r`).
    pub fn vbroadcastsd_yx(&mut self, dst: Xmm, src: Xmm) {
        self.vex3_ymm_rr(2, 1, false, dst, None, src, 0x19);
    }
    /// `vmovupd ymm, m256` (`VEX.256.66.0F.WIG 10 /r`).
    pub fn vmovupd_ym(&mut self, dst: Xmm, mem: Mem) {
        self.vex3_ymm_mem(1, 1, false, dst, None, mem, 0x10);
    }
    /// `vmovupd m256, ymm` (`VEX.256.66.0F.WIG 11 /r`).
    pub fn vmovupd_my(&mut self, mem: Mem, src: Xmm) {
        self.vex3_ymm_mem(1, 1, false, src, None, mem, 0x11);
    }
    pub fn vaddpd_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(1, 1, false, dst, Some(a), mem, 0x58);
    }
    pub fn vsubpd_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(1, 1, false, dst, Some(a), mem, 0x5c);
    }
    pub fn vmulpd_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(1, 1, false, dst, Some(a), mem, 0x59);
    }
    pub fn vdivpd_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(1, 1, false, dst, Some(a), mem, 0x5e);
    }
    pub fn vaddpd_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 1, false, dst, Some(a), b, 0x58);
    }
    pub fn vsubpd_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 1, false, dst, Some(a), b, 0x5c);
    }
    pub fn vmulpd_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 1, false, dst, Some(a), b, 0x59);
    }
    pub fn vdivpd_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 1, false, dst, Some(a), b, 0x5e);
    }

    /// `vzeroupper` (`C5 F8 77`): clears upper 128 bits of all YMM0..YMM15, eliminating AVX-SSE stalls.
    pub fn vzeroupper(&mut self) {
        self.emit_u8(0xc5);
        self.emit_u8(0xf8);
        self.emit_u8(0x77);
    }

    // ---- Additional AVX2 256-bit YMM vector encoders (`x64-v3`) ----

    pub fn vbroadcastss_yx(&mut self, dst: Xmm, src: Xmm) {
        self.vex3_ymm_rr(2, 1, false, dst, None, src, 0x18);
    }
    pub fn vpbroadcastd_yx(&mut self, dst: Xmm, src: Xmm) {
        self.vex3_ymm_rr(2, 1, false, dst, None, src, 0x58);
    }
    pub fn vpbroadcastq_yx(&mut self, dst: Xmm, src: Xmm) {
        self.vex3_ymm_rr(2, 1, false, dst, None, src, 0x59);
    }
    pub fn vmovups_ym(&mut self, dst: Xmm, mem: Mem) {
        self.vex3_ymm_mem(1, 0, false, dst, None, mem, 0x10);
    }
    pub fn vmovups_my(&mut self, mem: Mem, src: Xmm) {
        self.vex3_ymm_mem(1, 0, false, src, None, mem, 0x11);
    }
    pub fn vaddps_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 0, false, dst, Some(a), b, 0x58);
    }
    pub fn vsubps_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 0, false, dst, Some(a), b, 0x5c);
    }
    pub fn vmulps_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 0, false, dst, Some(a), b, 0x59);
    }
    pub fn vdivps_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 0, false, dst, Some(a), b, 0x5e);
    }
    pub fn vmovaps_yy(&mut self, dst: Xmm, src: Xmm) {
        if dst != src {
            self.vex3_ymm_rr(1, 0, false, dst, None, src, 0x28);
        }
    }
    pub fn vsqrtps_yy(&mut self, dst: Xmm, src: Xmm) {
        self.vex3_ymm_rr(1, 0, false, dst, None, src, 0x51);
    }
    pub fn vcvtdq2ps_yy(&mut self, dst: Xmm, src: Xmm) {
        self.vex3_ymm_rr(1, 0, false, dst, None, src, 0x5b);
    }
    pub fn vcmpps_yyyi(&mut self, dst: Xmm, a: Xmm, b: Xmm, pred: u8) {
        self.vex3_ymm_rr(1, 0, false, dst, Some(a), b, 0xc2);
        self.emit_u8(pred);
    }
    pub fn vpcmpgtd_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 1, false, dst, Some(a), b, 0x66);
    }
    pub fn vptest_yy(&mut self, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(2, 1, false, a, None, b, 0x17);
    }
    pub fn vaddps_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(1, 0, false, dst, Some(a), mem, 0x58);
    }
    pub fn vsubps_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(1, 0, false, dst, Some(a), mem, 0x5c);
    }
    pub fn vmulps_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(1, 0, false, dst, Some(a), mem, 0x59);
    }
    pub fn vdivps_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(1, 0, false, dst, Some(a), mem, 0x5e);
    }
    pub fn vpaddd_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 1, false, dst, Some(a), b, 0xfe);
    }
    pub fn vpsubd_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 1, false, dst, Some(a), b, 0xfa);
    }
    pub fn vpmulld_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(2, 1, false, dst, Some(a), b, 0x40);
    }
    pub fn vpand_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 1, false, dst, Some(a), b, 0xdb);
    }
    pub fn vpor_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 1, false, dst, Some(a), b, 0xeb);
    }
    pub fn vpsubq_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 1, false, dst, Some(a), b, 0xfb);
    }
    pub fn vpaddd_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(1, 1, false, dst, Some(a), mem, 0xfe);
    }
    pub fn vpsubd_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(1, 1, false, dst, Some(a), mem, 0xfa);
    }
    pub fn vpmulld_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(2, 1, false, dst, Some(a), mem, 0x40);
    }
    pub fn vpand_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(1, 1, false, dst, Some(a), mem, 0xdb);
    }
    pub fn vpor_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(1, 1, false, dst, Some(a), mem, 0xeb);
    }
    pub fn vpxor_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(1, 1, false, dst, Some(a), mem, 0xef);
    }
    pub fn vpaddq_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(1, 1, false, dst, Some(a), mem, 0xd4);
    }
    pub fn vpsubq_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(1, 1, false, dst, Some(a), mem, 0xfb);
    }
    /// `vfmadd231ps ymm, ymm, ymm` (`VEX.256.66.0F38.W0 B8 /r`).
    pub fn vfmadd231ps_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(2, 1, false, dst, Some(a), b, 0xb8);
    }
    /// `vfmadd231ps ymm, ymm, m256` (`VEX.256.66.0F38.W0 B8 /r`).
    pub fn vfmadd231ps_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(2, 1, false, dst, Some(a), mem, 0xb8);
    }
    /// `vfnmadd231ps ymm, ymm, ymm` (`VEX.256.66.0F38.W0 BC /r`).
    pub fn vfnmadd231ps_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(2, 1, false, dst, Some(a), b, 0xbc);
    }
    /// `vfnmadd231ps ymm, ymm, m256` (`VEX.256.66.0F38.W0 BC /r`).
    pub fn vfnmadd231ps_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(2, 1, false, dst, Some(a), mem, 0xbc);
    }
    /// `vfmadd231pd ymm, ymm, ymm` (`VEX.256.66.0F38.W1 B8 /r`).
    pub fn vfmadd231pd_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(2, 1, true, dst, Some(a), b, 0xb8);
    }
    /// `vfmadd231pd ymm, ymm, m256` (`VEX.256.66.0F38.W1 B8 /r`).
    pub fn vfmadd231pd_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(2, 1, true, dst, Some(a), mem, 0xb8);
    }
    /// `vfnmadd231pd ymm, ymm, ymm` (`VEX.256.66.0F38.W1 BC /r`).
    pub fn vfnmadd231pd_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(2, 1, true, dst, Some(a), b, 0xbc);
    }
    /// `vfnmadd231pd ymm, ymm, m256` (`VEX.256.66.0F38.W1 BC /r`).
    pub fn vfnmadd231pd_yym(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.vex3_ymm_mem(2, 1, true, dst, Some(a), mem, 0xbc);
    }
    /// `vinsertf128 ymm, ymm, xmm, imm8` (`VEX.256.66.0F3A.W0 18 /r ib`).
    pub fn vinsertf128_yyx(&mut self, dst: Xmm, a: Xmm, b: Xmm, imm: u8) {
        self.vex3_ymm_rr(3, 1, false, dst, Some(a), b, 0x18);
        self.emit_u8(imm & 1);
    }
    /// `vcmpeqps ymm, ymm, ymm` (`VEX.256.0F.WIG C2 /r 00`).
    pub fn vcmpeqps_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 0, false, dst, Some(a), b, 0xc2);
        self.emit_u8(0);
    }
    /// `vcmpeqpd ymm, ymm, ymm` (`VEX.256.66.0F.WIG C2 /r 00`).
    pub fn vcmpeqpd_yyy(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.vex3_ymm_rr(1, 1, false, dst, Some(a), b, 0xc2);
        self.emit_u8(0);
    }
    /// `vblendvps ymm, ymm, ymm, ymm` (`VEX.256.66.0F3A.W0 4A /r /is4`).
    pub fn vblendvps_yyyy(&mut self, dst: Xmm, a: Xmm, b: Xmm, mask: Xmm) {
        assert!(mask.id() < 16);
        self.vex3_ymm_rr(3, 1, false, dst, Some(a), b, 0x4a);
        self.emit_u8(mask.id() << 4);
    }
    /// `vblendvpd ymm, ymm, ymm, ymm` (`VEX.256.66.0F3A.W0 4B /r /is4`).
    pub fn vblendvpd_yyyy(&mut self, dst: Xmm, a: Xmm, b: Xmm, mask: Xmm) {
        assert!(mask.id() < 16);
        self.vex3_ymm_rr(3, 1, false, dst, Some(a), b, 0x4b);
        self.emit_u8(mask.id() << 4);
    }
    /// `vblendvps xmm, xmm, xmm, xmm` (`VEX.128.66.0F3A.W0 4A /r /is4`).
    pub fn vblendvps_xxxx(&mut self, dst: Xmm, a: Xmm, b: Xmm, mask: Xmm) {
        assert!(mask.id() < 16);
        self.vex3_xmm_rr(3, 1, false, dst, Some(a), b, 0x4a);
        self.emit_u8(mask.id() << 4);
    }
    /// `vblendvpd xmm, xmm, xmm, xmm` (`VEX.128.66.0F3A.W0 4B /r /is4`).
    pub fn vblendvpd_xxxx(&mut self, dst: Xmm, a: Xmm, b: Xmm, mask: Xmm) {
        assert!(mask.id() < 16);
        self.vex3_xmm_rr(3, 1, false, dst, Some(a), b, 0x4b);
        self.emit_u8(mask.id() << 4);
    }

    // ---- AVX-512 (EVEX 512-bit ZMM + 128-bit VL `vpternlogd`) Encoders (`x64-v4`) ----

    /// ModRM + SIB + EVEX `disp8*N` compressed displacement encoder.
    fn modrm_mem_evex(&mut self, reg_lo: u8, mem: Mem, disp8_scale: i32) {
        let reg_bits = (reg_lo & 7) << 3;
        let pick_disp = |disp: i32, blo: u8| -> (u8, u8, u8) {
            if disp == 0 && blo != 5 {
                (0x00, 0, 0)
            } else if disp8_scale > 0
                && disp % disp8_scale == 0
                && let Ok(d8) = i8::try_from(disp / disp8_scale)
            {
                (0x40, 1, d8 as u8)
            } else {
                (0x80, 4, 0)
            }
        };
        match mem {
            Mem::BaseDisp { base, disp } => {
                let blo = base.lo();
                let need_sib = blo == 4;
                let (m, dlen, d8) = pick_disp(disp, blo);
                self.code.push(m | reg_bits | blo);
                if need_sib {
                    self.code.push(0x24);
                }
                match dlen {
                    1 => self.code.push(d8),
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
                let (m, dlen, d8) = pick_disp(disp, blo);
                self.code.push(m | reg_bits | 4);
                self.code.push((sbits << 6) | (ilo << 3) | blo);
                match dlen {
                    1 => self.code.push(d8),
                    4 => self.emit_i32(disp),
                    _ => {}
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn evex_rr_mask(
        &mut self,
        map: u8,
        pp: u8,
        w: bool,
        ll: u8,
        k_mask: u8,
        reg: Xmm,
        vvvv: Option<Xmm>,
        rm: Xmm,
        op: u8,
    ) {
        let r_inv = u8::from(reg.hi() == 0);
        let x_inv = u8::from(rm.hi4() == 0); // EVEX.X = ~rm[4] for reg-reg
        let b_inv = u8::from(rm.hi() == 0);
        let rp_inv = u8::from(reg.hi4() == 0); // EVEX.R' = ~reg[4]
        let v = vvvv.map_or(0x0f, |x| (!x.id()) & 0x0f);
        let vp_inv = vvvv.map_or(1u8, |x| u8::from(x.hi4() == 0)); // EVEX.V' = ~vvvv[4]
        self.emit_u8(0x62);
        self.emit_u8((r_inv << 7) | (x_inv << 6) | (b_inv << 5) | (rp_inv << 4) | (map & 0x03));
        self.emit_u8((u8::from(w) << 7) | (v << 3) | (1 << 2) | (pp & 0x03));
        self.emit_u8(((ll & 0x03) << 5) | (vp_inv << 3) | (k_mask & 0x07));
        self.emit_u8(op);
        self.modrm_reg(reg.lo(), rm.lo());
    }

    #[allow(clippy::too_many_arguments)]
    fn evex_rr(
        &mut self,
        map: u8,
        pp: u8,
        w: bool,
        ll: u8,
        reg: Xmm,
        vvvv: Option<Xmm>,
        rm: Xmm,
        op: u8,
    ) {
        self.evex_rr_mask(map, pp, w, ll, 0, reg, vvvv, rm, op);
    }

    #[allow(clippy::too_many_arguments)]
    fn evex_mem(
        &mut self,
        map: u8,
        pp: u8,
        w: bool,
        ll: u8,
        disp8_scale: i32,
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
        let rp_inv = u8::from(reg.hi4() == 0);
        let v = vvvv.map_or(0x0f, |x| (!x.id()) & 0x0f);
        let vp_inv = vvvv.map_or(1u8, |x| u8::from(x.hi4() == 0));
        self.emit_u8(0x62);
        self.emit_u8((r_inv << 7) | (x_inv << 6) | (b_inv << 5) | (rp_inv << 4) | (map & 0x03));
        self.emit_u8((u8::from(w) << 7) | (v << 3) | (1 << 2) | (pp & 0x03));
        self.emit_u8(((ll & 0x03) << 5) | (vp_inv << 3));
        self.emit_u8(op);
        self.modrm_mem_evex(reg.lo(), mem, disp8_scale);
    }

    /// Emits a 4-byte EVEX RIP-relative memory instruction (full 5-bit `reg` / `vvvv`, raw `disp32`).
    #[allow(clippy::too_many_arguments)]
    fn evex_rip(
        &mut self,
        map: u8,
        pp: u8,
        w: bool,
        ll: u8,
        reg: Xmm,
        vvvv: Option<Xmm>,
        op: u8,
        target: RelocTarget,
    ) {
        let r_inv = u8::from(reg.hi() == 0);
        let rp_inv = u8::from(reg.hi4() == 0);
        let v = vvvv.map_or(0x0f, |x| (!x.id()) & 0x0f);
        let vp_inv = vvvv.map_or(1u8, |x| u8::from(x.hi4() == 0));
        self.emit_u8(0x62);
        self.emit_u8((r_inv << 7) | (1 << 6) | (1 << 5) | (rp_inv << 4) | (map & 0x03));
        self.emit_u8((u8::from(w) << 7) | (v << 3) | (1 << 2) | (pp & 0x03));
        self.emit_u8(((ll & 0x03) << 5) | (vp_inv << 3));
        self.emit_u8(op);
        self.emit_u8((reg.lo() << 3) | 5);
        let patch_off = self.pos();
        self.emit_i32(0);
        let next_ip_off = self.pos();
        self.relocs.push(Reloc {
            patch_off,
            next_ip_off,
            target,
        });
    }

    /// Emits a 4-byte EVEX 128-bit instruction with `reg = Xmm`, optional `vvvv = Xmm`, `rm = Gp`.
    #[allow(clippy::too_many_arguments)]
    fn evex_xmm_gp(
        &mut self,
        map: u8,
        pp: u8,
        w: bool,
        reg: Xmm,
        vvvv: Option<Xmm>,
        rm_gp: Gp,
        op: u8,
    ) {
        let r_inv = u8::from(reg.hi() == 0);
        let b_inv = u8::from(rm_gp.hi() == 0);
        let rp_inv = u8::from(reg.hi4() == 0);
        let v = vvvv.map_or(0x0f, |x| (!x.id()) & 0x0f);
        let vp_inv = vvvv.map_or(1u8, |x| u8::from(x.hi4() == 0));
        self.emit_u8(0x62);
        self.emit_u8((r_inv << 7) | (1 << 6) | (b_inv << 5) | (rp_inv << 4) | (map & 0x03));
        self.emit_u8((u8::from(w) << 7) | (v << 3) | (1 << 2) | (pp & 0x03));
        self.emit_u8(vp_inv << 3);
        self.emit_u8(op);
        self.modrm_reg(reg.lo(), rm_gp.lo());
    }

    /// Emits a 4-byte EVEX 128-bit instruction with `reg = Gp`, `rm = Xmm` (e.g. `vcvttss2si`, `vpextrw`).
    fn evex_gp_xmm(&mut self, map: u8, pp: u8, w: bool, reg_gp: Gp, rm: Xmm, op: u8) {
        let r_inv = u8::from(reg_gp.hi() == 0);
        let x_inv = u8::from(rm.hi4() == 0);
        let b_inv = u8::from(rm.hi() == 0);
        self.emit_u8(0x62);
        self.emit_u8((r_inv << 7) | (x_inv << 6) | (b_inv << 5) | (1 << 4) | (map & 0x03));
        self.emit_u8((u8::from(w) << 7) | (0x0f << 3) | (1 << 2) | (pp & 0x03));
        self.emit_u8(1 << 3);
        self.emit_u8(op);
        self.modrm_reg(reg_gp.lo(), rm.lo());
    }

    /// Emits a 4-byte EVEX 128-bit immediate shift (`EVEX.128.66.0F.{W0/W1} op /reg_ext ib`, `vvvv=dst`, `rm=src`).
    fn evex_xmm_shift_imm(&mut self, w: bool, op: u8, reg_ext: u8, dst: Xmm, src: Xmm, imm: u8) {
        let x_inv = u8::from(src.hi4() == 0);
        let b_inv = u8::from(src.hi() == 0);
        let v = (!dst.id()) & 0x0f;
        let vp_inv = u8::from(dst.hi4() == 0);
        self.emit_u8(0x62);
        self.emit_u8((1 << 7) | (x_inv << 6) | (b_inv << 5) | (1 << 4) | 1);
        self.emit_u8((u8::from(w) << 7) | (v << 3) | (1 << 2) | 1);
        self.emit_u8(vp_inv << 3);
        self.emit_u8(op);
        self.modrm_reg(reg_ext, src.lo());
        self.emit_u8(imm);
    }

    /// `vpternlogd xmm, xmm, xmm, imm8` (`EVEX.128.66.0F3A.W0 25 /r ib`; AVX-512VL).
    pub fn vpternlogd_xxx(&mut self, dst: Xmm, vsrc: Xmm, rm: Xmm, imm: u8) {
        self.evex_rr(3, 1, false, 0, dst, Some(vsrc), rm, 0x25);
        self.emit_u8(imm);
    }
    /// `vcvtudq2ps xmm, xmm` (`EVEX.128.F2.0F.W0 7A /r`; AVX-512VL).
    pub fn vcvtudq2ps_xx(&mut self, dst: Xmm, src: Xmm) {
        self.evex_rr(1, 3, false, 0, dst, None, src, 0x7a);
    }
    /// `vbroadcastsd zmm, xmm` (`EVEX.512.66.0F38.W1 19 /r`).
    pub fn vbroadcastsd_zx(&mut self, dst: Xmm, src: Xmm) {
        self.evex_rr(2, 1, true, 2, dst, None, src, 0x19);
    }
    /// `vbroadcastss zmm, xmm` (`EVEX.512.66.0F38.W0 18 /r`).
    pub fn vbroadcastss_zx(&mut self, dst: Xmm, src: Xmm) {
        self.evex_rr(2, 1, false, 2, dst, None, src, 0x18);
    }
    /// `vpbroadcastb zmm, xmm` (`EVEX.512.66.0F38.W0 78 /r`; AVX-512BW).
    pub fn vpbroadcastb_zx(&mut self, dst: Xmm, src: Xmm) {
        self.evex_rr(2, 1, false, 2, dst, None, src, 0x78);
    }
    /// `vpbroadcastd zmm, xmm` (`EVEX.512.66.0F38.W0 58 /r`).
    pub fn vpbroadcastd_zx(&mut self, dst: Xmm, src: Xmm) {
        self.evex_rr(2, 1, false, 2, dst, None, src, 0x58);
    }
    /// `vpbroadcastq zmm, xmm` (`EVEX.512.66.0F38.W1 59 /r`).
    pub fn vpbroadcastq_zx(&mut self, dst: Xmm, src: Xmm) {
        self.evex_rr(2, 1, true, 2, dst, None, src, 0x59);
    }
    /// `vmovupd zmm, m512` (`EVEX.512.66.0F.W1 10 /r`).
    pub fn vmovupd_zm(&mut self, dst: Xmm, mem: Mem) {
        self.evex_mem(1, 1, true, 2, 64, dst, None, mem, 0x10);
    }
    /// `vmovupd m512, zmm` (`EVEX.512.66.0F.W1 11 /r`).
    pub fn vmovupd_mz(&mut self, mem: Mem, src: Xmm) {
        self.evex_mem(1, 1, true, 2, 64, src, None, mem, 0x11);
    }
    /// `vmovups zmm, m512` (`EVEX.512.0F.W0 10 /r`).
    pub fn vmovups_zm(&mut self, dst: Xmm, mem: Mem) {
        self.evex_mem(1, 0, false, 2, 64, dst, None, mem, 0x10);
    }
    /// `vmovups m512, zmm` (`EVEX.512.0F.W0 11 /r`).
    pub fn vmovups_mz(&mut self, mem: Mem, src: Xmm) {
        self.evex_mem(1, 0, false, 2, 64, src, None, mem, 0x11);
    }
    /// `vmovdqu32 zmm, m512` (`EVEX.512.F3.0F.W0 6F /r`).
    pub fn vmovdqu32_zm(&mut self, dst: Xmm, mem: Mem) {
        self.evex_mem(1, 2, false, 2, 64, dst, None, mem, 0x6f);
    }
    /// `vmovdqu32 m512, zmm` (`EVEX.512.F3.0F.W0 7F /r`).
    pub fn vmovdqu32_mz(&mut self, mem: Mem, src: Xmm) {
        self.evex_mem(1, 2, false, 2, 64, src, None, mem, 0x7f);
    }
    /// `vmovdqu64 zmm, m512` (`EVEX.512.F3.0F.W1 6F /r`).
    pub fn vmovdqu64_zm(&mut self, dst: Xmm, mem: Mem) {
        self.evex_mem(1, 2, true, 2, 64, dst, None, mem, 0x6f);
    }
    /// `vmovdqu64 m512, zmm` (`EVEX.512.F3.0F.W1 7F /r`).
    pub fn vmovdqu64_mz(&mut self, mem: Mem, src: Xmm) {
        self.evex_mem(1, 2, true, 2, 64, src, None, mem, 0x7f);
    }
    pub fn vaddpd_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(1, 1, true, 2, dst, Some(a), b, 0x58);
    }
    pub fn vsubpd_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(1, 1, true, 2, dst, Some(a), b, 0x5c);
    }
    pub fn vmulpd_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(1, 1, true, 2, dst, Some(a), b, 0x59);
    }
    pub fn vdivpd_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(1, 1, true, 2, dst, Some(a), b, 0x5e);
    }
    pub fn vaddpd_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(1, 1, true, 2, 64, dst, Some(a), mem, 0x58);
    }
    pub fn vsubpd_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(1, 1, true, 2, 64, dst, Some(a), mem, 0x5c);
    }
    pub fn vmulpd_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(1, 1, true, 2, 64, dst, Some(a), mem, 0x59);
    }
    pub fn vdivpd_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(1, 1, true, 2, 64, dst, Some(a), mem, 0x5e);
    }
    pub fn vaddps_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(1, 0, false, 2, dst, Some(a), b, 0x58);
    }
    pub fn vsubps_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(1, 0, false, 2, dst, Some(a), b, 0x5c);
    }
    pub fn vmulps_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(1, 0, false, 2, dst, Some(a), b, 0x59);
    }
    pub fn vdivps_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(1, 0, false, 2, dst, Some(a), b, 0x5e);
    }
    pub fn vaddps_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(1, 0, false, 2, 64, dst, Some(a), mem, 0x58);
    }
    pub fn vsubps_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(1, 0, false, 2, 64, dst, Some(a), mem, 0x5c);
    }
    pub fn vmulps_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(1, 0, false, 2, 64, dst, Some(a), mem, 0x59);
    }
    pub fn vdivps_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(1, 0, false, 2, 64, dst, Some(a), mem, 0x5e);
    }
    pub fn vpaddd_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(1, 1, false, 2, dst, Some(a), b, 0xfe);
    }
    pub fn vpsubd_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(1, 1, false, 2, dst, Some(a), b, 0xfa);
    }
    pub fn vpmulld_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(2, 1, false, 2, dst, Some(a), b, 0x40);
    }
    pub fn vpandd_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(1, 1, false, 2, dst, Some(a), b, 0xdb);
    }
    pub fn vpord_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(1, 1, false, 2, dst, Some(a), b, 0xeb);
    }
    pub fn vpxord_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(1, 1, false, 2, dst, Some(a), b, 0xef);
    }
    pub fn vpaddd_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(1, 1, false, 2, 64, dst, Some(a), mem, 0xfe);
    }
    pub fn vpsubd_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(1, 1, false, 2, 64, dst, Some(a), mem, 0xfa);
    }
    pub fn vpmulld_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(2, 1, false, 2, 64, dst, Some(a), mem, 0x40);
    }
    pub fn vpandd_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(1, 1, false, 2, 64, dst, Some(a), mem, 0xdb);
    }
    pub fn vpord_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(1, 1, false, 2, 64, dst, Some(a), mem, 0xeb);
    }
    pub fn vpxord_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(1, 1, false, 2, 64, dst, Some(a), mem, 0xef);
    }
    pub fn vpaddq_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(1, 1, true, 2, dst, Some(a), b, 0xd4);
    }
    pub fn vpsubq_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(1, 1, true, 2, dst, Some(a), b, 0xfb);
    }
    pub fn vpandq_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(1, 1, true, 2, dst, Some(a), b, 0xdb);
    }
    pub fn vporq_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(1, 1, true, 2, dst, Some(a), b, 0xeb);
    }
    pub fn vpxorq_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(1, 1, true, 2, dst, Some(a), b, 0xef);
    }
    pub fn vpaddq_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(1, 1, true, 2, 64, dst, Some(a), mem, 0xd4);
    }
    pub fn vpsubq_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(1, 1, true, 2, 64, dst, Some(a), mem, 0xfb);
    }
    pub fn vpandq_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(1, 1, true, 2, 64, dst, Some(a), mem, 0xdb);
    }
    pub fn vporq_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(1, 1, true, 2, 64, dst, Some(a), mem, 0xeb);
    }
    pub fn vpxorq_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(1, 1, true, 2, 64, dst, Some(a), mem, 0xef);
    }
    /// `vpsadbw zmm, zmm, m512` (`EVEX.512.66.0F.WIG F6 /r`; AVX-512BW).
    pub fn vpsadbw_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(1, 1, false, 2, 64, dst, Some(a), mem, 0xf6);
    }
    /// `vextracti64x4 ymm, zmm, imm8` (`EVEX.512.66.0F3A.W1 3B /r ib`; `reg = src`, `rm = dst`).
    pub fn vextracti64x4_yz(&mut self, dst: Xmm, src: Xmm, imm: u8) {
        self.evex_rr(3, 1, true, 2, src, None, dst, 0x3b);
        self.emit_u8(imm & 1);
    }
    /// `vshufi32x4 zmm, zmm, zmm, imm8` (`EVEX.512.66.0F3A.W0 43 /r ib`).
    pub fn vshufi32x4_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm, imm: u8) {
        self.evex_rr(3, 1, false, 2, dst, Some(a), b, 0x43);
        self.emit_u8(imm);
    }
    /// `vinsertf32x4 ymm, ymm, xmm, imm8` (`EVEX.256.66.0F3A.W0 18 /r ib`; AVX-512VL).
    pub fn vinsertf32x4_yyx(&mut self, dst: Xmm, a: Xmm, b: Xmm, imm: u8) {
        self.evex_rr(3, 1, false, 1, dst, Some(a), b, 0x18);
        self.emit_u8(imm & 1);
    }
    /// `vfmadd231ps zmm, zmm, zmm` (`EVEX.512.66.0F38.W0 B8 /r`).
    pub fn vfmadd231ps_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(2, 1, false, 2, dst, Some(a), b, 0xb8);
    }
    /// `vfmadd231ps zmm, zmm, m512` (`EVEX.512.66.0F38.W0 B8 /r`).
    pub fn vfmadd231ps_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(2, 1, false, 2, 64, dst, Some(a), mem, 0xb8);
    }
    /// `vfnmadd231ps zmm, zmm, zmm` (`EVEX.512.66.0F38.W0 BC /r`).
    pub fn vfnmadd231ps_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(2, 1, false, 2, dst, Some(a), b, 0xbc);
    }
    /// `vfnmadd231ps zmm, zmm, m512` (`EVEX.512.66.0F38.W0 BC /r`).
    pub fn vfnmadd231ps_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(2, 1, false, 2, 64, dst, Some(a), mem, 0xbc);
    }
    /// `vfmadd231pd zmm, zmm, zmm` (`EVEX.512.66.0F38.W1 B8 /r`).
    pub fn vfmadd231pd_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(2, 1, true, 2, dst, Some(a), b, 0xb8);
    }
    /// `vfmadd231pd zmm, zmm, m512` (`EVEX.512.66.0F38.W1 B8 /r`).
    pub fn vfmadd231pd_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(2, 1, true, 2, 64, dst, Some(a), mem, 0xb8);
    }
    /// `vfnmadd231pd zmm, zmm, zmm` (`EVEX.512.66.0F38.W1 BC /r`).
    pub fn vfnmadd231pd_zzz(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr(2, 1, true, 2, dst, Some(a), b, 0xbc);
    }
    /// `vfnmadd231pd zmm, zmm, m512` (`EVEX.512.66.0F38.W1 BC /r`).
    pub fn vfnmadd231pd_zzm(&mut self, dst: Xmm, a: Xmm, mem: Mem) {
        self.evex_mem(2, 1, true, 2, 64, dst, Some(a), mem, 0xbc);
    }
    /// `vcmpps k1, zmm/ymm/xmm, zmm/ymm/xmm, imm8` (`EVEX.{128,256,512}.0F.W0 C2 /r ib`; `ll`: 0=128, 1=256, 2=512).
    pub fn vcmpps_k1_evex(&mut self, ll: u8, a: Xmm, b: Xmm, imm: u8) {
        self.evex_rr(1, 0, false, ll, Xmm::Xmm1, Some(a), b, 0xc2);
        self.emit_u8(imm);
    }
    /// `vcmppd k1, zmm/ymm/xmm, zmm/ymm/xmm, imm8` (`EVEX.{128,256,512}.66.0F.W1 C2 /r ib`; `ll`: 0=128, 1=256, 2=512).
    pub fn vcmppd_k1_evex(&mut self, ll: u8, a: Xmm, b: Xmm, imm: u8) {
        self.evex_rr(1, 1, true, ll, Xmm::Xmm1, Some(a), b, 0xc2);
        self.emit_u8(imm);
    }
    /// `vmovaps zmm/ymm/xmm {k1}, zmm/ymm/xmm` (`EVEX.{128,256,512}.0F.W0 28 /r`, merge-masked with `k1`).
    pub fn vmovaps_k1_evex(&mut self, ll: u8, dst: Xmm, src: Xmm) {
        self.evex_rr_mask(1, 0, false, ll, 1, dst, None, src, 0x28);
    }
    /// `vmovapd zmm/ymm/xmm {k1}, zmm/ymm/xmm` (`EVEX.{128,256,512}.66.0F.W1 28 /r`, merge-masked with `k1`).
    pub fn vmovapd_k1_evex(&mut self, ll: u8, dst: Xmm, src: Xmm) {
        self.evex_rr_mask(1, 1, true, ll, 1, dst, None, src, 0x28);
    }
    /// `vmovaps zmm, zmm` (`EVEX.512.0F.W0 28 /r`).
    pub fn vmovaps_zz(&mut self, dst: Xmm, src: Xmm) {
        if dst != src {
            self.evex_rr(1, 0, false, 2, dst, None, src, 0x28);
        }
    }
    /// `vsqrtps zmm, zmm` (`EVEX.512.0F.W0 51 /r`).
    pub fn vsqrtps_zz(&mut self, dst: Xmm, src: Xmm) {
        self.evex_rr(1, 0, false, 2, dst, None, src, 0x51);
    }
    /// `vcvtdq2ps zmm, zmm` (`EVEX.512.0F.W0 5B /r`).
    pub fn vcvtdq2ps_zz(&mut self, dst: Xmm, src: Xmm) {
        self.evex_rr(1, 0, false, 2, dst, None, src, 0x5b);
    }
    /// `vcmpps k1 {k1}, zmm, zmm, imm8` (`EVEX.512.0F.W0 C2 /r ib`, masked by `k1` so `k1 &= cmp(a, b)`).
    pub fn vcmpps_k1_k1_z(&mut self, a: Xmm, b: Xmm, imm: u8) {
        self.evex_rr_mask(1, 0, false, 2, 1, Xmm::Xmm1, Some(a), b, 0xc2);
        self.emit_u8(imm);
    }
    /// `vpaddd zmm {k1}, zmm, zmm` (`EVEX.512.66.0F.W0 FE /r`, merge-masked by `k1`).
    pub fn vpaddd_zzz_k1(&mut self, dst: Xmm, a: Xmm, b: Xmm) {
        self.evex_rr_mask(1, 1, false, 2, 1, dst, Some(a), b, 0xfe);
    }
    /// `kmovw k1, r32` (`VEX.L0.0F.W0 92 /r`).
    pub fn kmovw_k1_r32(&mut self, src: Gp) {
        if src.hi() == 0 {
            self.emit_u8(0xc5);
            self.emit_u8(0xf8);
        } else {
            self.emit_u8(0xc4);
            self.emit_u8(0xc1);
            self.emit_u8(0x78);
        }
        self.emit_u8(0x92);
        self.modrm_reg(1, src.lo());
    }
    /// `kortestw k1, k1` (`VEX.L0.0F.W0 98 /r`): sets `ZF = 1` iff `k1 == 0`.
    pub fn kortestw_k1_k1(&mut self) {
        self.emit_u8(0xc5);
        self.emit_u8(0xf8);
        self.emit_u8(0x98);
        self.emit_u8(0xc9);
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
