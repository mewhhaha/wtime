//! Fast exact cross-block liveness + clobber-aware linear-scan register allocator,
//! plus the single source of truth for internal Wasm ABI layouts and helper clobbers.
//!
//! Register partition (internal Wasm ABI):
//! - Fixed / pinned:
//!   - `Rsp`: native stack pointer (16-byte aligned at calls)
//!   - `R15`: `Ctx*`
//!   - `R14`: `mem_base` (fixed virtual `mmap` base — invariant across `memory.grow`)
//!   - `R13`: `mem_len` (current byte length of memory — updated in-place by `memory.grow`)
//!   - `R12`: pinned hottest scalar mutable global (when present; else unused)
//! - Scratch (never allocated):
//!   - `R11` / `Xmm15`: primary scratch & parallel-move cycle-breaking temporary
//!   - `R10` / `Xmm14`: secondary scratch & parallel-move mem-to-mem copy temporary
//! - Allocatable GP (9 regs):
//!   - Caller-saved (7): `Rax, Rcx, Rdx, Rsi, Rdi, R8, R9`
//!   - Callee-saved (2): `Rbx, Rbp`
//! - Allocatable XMM (14 regs):
//!   - Caller-saved (8): `Xmm0..Xmm7`
//!   - Callee-saved (6, Stage 2 internal convention): `Xmm8..Xmm13`

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::asm::{Gp, Xmm};
use super::ir::{IntBinOp, IrFunc, Op, Term, Ty};
use super::module::{CompileBudget, MAX_FRAME_BYTES, MAX_LIVENESS_BYTES, MAX_LIVENESS_WORK_UNITS};

/// Pure scalar constant value that can be rematerialized cheaply at use sites with zero stack slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RematConst {
    I32(i32),
    I64(i64),
    F32(u32),
    F64(u64),
}

impl RematConst {
    #[inline]
    pub const fn is_zero(self) -> bool {
        matches!(
            self,
            RematConst::I32(0) | RematConst::I64(0) | RematConst::F32(0) | RematConst::F64(0)
        )
    }
}

/// Physical location assigned to an SSA `Val`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loc {
    Gp(Gp),
    Xmm(Xmm),
    /// Stack spill slot at `[rsp + offset]` (8-byte aligned for scalars, 16-byte aligned for V128).
    Spill(i32),
    /// Rematerialized constant (zero stack slot, zero def code, zero register/clobber interference).
    Remat(RematConst),
}

/// Optional module-wide extra callee-saved GP registers and ISA clobber flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AllocConfig {
    /// True when `layout.pinned_global.is_none()`, freeing `R12` as a 3rd callee-saved GP register.
    pub allow_r12: bool,
    /// True when `layout.fixed_mem_bytes.is_some()`, freeing `R13` as a 4th callee-saved GP register.
    pub allow_r13: bool,
    /// True when BMI2 `shlx`/`shrx`/`sarx` are enabled, eliminating `RCX` clobbers on variable shifts.
    pub has_bmi2: bool,
}

impl AllocConfig {
    pub fn gp_alloc(self) -> ([Gp; 11], usize) {
        let mut regs = [
            Gp::Rax,
            Gp::Rcx,
            Gp::Rdx,
            Gp::Rsi,
            Gp::Rdi,
            Gp::R8,
            Gp::R9,
            Gp::Rbx,
            Gp::Rbp,
            Gp::R12,
            Gp::R13,
        ];
        let mut n = 9usize;
        if self.allow_r12 {
            regs[n] = Gp::R12;
            n += 1;
        }
        if self.allow_r13 {
            regs[n] = Gp::R13;
            n += 1;
        }
        (regs, n)
    }

    pub fn gp_callee_saved(self) -> ([Gp; 4], usize) {
        let mut regs = [Gp::Rbx, Gp::Rbp, Gp::R12, Gp::R13];
        let mut n = 2usize;
        if self.allow_r12 {
            regs[n] = Gp::R12;
            n += 1;
        }
        if self.allow_r13 {
            regs[n] = Gp::R13;
            n += 1;
        }
        (regs, n)
    }

    #[inline]
    pub const fn is_callee_saved_gp(self, g: Gp) -> bool {
        matches!(g, Gp::Rbx | Gp::Rbp)
            || (self.allow_r12 && matches!(g, Gp::R12))
            || (self.allow_r13 && matches!(g, Gp::R13))
    }

    #[inline]
    pub const fn is_alloc_gp(self, g: Gp) -> bool {
        matches!(
            g,
            Gp::Rax | Gp::Rcx | Gp::Rdx | Gp::Rsi | Gp::Rdi | Gp::R8 | Gp::R9 | Gp::Rbx | Gp::Rbp
        ) || (self.allow_r12 && matches!(g, Gp::R12))
            || (self.allow_r13 && matches!(g, Gp::R13))
    }
}

pub const CALL_GP_ARGS: [Gp; 6] = [Gp::Rdi, Gp::Rsi, Gp::Rdx, Gp::Rcx, Gp::R8, Gp::R9];
pub const CALL_FP_ARGS: [Xmm; 8] = [
    Xmm::Xmm0,
    Xmm::Xmm1,
    Xmm::Xmm2,
    Xmm::Xmm3,
    Xmm::Xmm4,
    Xmm::Xmm5,
    Xmm::Xmm6,
    Xmm::Xmm7,
];
pub const CALL_GP_RETS: [Gp; 7] = [Gp::Rax, Gp::Rdx, Gp::Rcx, Gp::Rsi, Gp::Rdi, Gp::R8, Gp::R9];
pub const CALL_FP_RETS: [Xmm; 8] = CALL_FP_ARGS;

#[allow(dead_code)]
pub const GP_CALLER_SAVED: [Gp; 7] = [Gp::Rax, Gp::Rcx, Gp::Rdx, Gp::Rsi, Gp::Rdi, Gp::R8, Gp::R9];
#[allow(dead_code)]
pub const GP_CALLEE_SAVED: [Gp; 2] = [Gp::Rbx, Gp::Rbp];
#[allow(dead_code)]
pub const GP_ALLOC: [Gp; 9] = [
    Gp::Rax,
    Gp::Rcx,
    Gp::Rdx,
    Gp::Rsi,
    Gp::Rdi,
    Gp::R8,
    Gp::R9,
    Gp::Rbx,
    Gp::Rbp,
];
#[allow(dead_code)]
pub const XMM_CALLER_SAVED: [Xmm; 8] = [
    Xmm::Xmm0,
    Xmm::Xmm1,
    Xmm::Xmm2,
    Xmm::Xmm3,
    Xmm::Xmm4,
    Xmm::Xmm5,
    Xmm::Xmm6,
    Xmm::Xmm7,
];
pub const XMM_CALLEE_SAVED: [Xmm; 6] = [
    Xmm::Xmm8,
    Xmm::Xmm9,
    Xmm::Xmm10,
    Xmm::Xmm11,
    Xmm::Xmm12,
    Xmm::Xmm13,
];
pub const XMM_ALLOC: [Xmm; 14] = [
    Xmm::Xmm0,
    Xmm::Xmm1,
    Xmm::Xmm2,
    Xmm::Xmm3,
    Xmm::Xmm4,
    Xmm::Xmm5,
    Xmm::Xmm6,
    Xmm::Xmm7,
    Xmm::Xmm8,
    Xmm::Xmm9,
    Xmm::Xmm10,
    Xmm::Xmm11,
    Xmm::Xmm12,
    Xmm::Xmm13,
];

/// Bitmask over `Gp::id()` (bits 0..15).
pub const MASK_GP_CALLER: u16 =
    (1 << 0) | (1 << 1) | (1 << 2) | (1 << 6) | (1 << 7) | (1 << 8) | (1 << 9);
pub const MASK_RAX: u16 = 1 << 0;
pub const MASK_RAX_RDX: u16 = (1 << 0) | (1 << 2);
pub const MASK_RCX: u16 = 1 << 1;
/// Bitmask over `Xmm::id()` (caller-saved `Xmm0..Xmm7`; `Xmm8..Xmm13` are callee-saved in Stage 2).
pub const MASK_XMM_CALLER: u16 = (1 << 8) - 1;

/// Single source of truth for internal runtime helper clobber masks `(gp_mask, xmm_mask)`.
///
/// Every helper guarantees preservation of `Rbx, Rbp, R12..R15` (except `MemGrow` which updates
/// `R13 = mem_len` in place) and all allocated `Xmm0..Xmm13` (helpers use only scratch `Xmm14`/`Xmm15`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelperKind {
    /// Input in `Xmm15`, returns int in `Rax` + trap code in `R11d`; clobbers only `Rax, R10, R11, Xmm14, Xmm15`.
    Trunc,
    /// Input in `Rdi`, returns `old_pages | -1` in `Rax`, updates `R13`; clobbers caller-saved GP only.
    MemGrow,
    /// Inputs in `Rdi, Rsi, Rdx`, returns trap in `R11d`; clobbers caller-saved GP only.
    MemCopy,
    /// Inputs in `Rdi, Rsi, Rdx`, returns trap in `R11d`; clobbers caller-saved GP only.
    MemFill,
}

#[inline]
pub const fn helper_clobbers(kind: HelperKind) -> (u16, u16) {
    match kind {
        HelperKind::Trunc => (MASK_RAX, 0),
        HelperKind::MemGrow | HelperKind::MemCopy | HelperKind::MemFill => (MASK_GP_CALLER, 0),
    }
}

/// Typed slot placement for internal Wasm call arguments or multi-value return buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbiSlot {
    Gp(Gp),
    Xmm(Xmm),
    /// Byte offset from the start of the outgoing arg area (`[rsp + off]`) or return buffer (`[buf + off]`).
    Stack(i32),
}

/// Unified typed ABI layout shared by caller, callee, and stack-frame sizing.
pub struct AbiLayout {
    pub slots: Vec<AbiSlot>,
    /// 16-byte aligned byte size required for overflow slots (0 if all values fit in registers).
    pub stack_bytes: i32,
}

fn checked_align_i32(v: i32, align: i32) -> Result<i32, String> {
    let mask = align - 1;
    v.checked_add(mask)
        .map(|x| x & !mask)
        .ok_or_else(|| "x64: ABI/frame i32 offset overflow".to_string())
}

pub fn layout_args_stack_bytes<I: IntoIterator<Item = Ty>>(tys: I) -> Result<i32, String> {
    let (mut ng, mut nf, mut off) = (0usize, 0usize, 0i32);
    for ty in tys {
        if ty.is_fp() {
            if nf < CALL_FP_ARGS.len() {
                nf += 1;
            } else {
                let (sz, align) = if ty == Ty::V128 { (16, 16) } else { (8, 8) };
                off = checked_align_i32(off, align)?;
                off = off.checked_add(sz).ok_or("x64: arg stack size overflow")?;
            }
        } else if ng < CALL_GP_ARGS.len() {
            ng += 1;
        } else {
            off = checked_align_i32(off, 8)?;
            off = off.checked_add(8).ok_or("x64: arg stack size overflow")?;
        }
    }
    checked_align_i32(off, 16)
}

pub fn layout_rets_stack_bytes<I: IntoIterator<Item = Ty>>(tys: I) -> Result<i32, String> {
    let (mut ng, mut nf, mut off) = (0usize, 0usize, 0i32);
    for ty in tys {
        if ty.is_fp() {
            if nf < CALL_FP_RETS.len() {
                nf += 1;
            } else {
                let (sz, align) = if ty == Ty::V128 { (16, 16) } else { (8, 8) };
                off = checked_align_i32(off, align)?;
                off = off.checked_add(sz).ok_or("x64: ret stack size overflow")?;
            }
        } else if ng < CALL_GP_RETS.len() {
            ng += 1;
        } else {
            off = checked_align_i32(off, 8)?;
            off = off.checked_add(8).ok_or("x64: ret stack size overflow")?;
        }
    }
    checked_align_i32(off, 16)
}

#[inline]
pub fn for_each_arg_slot<I, F>(tys: I, mut f: F) -> Result<i32, String>
where
    I: IntoIterator<Item = Ty>,
    F: FnMut(usize, AbiSlot),
{
    let (mut ng, mut nf, mut off) = (0usize, 0usize, 0i32);
    for (idx, ty) in tys.into_iter().enumerate() {
        let slot = if ty.is_fp() {
            if nf < CALL_FP_ARGS.len() {
                let x = CALL_FP_ARGS[nf];
                nf += 1;
                AbiSlot::Xmm(x)
            } else {
                let (sz, align) = if ty == Ty::V128 { (16, 16) } else { (8, 8) };
                off = checked_align_i32(off, align)?;
                let s = AbiSlot::Stack(off);
                off = off.checked_add(sz).ok_or("x64: arg stack size overflow")?;
                s
            }
        } else if ng < CALL_GP_ARGS.len() {
            let g = CALL_GP_ARGS[ng];
            ng += 1;
            AbiSlot::Gp(g)
        } else {
            off = checked_align_i32(off, 8)?;
            let s = AbiSlot::Stack(off);
            off = off.checked_add(8).ok_or("x64: arg stack size overflow")?;
            s
        };
        f(idx, slot);
    }
    checked_align_i32(off, 16)
}

#[inline]
pub fn for_each_ret_slot<I, F>(tys: I, mut f: F) -> Result<i32, String>
where
    I: IntoIterator<Item = Ty>,
    F: FnMut(usize, AbiSlot),
{
    let (mut ng, mut nf, mut off) = (0usize, 0usize, 0i32);
    for (idx, ty) in tys.into_iter().enumerate() {
        let slot = if ty.is_fp() {
            if nf < CALL_FP_RETS.len() {
                let x = CALL_FP_RETS[nf];
                nf += 1;
                AbiSlot::Xmm(x)
            } else {
                let (sz, align) = if ty == Ty::V128 { (16, 16) } else { (8, 8) };
                off = checked_align_i32(off, align)?;
                let s = AbiSlot::Stack(off);
                off = off.checked_add(sz).ok_or("x64: ret stack size overflow")?;
                s
            }
        } else if ng < CALL_GP_RETS.len() {
            let g = CALL_GP_RETS[ng];
            ng += 1;
            AbiSlot::Gp(g)
        } else {
            off = checked_align_i32(off, 8)?;
            let s = AbiSlot::Stack(off);
            off = off.checked_add(8).ok_or("x64: ret stack size overflow")?;
            s
        };
        f(idx, slot);
    }
    checked_align_i32(off, 16)
}

pub fn layout_args(tys: &[Ty]) -> Result<AbiLayout, String> {
    let mut slots = Vec::with_capacity(tys.len());
    let stack_bytes = for_each_arg_slot(tys.iter().copied(), |_, s| slots.push(s))?;
    Ok(AbiLayout { slots, stack_bytes })
}

pub fn layout_rets(tys: &[Ty]) -> Result<AbiLayout, String> {
    let mut slots = Vec::with_capacity(tys.len());
    let stack_bytes = for_each_ret_slot(tys.iter().copied(), |_, s| slots.push(s))?;
    Ok(AbiLayout { slots, stack_bytes })
}

pub struct AllocResult {
    pub locs: Vec<Loc>,
    /// Block order emitted linearly (reachable blocks in RPO order).
    pub block_order: Vec<u32>,
    /// Callee-saved GP registers actually modified (`Rbx`, `Rbp`).
    pub used_callee_gp: Vec<Gp>,
    /// Callee-saved XMM registers actually modified (`Xmm8..Xmm13`), saved in frame at `saved_xmm_base`.
    pub used_callee_xmm: Vec<Xmm>,
    /// 16-byte aligned offset in this frame (`[rsp + saved_xmm_base]`) where `used_callee_xmm` are saved.
    pub saved_xmm_base: i32,
    /// Total stack bytes subtracted from `rsp` below pushed callee-saved regs (keeps `rsp` 16-byte aligned).
    pub frame_size: i32,
    /// Offset of the outgoing stack-arg area (always 0 when non-empty).
    pub outgoing_arg_bytes: i32,
    /// Offset in this frame (`[rsp + ret_buf_off]`) reserved for callee overflow returns (16-byte aligned).
    pub ret_buf_off: i32,
    /// Size of `ret_buf` (0 if no call in this function has overflow returns).
    pub ret_buf_bytes: i32,
    /// Frame slot (`[rsp + off]`) saving incoming `[r15 + OFF_RET_SPILL_PTR]` when THIS function has overflow returns.
    pub saved_ret_ptr_off: Option<i32>,
    /// Per-block (`block_id`-indexed) bitmask of GP registers live-in at block entry (excluding `block.params`).
    pub block_live_in_gp: Vec<u16>,
    /// Per-block (`block_id`-indexed) bitmask of XMM registers live-in at block entry (excluding `block.params`).
    pub block_live_in_xmm: Vec<u16>,
}

#[derive(Clone, Copy)]
struct Clobber {
    pos: u32,
    gp_mask: u16,
    xmm_mask: u16,
}

#[derive(Clone, Copy)]
struct Interval {
    val: u32,
    start: u32,
    end: u32,
    /// Index into `segs_pool` when `seg_len > 0` (live range has holes); `seg_len == 0` means single `[start, end)`.
    seg_off: u32,
    seg_len: u16,
    ty: Ty,
    clob_gp: u16,
    clob_xmm: u16,
    score: u64,
    remat: Option<RematConst>,
    /// True when this non-zero constant is loop-invariant (defined at lower loop depth than a use)
    /// and may occupy a free callee-saved GP register to avoid in-loop constant rematerialization.
    remat_hoisted: bool,
}

#[derive(Clone, Copy)]
struct ActiveEntry {
    start: u32,
    end: u32,
    seg_off: u32,
    seg_len: u16,
    reg_idx: u8,
    val: u32,
    score: u64,
    remat: Option<RematConst>,
}

#[inline]
fn segs_overlap(a: &[(u32, u32)], b: &[(u32, u32)]) -> bool {
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        let (as_, ae) = a[i];
        let (bs, be) = b[j];
        if as_ < be && bs < ae {
            return true;
        }
        if ae <= be {
            i += 1;
        } else {
            j += 1;
        }
    }
    false
}

#[inline]
fn active_overlaps_iv(a: &ActiveEntry, iv: &Interval, pool: &[(u32, u32)]) -> bool {
    if a.end <= iv.start || iv.end <= a.start {
        return false;
    }
    if a.seg_len == 0 && iv.seg_len == 0 {
        return true;
    }
    let a_single = [(a.start, a.end)];
    let b_single = [(iv.start, iv.end)];
    let sa = if a.seg_len == 0 {
        &a_single[..]
    } else {
        let off = a.seg_off as usize;
        &pool[off..off + a.seg_len as usize]
    };
    let sb = if iv.seg_len == 0 {
        &b_single[..]
    } else {
        let off = iv.seg_off as usize;
        &pool[off..off + iv.seg_len as usize]
    };
    segs_overlap(sa, sb)
}

/// Computes RPO order of reachable blocks, exact cross-block liveness, and linear-scan allocation.
#[allow(dead_code)]
pub fn allocate(ir: &IrFunc) -> Result<AllocResult, String> {
    allocate_with_config(ir, AllocConfig::default(), None)
}

/// Computes RPO order, exact cross-block liveness, and linear-scan allocation with default register partition.
#[allow(dead_code)]
pub fn allocate_with_budget(
    ir: &IrFunc,
    budget: Option<&CompileBudget>,
) -> Result<AllocResult, String> {
    allocate_with_config(ir, AllocConfig::default(), budget)
}

/// Reusable per-worker scratch buffers for `allocate_with_config_and_scratch`, eliminating
/// per-function CFG, liveness-helper, and interval heap allocations.
#[derive(Default)]
pub struct RegallocScratch {
    succ_offs: Vec<u32>,
    succ_data: Vec<u32>,
    succ_seen: Vec<u32>,
    visited: Vec<bool>,
    postorder: Vec<u32>,
    dfs: Vec<(u32, usize)>,
    rpo_pos: Vec<u32>,
    is_trap_blk: Vec<bool>,
    has_call_blk: Vec<bool>,
    has_store_blk: Vec<bool>,
    has_unchecked_mem_blk: Vec<bool>,
    placed: Vec<bool>,
    block_pos_in_order: Vec<u32>,
    block_start: Vec<u32>,
    block_end: Vec<u32>,
    loop_depth: Vec<u8>,
    rpo_pred_counts: Vec<u32>,
    rpo_pred_offs: Vec<u32>,
    rpo_pred_data: Vec<u32>,
    backedges: Vec<(u32, u32)>,
    rpo_depth: Vec<u8>,
    loop_seen: Vec<u32>,
    loop_wl: Vec<u32>,
    cb_hull: Vec<(u32, u32)>,
    last_use_in_blk: Vec<(u32, u32)>,
    def_pos: Vec<u32>,
    def_block: Vec<u32>,
    last_use: Vec<u32>,
    cross_block: Vec<bool>,
    val_remat: Vec<Option<RematConst>>,
    clobbers: Vec<Clobber>,
    clob_seg: Vec<(u16, u16)>,
    live_use: Vec<u64>,
    live_def: Vec<u64>,
    live_in: Vec<u64>,
    live_out: Vec<u64>,
    cb_seg_first: Vec<(u32, u32)>,
    cb_seg_extra_idx: Vec<u32>,
    cb_seg_overflow: Vec<bool>,
    segs_pool: Vec<(u32, u32)>,
    cb_id: Vec<u32>,
    cb_vals: Vec<u32>,
    val_weight: Vec<u32>,
    max_use_loop_depth: Vec<u8>,
    val_non_trap: Vec<bool>,
    hint_gp: Vec<Option<Gp>>,
    hint_xmm: Vec<Option<Xmm>>,
    hint_peers: Vec<[u32; 4]>,
    first_edge_arg: Vec<u32>,
    assigned: Vec<bool>,
    val_clob_gp: Vec<u16>,
    val_clob_xmm: Vec<u16>,
    gp_intervals: Vec<Interval>,
    fp_intervals: Vec<Interval>,
    active_gp: Vec<ActiveEntry>,
    active_fp: Vec<ActiveEntry>,
    slot8_end: Vec<u32>,
    free_spills8: BinaryHeap<Reverse<(u32, i32)>>,
    free_spills16: BinaryHeap<Reverse<(u32, i32)>>,
}

#[inline]
fn reset_fill<T: Copy>(v: &mut Vec<T>, len: usize, fill: T) {
    v.clear();
    if v.capacity() > 16_384 && len <= 4096 {
        v.shrink_to(4096);
    }
    v.resize(len, fill);
}

/// Computes RPO order, exact cross-block liveness (charging concurrent peak liveness memory BEFORE
/// allocation), constant rematerialization, spill-slot coalescing, and linear-scan allocation while
/// polling `budget`'s cooperative deadline.
pub fn allocate_with_config(
    ir: &IrFunc,
    cfg: AllocConfig,
    budget: Option<&CompileBudget>,
) -> Result<AllocResult, String> {
    let mut scratch = RegallocScratch::default();
    allocate_with_config_and_scratch(ir, cfg, budget, &mut scratch)
}

/// Same as `allocate_with_config`, reusing worker-local `RegallocScratch` buffers.
pub fn allocate_with_config_and_scratch(
    ir: &IrFunc,
    cfg: AllocConfig,
    budget: Option<&CompileBudget>,
    scratch: &mut RegallocScratch,
) -> Result<AllocResult, String> {
    if let Some(b) = budget {
        b.check_deadline()?;
    }
    let (gp_alloc_buf, gp_alloc_len) = cfg.gp_alloc();
    let gp_alloc = &gp_alloc_buf[..gp_alloc_len];
    let (gp_callee_buf, gp_callee_len) = cfg.gp_callee_saved();
    let gp_callee = &gp_callee_buf[..gp_callee_len];
    let n_blocks = ir.blocks.len();
    let n_vals = ir.val_tys.len();

    // 1. Compute reachable block order (RPO) and hot-fallthrough trace layout using flat CSR
    //    `succ_offs`/`succ_data` and worker-local scratch buffers (zero per-block `Vec`s).
    scratch.succ_offs.clear();
    scratch.succ_data.clear();
    scratch.postorder.clear();
    let block_order: Vec<u32> = if n_blocks == 1 {
        let mut has_self = false;
        ir.blocks[0].term.for_each_succ(&ir.br_tables, |s| {
            if s.0 == 0 {
                has_self = true;
            }
        });
        scratch
            .succ_offs
            .extend_from_slice(&[0, u32::from(has_self)]);
        if has_self {
            scratch.succ_data.push(0);
        }
        scratch.postorder.push(0);
        reset_fill(&mut scratch.rpo_pos, 1, 0u32);
        reset_fill(&mut scratch.block_pos_in_order, 1, 0u32);
        vec![0u32]
    } else {
        reset_fill(&mut scratch.succ_seen, n_blocks, u32::MAX);
        scratch.succ_offs.reserve(n_blocks + 1);
        scratch.succ_offs.push(0);
        for (bi, bb) in ir.blocks.iter().enumerate() {
            if let Some(b) = budget
                && (bi & 4095) == 4095
            {
                b.check_deadline()?;
            }
            let stamp = bi as u32;
            bb.term.for_each_succ(&ir.br_tables, |s| {
                let sid = s.0 as usize;
                if scratch.succ_seen[sid] != stamp {
                    scratch.succ_seen[sid] = stamp;
                    scratch.succ_data.push(s.0);
                }
            });
            scratch.succ_offs.push(scratch.succ_data.len() as u32);
        }
        reset_fill(&mut scratch.visited, n_blocks, false);
        scratch.dfs.clear();
        scratch.dfs.push((0, 0));
        scratch.visited[0] = true;
        let mut dfs_steps: usize = 0;
        while let Some((b, si)) = scratch.dfs.last_mut() {
            dfs_steps += 1;
            if let Some(bg) = budget
                && (dfs_steps & 4095) == 0
            {
                bg.check_deadline()?;
            }
            let cur = *b as usize;
            let ss = scratch.succ_offs[cur] as usize;
            let se = scratch.succ_offs[cur + 1] as usize;
            if *si < se - ss {
                let nxt = scratch.succ_data[ss + *si];
                *si += 1;
                if !scratch.visited[nxt as usize] {
                    scratch.visited[nxt as usize] = true;
                    scratch.dfs.push((nxt, 0));
                }
            } else {
                scratch.postorder.push(*b);
                scratch.dfs.pop();
            }
        }
        scratch.postorder.reverse();
        let n_ord = scratch.postorder.len();
        reset_fill(&mut scratch.rpo_pos, n_blocks, u32::MAX);
        for (ri, &b) in scratch.postorder.iter().enumerate() {
            scratch.rpo_pos[b as usize] = ri as u32;
        }

        // 1b. Hot-fallthrough trace layout over reachable blocks (always starts with bb0).
        reset_fill(&mut scratch.is_trap_blk, n_blocks, false);
        reset_fill(&mut scratch.has_call_blk, n_blocks, false);
        reset_fill(&mut scratch.has_store_blk, n_blocks, false);
        reset_fill(&mut scratch.has_unchecked_mem_blk, n_blocks, false);
        for &b in &scratch.postorder {
            let bb = &ir.blocks[b as usize];
            let mut trap = matches!(bb.term, Term::Unreachable | Term::ThrowActive);
            let mut call = matches!(
                bb.term,
                Term::ReturnCall { .. }
                    | Term::ReturnCallIndirect { .. }
                    | Term::CallCatch { .. }
                    | Term::CallIndirectCatch { .. }
            );
            let mut store = false;
            let mut unchecked_mem = false;
            for inst in &bb.insts {
                match inst.op {
                    Op::Trap(_) | Op::MemGrow(_) | Op::MemGrowIdx { .. } => trap = true,
                    Op::Call { .. } | Op::CallIndirect { .. } => call = true,
                    Op::Store { checked, .. } | Op::V128StoreLane { checked, .. } => {
                        store = true;
                        if !checked {
                            unchecked_mem = true;
                        }
                    }
                    Op::StoreMem { .. } | Op::V128StoreLaneMem { .. } => {
                        store = true;
                        unchecked_mem = true;
                    }
                    Op::StoreScaled { .. } => {
                        store = true;
                    }
                    Op::Load { checked: false, .. }
                    | Op::LoadMem { .. }
                    | Op::V128LoadLane { checked: false, .. }
                    | Op::V128LoadLaneMem { .. }
                    | Op::BoundsCheck { .. }
                    | Op::MemCopy { .. }
                    | Op::MemCopyIdx { .. }
                    | Op::MemFill { .. }
                    | Op::MemFillIdx { .. }
                    | Op::MemCopyConst {
                        dst_checked: false, ..
                    }
                    | Op::MemCopyConst {
                        src_checked: false, ..
                    }
                    | Op::MemFillConst { checked: false, .. } => {
                        unchecked_mem = true;
                    }
                    Op::MemCopyConst { .. } | Op::MemFillConst { .. } => store = true,
                    _ => {}
                }
            }
            scratch.is_trap_blk[b as usize] = trap;
            scratch.has_call_blk[b as usize] = call;
            scratch.has_store_blk[b as usize] = store;
            scratch.has_unchecked_mem_blk[b as usize] = unchecked_mem;
        }
        // Backward propagation in reverse RPO: any non-entry block whose every successor is a
        // cold trap/unreachable block (e.g., error-formatting or abort-call helper blocks before
        // `unreachable`) is itself cold and sunk to the function tail.
        for &b in scratch.postorder.iter().rev() {
            let bu = b as usize;
            if b == 0 || scratch.is_trap_blk[bu] {
                continue;
            }
            let ss = scratch.succ_offs[bu] as usize;
            let se = scratch.succ_offs[bu + 1] as usize;
            if ss < se
                && scratch.succ_data[ss..se]
                    .iter()
                    .all(|&s| scratch.is_trap_blk[s as usize])
            {
                scratch.is_trap_blk[bu] = true;
            }
        }

        let succ_offs_ro = &scratch.succ_offs;
        let succ_data_ro = &scratch.succ_data;
        let is_trap_blk = &scratch.is_trap_blk;
        let has_call_blk = &scratch.has_call_blk;
        let has_store_blk = &scratch.has_store_blk;
        let has_unchecked_mem_blk = &scratch.has_unchecked_mem_blk;
        let cand_score = |cur: u32, cand: u32, is_then: bool, placed: &[bool]| -> i32 {
            let mut s = 0i32;
            if is_trap_blk[cand as usize] {
                s += 1000;
            }
            if is_then
                && let Term::CmpBranch { kind, a, b, .. } = ir.blocks[cur as usize].term
                && matches!(
                    kind,
                    super::ir::CmpBranchKind::F32(_) | super::ir::CmpBranchKind::F64(_)
                )
                && a != b
            {
                s += 500;
            }
            let cbb = &ir.blocks[cand as usize];
            let ss = succ_offs_ro[cand as usize] as usize;
            let se = succ_offs_ro[cand as usize + 1] as usize;
            let backedges_to_placed_hdr = succ_data_ro[ss..se]
                .iter()
                .any(|&x| placed[x as usize] && ir.blocks[x as usize].is_loop_header);
            if cbb.is_loop_header || backedges_to_placed_hdr {
                s -= 100;
            }
            if has_call_blk[cand as usize] {
                s += 40;
            }
            if has_store_blk[cand as usize] {
                s += 20;
            }
            if !cbb.is_loop_header && cbb.insts.len() > 4 {
                s += 10;
            }
            let mut unchecked = has_unchecked_mem_blk[cand as usize];
            if !unchecked
                && cbb.insts.is_empty()
                && let Term::CmpBranch { then_bb, .. } | Term::BranchIf { then_bb, .. } = cbb.term
            {
                unchecked = has_unchecked_mem_blk[then_bb.0 as usize];
            }
            if unchecked {
                s += 15;
            }
            s
        };

        let mut block_order = Vec::with_capacity(n_ord);
        reset_fill(&mut scratch.placed, n_blocks, false);
        let placed = &mut scratch.placed;
        let rpo_pos_ro = &scratch.rpo_pos;
        // Pass 1: place all hot / non-trap blocks in contiguous traces (sinking cold trap/abort
        // blocks out of hot loops so they neither pollute I-cache nor split/clobber hot ranges).
        // Pass 2: append remaining cold trap/unreachable blocks at the tail of the function.
        for pass in 0..2 {
            for &seed in &scratch.postorder {
                if placed[seed as usize] {
                    continue;
                }
                if pass == 0 && seed != 0 && is_trap_blk[seed as usize] {
                    continue;
                }
                let mut cur = seed;
                loop {
                    placed[cur as usize] = true;
                    block_order.push(cur);
                    let next = match ir.blocks[cur as usize].term {
                        Term::Jump { target, .. } => (!placed[target.0 as usize]
                            && (pass == 1 || !is_trap_blk[target.0 as usize]))
                            .then_some(target.0),
                        Term::BranchIf {
                            then_bb, else_bb, ..
                        }
                        | Term::CmpBranch {
                            then_bb, else_bb, ..
                        } => {
                            let t_ok = !placed[then_bb.0 as usize];
                            let e_ok = !placed[else_bb.0 as usize] && else_bb != then_bb;
                            match (t_ok, e_ok) {
                                (true, true) => {
                                    let mut ts = cand_score(cur, then_bb.0, true, placed);
                                    let mut es = cand_score(cur, else_bb.0, false, placed);
                                    // Prefer linear fallthrough on triangle `if` diamonds
                                    // (`cur -> arm -> merge`, e.g. `if right < end { ... }`) so
                                    // the single-pred arm falls straight into `merge` without a
                                    // forward jump + unconditional jump back!
                                    let is_tri = |arm: u32, merge: u32| -> bool {
                                        !is_trap_blk[arm as usize]
                                            && !has_call_blk[arm as usize]
                                            && !ir.blocks[arm as usize].is_loop_header
                                            && ir.blocks[arm as usize].preds == 1
                                            && matches!(
                                                ir.blocks[arm as usize].term,
                                                Term::Jump { target, .. } if target.0 == merge
                                            )
                                    };
                                    if is_tri(then_bb.0, else_bb.0) {
                                        ts -= 150;
                                    } else if is_tri(else_bb.0, then_bb.0) {
                                        es -= 150;
                                    }
                                    let best = if (ts, rpo_pos_ro[then_bb.0 as usize])
                                        <= (es, rpo_pos_ro[else_bb.0 as usize])
                                    {
                                        then_bb.0
                                    } else {
                                        else_bb.0
                                    };
                                    (pass == 1 || !is_trap_blk[best as usize]).then_some(best)
                                }
                                (true, false) => (pass == 1 || !is_trap_blk[then_bb.0 as usize])
                                    .then_some(then_bb.0),
                                (false, true) => (pass == 1 || !is_trap_blk[else_bb.0 as usize])
                                    .then_some(else_bb.0),
                                (false, false) => None,
                            }
                        }
                        Term::CallCatch { ok_bb, .. } | Term::CallIndirectCatch { ok_bb, .. } => {
                            (!placed[ok_bb.0 as usize]
                                && (pass == 1 || !is_trap_blk[ok_bb.0 as usize]))
                                .then_some(ok_bb.0)
                        }
                        Term::BrTable { .. }
                        | Term::Return { .. }
                        | Term::ReturnCall { .. }
                        | Term::ReturnCallIndirect { .. }
                        | Term::ThrowActive
                        | Term::Unreachable => None,
                    };
                    match next {
                        Some(nxt) => cur = nxt,
                        None => break,
                    }
                }
            }
        }
        reset_fill(&mut scratch.block_pos_in_order, n_blocks, u32::MAX);
        for (oi, &b) in block_order.iter().enumerate() {
            scratch.block_pos_in_order[b as usize] = oi as u32;
        }
        block_order
    };
    let n_ord = scratch.postorder.len();

    // 2. Assign linear program positions (2 ticks per instruction: even=use, odd=def).
    reset_fill(&mut scratch.block_start, n_ord, 0u32);
    reset_fill(&mut scratch.block_end, n_ord, 0u32);
    let block_start = &mut scratch.block_start;
    let block_end = &mut scratch.block_end;
    reset_fill(&mut scratch.def_pos, n_vals, u32::MAX);
    reset_fill(&mut scratch.def_block, n_vals, u32::MAX);
    reset_fill(&mut scratch.last_use, n_vals, 0u32);
    reset_fill(&mut scratch.cross_block, n_vals, false);
    reset_fill(&mut scratch.val_remat, n_vals, None);
    scratch.clobbers.clear();
    let def_pos = &mut scratch.def_pos;
    let def_block = &mut scratch.def_block;
    let last_use = &mut scratch.last_use;
    let cross_block = &mut scratch.cross_block;
    let val_remat = &mut scratch.val_remat;
    let clobbers = &mut scratch.clobbers;
    let mut max_outgoing_arg_bytes = 0i32;
    let mut max_ret_buf_bytes = 0i32;
    let self_param_stack_bytes =
        layout_args_stack_bytes(ir.params.iter().map(|&pv| ir.val_tys[pv.0 as usize]))?;

    let mut tick = 0u32;
    let mut scan_steps: usize = 0;
    for (oi, &bid) in block_order.iter().enumerate() {
        let bb = &ir.blocks[bid as usize];
        block_start[oi] = tick;
        for &p in &bb.params {
            let vi = p.0 as usize;
            def_pos[vi] = tick;
            def_block[vi] = oi as u32;
            // Ensure all params of the same block overlap on `[tick, tick + 1]` so they get distinct locations.
            if last_use[vi] < tick + 1 {
                last_use[vi] = tick + 1;
            }
        }
        tick += 2;
        for inst in &bb.insts {
            scan_steps += 1;
            if let Some(bg) = budget
                && (scan_steps & 4095) == 0
            {
                bg.check_deadline()?;
            }
            let pos_use = tick;
            let pos_def = tick + 1;
            inst.op.for_each_use(&ir.val_pool, |v| {
                let vi = v.0 as usize;
                if def_block[vi] != oi as u32 {
                    cross_block[vi] = true;
                }
                if pos_use > last_use[vi] {
                    last_use[vi] = pos_use;
                }
            });
            match inst.op {
                Op::Call { args, rets, .. } | Op::CallIndirect { args, rets, .. } => {
                    clobbers.push(Clobber {
                        pos: pos_use,
                        gp_mask: MASK_GP_CALLER,
                        xmm_mask: MASK_XMM_CALLER,
                    });
                    let arg_stack_bytes = layout_args_stack_bytes(
                        ir.pool_slice(args)
                            .iter()
                            .map(|&av| ir.val_tys[av.0 as usize]),
                    )?;
                    if arg_stack_bytes > max_outgoing_arg_bytes {
                        max_outgoing_arg_bytes = arg_stack_bytes;
                    }
                    let ret_slice = ir.pool_slice(rets);
                    for &rv in ret_slice {
                        let vi = rv.0 as usize;
                        def_pos[vi] = pos_def;
                        def_block[vi] = oi as u32;
                        if last_use[vi] < pos_def + 1 {
                            last_use[vi] = pos_def + 1;
                        }
                    }
                    let ret_stack_bytes = layout_rets_stack_bytes(
                        ret_slice.iter().map(|&rv| ir.val_tys[rv.0 as usize]),
                    )?;
                    if ret_stack_bytes > max_ret_buf_bytes {
                        max_ret_buf_bytes = ret_stack_bytes;
                    }
                }
                Op::MemCopy { .. } | Op::MemCopyIdx { .. } => {
                    let (gp_mask, xmm_mask) = helper_clobbers(HelperKind::MemCopy);
                    clobbers.push(Clobber {
                        pos: pos_use,
                        gp_mask,
                        xmm_mask,
                    });
                }
                Op::MemFill { .. } | Op::MemFillIdx { .. } => {
                    let (gp_mask, xmm_mask) = helper_clobbers(HelperKind::MemFill);
                    clobbers.push(Clobber {
                        pos: pos_use,
                        gp_mask,
                        xmm_mask,
                    });
                }
                Op::MemGrow(_) | Op::MemGrowIdx { .. } => {
                    let (gp_mask, xmm_mask) = helper_clobbers(HelperKind::MemGrow);
                    clobbers.push(Clobber {
                        pos: pos_use,
                        gp_mask,
                        xmm_mask,
                    });
                }
                Op::Trunc(_, _) => {
                    let (gp_mask, xmm_mask) = helper_clobbers(HelperKind::Trunc);
                    clobbers.push(Clobber {
                        pos: pos_use,
                        gp_mask,
                        xmm_mask,
                    });
                }
                Op::I32Bin(op, _, _) | Op::I64Bin(op, _, _) => match op {
                    IntBinOp::DivS
                    | IntBinOp::DivU
                    | IntBinOp::DivUNz
                    | IntBinOp::RemS
                    | IntBinOp::RemU
                    | IntBinOp::RemUNz
                    | IntBinOp::MulHiS
                    | IntBinOp::MulHiU => {
                        clobbers.push(Clobber {
                            pos: pos_use,
                            gp_mask: MASK_RAX_RDX,
                            xmm_mask: 0,
                        });
                    }
                    IntBinOp::Shl | IntBinOp::ShrS | IntBinOp::ShrU => {
                        if !cfg.has_bmi2 {
                            clobbers.push(Clobber {
                                pos: pos_use,
                                gp_mask: MASK_RCX,
                                xmm_mask: 0,
                            });
                        }
                    }
                    IntBinOp::Rotl | IntBinOp::Rotr => {
                        clobbers.push(Clobber {
                            pos: pos_use,
                            gp_mask: MASK_RCX,
                            xmm_mask: 0,
                        });
                    }
                    _ => {}
                },
                Op::I64BinImm(
                    op @ (IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU),
                    _,
                    imm,
                ) => {
                    let is_pow2 = if matches!(op, IntBinOp::DivU | IntBinOp::RemU) {
                        u64::from(imm as u32).is_power_of_two()
                    } else {
                        i64::from(imm).unsigned_abs().is_power_of_two()
                    };
                    if !is_pow2 {
                        clobbers.push(Clobber {
                            pos: pos_use,
                            gp_mask: MASK_RAX_RDX,
                            xmm_mask: 0,
                        });
                    }
                }
                Op::F64AxpyLoop { .. } => {
                    clobbers.push(Clobber {
                        pos: pos_use,
                        gp_mask: MASK_RAX_RDX | MASK_RCX,
                        xmm_mask: (1 << 0) | (1 << 1) | (1 << 2),
                    });
                }
                _ => {}
            }
            if let Some(res) = inst.res {
                let vi = res.0 as usize;
                def_pos[vi] = pos_def;
                def_block[vi] = oi as u32;
                if last_use[vi] < pos_def {
                    last_use[vi] = pos_def;
                }
                val_remat[vi] = match inst.op {
                    Op::ConstI32(c) => Some(RematConst::I32(c)),
                    Op::ConstI64(c) => Some(RematConst::I64(c)),
                    Op::ConstF32(b) => Some(RematConst::F32(b)),
                    Op::ConstF64(b) => Some(RematConst::F64(b)),
                    _ => None,
                };
            }
            tick += 2;
        }
        let pos_term = tick;
        match bb.term {
            Term::CallCatch {
                args,
                ok_bb,
                catch_args,
                ..
            } => {
                for &v in ir.pool_slice(args) {
                    let vi = v.0 as usize;
                    if def_block[vi] != oi as u32 {
                        cross_block[vi] = true;
                    }
                    if pos_term > last_use[vi] {
                        last_use[vi] = pos_term;
                    }
                }
                for &v in ir.pool_slice(catch_args) {
                    let vi = v.0 as usize;
                    if def_block[vi] != oi as u32 {
                        cross_block[vi] = true;
                    }
                    if pos_term + 1 > last_use[vi] {
                        last_use[vi] = pos_term + 1;
                    }
                }
                clobbers.push(Clobber {
                    pos: pos_term,
                    gp_mask: MASK_GP_CALLER,
                    xmm_mask: MASK_XMM_CALLER,
                });
                let arg_stack_bytes = layout_args_stack_bytes(
                    ir.pool_slice(args)
                        .iter()
                        .map(|&av| ir.val_tys[av.0 as usize]),
                )?;
                if arg_stack_bytes > max_outgoing_arg_bytes {
                    max_outgoing_arg_bytes = arg_stack_bytes;
                }
                let ret_stack_bytes = layout_rets_stack_bytes(
                    ir.blocks[ok_bb.0 as usize]
                        .params
                        .iter()
                        .map(|&rv| ir.val_tys[rv.0 as usize]),
                )?;
                if ret_stack_bytes > max_ret_buf_bytes {
                    max_ret_buf_bytes = ret_stack_bytes;
                }
            }
            Term::CallIndirectCatch {
                idx,
                args,
                ok_bb,
                catch_args,
                ..
            } => {
                {
                    let vi = idx.0 as usize;
                    if def_block[vi] != oi as u32 {
                        cross_block[vi] = true;
                    }
                    if pos_term > last_use[vi] {
                        last_use[vi] = pos_term;
                    }
                }
                for &v in ir.pool_slice(args) {
                    let vi = v.0 as usize;
                    if def_block[vi] != oi as u32 {
                        cross_block[vi] = true;
                    }
                    if pos_term > last_use[vi] {
                        last_use[vi] = pos_term;
                    }
                }
                for &v in ir.pool_slice(catch_args) {
                    let vi = v.0 as usize;
                    if def_block[vi] != oi as u32 {
                        cross_block[vi] = true;
                    }
                    if pos_term + 1 > last_use[vi] {
                        last_use[vi] = pos_term + 1;
                    }
                }
                clobbers.push(Clobber {
                    pos: pos_term,
                    gp_mask: MASK_GP_CALLER,
                    xmm_mask: MASK_XMM_CALLER,
                });
                let arg_stack_bytes = layout_args_stack_bytes(
                    ir.pool_slice(args)
                        .iter()
                        .map(|&av| ir.val_tys[av.0 as usize]),
                )?;
                if arg_stack_bytes > max_outgoing_arg_bytes {
                    max_outgoing_arg_bytes = arg_stack_bytes;
                }
                let ret_stack_bytes = layout_rets_stack_bytes(
                    ir.blocks[ok_bb.0 as usize]
                        .params
                        .iter()
                        .map(|&rv| ir.val_tys[rv.0 as usize]),
                )?;
                if ret_stack_bytes > max_ret_buf_bytes {
                    max_ret_buf_bytes = ret_stack_bytes;
                }
            }
            Term::ReturnCall { args, .. } | Term::ReturnCallIndirect { args, .. } => {
                bb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
                    let vi = v.0 as usize;
                    if def_block[vi] != oi as u32 {
                        cross_block[vi] = true;
                    }
                    if pos_term > last_use[vi] {
                        last_use[vi] = pos_term;
                    }
                });
                let arg_stack_bytes = layout_args_stack_bytes(
                    ir.pool_slice(args)
                        .iter()
                        .map(|&av| ir.val_tys[av.0 as usize]),
                )?;
                if arg_stack_bytes > self_param_stack_bytes {
                    clobbers.push(Clobber {
                        pos: pos_term,
                        gp_mask: MASK_GP_CALLER,
                        xmm_mask: MASK_XMM_CALLER,
                    });
                    if arg_stack_bytes > max_outgoing_arg_bytes {
                        max_outgoing_arg_bytes = arg_stack_bytes;
                    }
                }
            }
            _ => {
                bb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
                    let vi = v.0 as usize;
                    if def_block[vi] != oi as u32 {
                        cross_block[vi] = true;
                    }
                    if pos_term > last_use[vi] {
                        last_use[vi] = pos_term;
                    }
                });
            }
        }
        block_end[oi] = if matches!(
            bb.term,
            Term::CallCatch { .. } | Term::CallIndirectCatch { .. }
        ) {
            pos_term + 2
        } else {
            pos_term + 1
        };
        tick += 2;
    }

    // 3. Exact cross-block liveness ONLY for values with `cross_block[v] == true`.
    reset_fill(&mut scratch.cb_id, n_vals, u32::MAX);
    scratch.cb_vals.clear();
    let cb_id = &mut scratch.cb_id;
    let cb_vals = &mut scratch.cb_vals;
    for vi in 0..n_vals {
        if cross_block[vi] && def_pos[vi] != u32::MAX {
            cb_id[vi] = cb_vals.len() as u32;
            cb_vals.push(vi as u32);
        }
    }
    let n_cb = cb_vals.len();
    let rpo = &scratch.postorder;
    let rpo_pos = &scratch.rpo_pos;
    let block_pos_in_order = &scratch.block_pos_in_order;
    let succ_offs = &scratch.succ_offs;
    let succ_data = &scratch.succ_data;
    let has_backedge = n_cb > 0
        && rpo.iter().enumerate().any(|(ri, &bid)| {
            let ss = succ_offs[bid as usize] as usize;
            let se = succ_offs[bid as usize + 1] as usize;
            succ_data[ss..se].iter().any(|&sid| {
                let sri = rpo_pos[sid as usize];
                sri != u32::MAX && sri <= ri as u32
            })
        });
    const MAX_SEGS_PER_VAL: usize = 64;
    const MAX_SEG_SLOTS_PER_FN: usize = 65_536;
    const SCRATCH_RECYCLE_CAP: usize = 4096;
    // Declare `_liveness_guard` BEFORE `cb_*` and `segs_pool` so Rust's LIFO drop order
    // guarantees all liveness buffers drop BEFORE releasing the reservation even on early return.
    let mut _liveness_guard = None;
    let mut cb_seg_first: Vec<(u32, u32)> = Vec::new();
    let mut cb_seg_extra_idx: Vec<u32> = Vec::new();
    let mut cb_extra_vecs: Vec<Vec<(u32, u32)>> = Vec::new();
    let mut cb_seg_overflow: Vec<bool> = Vec::new();
    let mut saved_live_in: Vec<u64> = Vec::new();
    let mut saved_live_words: usize = 0;
    let mut has_exact_segs = false;
    if n_cb > 0 {
        let words = n_cb.div_ceil(64);
        let matrix_words = n_ord
            .checked_mul(words)
            .ok_or("x64: liveness table size overflow")?;
        let matrix_bytes = matrix_words
            .checked_mul(4 * 8)
            .ok_or("x64: liveness table size overflow")?;
        // Account for BOTH `cb_hull` (8 B/val) and `last_use_in_blk` scratch (8 B/val).
        let hull_and_scratch_bytes = n_cb
            .checked_mul(2 * std::mem::size_of::<(u32, u32)>())
            .ok_or("x64: liveness hull size overflow")?;
        let base_bytes = matrix_bytes
            .checked_add(hull_and_scratch_bytes)
            .ok_or("x64: liveness memory size overflow")?;
        if base_bytes > MAX_LIVENESS_BYTES {
            return Err(format!(
                "x64: function {} liveness matrix ({base_bytes} B) exceeds cap {MAX_LIVENESS_BYTES} B",
                ir.func_index
            ));
        }
        // Budget exact live-range hole storage strictly inside the remaining `MAX_LIVENESS_BYTES`
        // headroom (at most 2 MiB) so parallel workers can never exceed the documented 16 MiB/fn
        // or 256 MiB aggregate liveness caps. First segment lives inline in `cb_seg_first` (zero
        // per-value heap allocation for contiguous live ranges!); only multi-segment values with
        // real live-range holes allocate an entry in `cb_extra_vecs`.
        // For `n_ord <= 2`, disjoint multi-block holes cannot exist, so skip exact segments completely!
        let flat_hdr_bytes =
            n_cb.saturating_mul(std::mem::size_of::<(u32, u32)>() + std::mem::size_of::<u32>() + 1);
        let remaining_cap = MAX_LIVENESS_BYTES.saturating_sub(base_bytes).min(2 << 20);
        let max_cap_per_val = n_ord.min(MAX_SEGS_PER_VAL).next_power_of_two().max(4);
        let seg_cap_slots = if n_ord > 2 && remaining_cap > flat_hdr_bytes {
            // 24 B per slot conservatively covers 8 B in `cb_extra_vecs` + Vec header share + 8 B in `segs_pool`.
            ((remaining_cap - flat_hdr_bytes) / 24)
                .min(n_cb.saturating_mul(max_cap_per_val))
                .min(MAX_SEG_SLOTS_PER_FN)
        } else {
            0
        };
        let seg_reserve_bytes = if seg_cap_slots > 0 {
            flat_hdr_bytes.saturating_add(seg_cap_slots.saturating_mul(24))
        } else {
            0
        };
        let total_liveness_bytes = base_bytes.saturating_add(seg_reserve_bytes);
        _liveness_guard = match budget {
            Some(b) => Some(b.reserve_liveness(total_liveness_bytes)?),
            None => None,
        };
        let mut live_use = std::mem::take(&mut scratch.live_use);
        let mut live_def = std::mem::take(&mut scratch.live_def);
        let mut live_in = std::mem::take(&mut scratch.live_in);
        let mut live_out = std::mem::take(&mut scratch.live_out);
        live_use.clear();
        live_use.resize(matrix_words, 0u64);
        live_def.clear();
        live_def.resize(matrix_words, 0u64);
        live_in.clear();
        live_in.resize(matrix_words, 0u64);
        live_out.clear();
        live_out.resize(matrix_words, 0u64);

        let set_bit = |buf: &mut [u64], oi: usize, bit: usize| {
            buf[oi * words + (bit >> 6)] |= 1u64 << (bit & 63);
        };
        let has_bit = |buf: &[u64], oi: usize, bit: usize| -> bool {
            (buf[oi * words + (bit >> 6)] & (1u64 << (bit & 63))) != 0
        };

        let mut def_use_steps: usize = 0;
        for (oi, &bid) in block_order.iter().enumerate() {
            let bb = &ir.blocks[bid as usize];
            for &p in &bb.params {
                let c = cb_id[p.0 as usize];
                if c != u32::MAX {
                    set_bit(&mut live_def, oi, c as usize);
                }
            }
            for inst in &bb.insts {
                def_use_steps += 1;
                if let Some(bg) = budget
                    && (def_use_steps & 4095) == 0
                {
                    bg.check_deadline()?;
                }
                inst.op.for_each_use(&ir.val_pool, |v| {
                    let c = cb_id[v.0 as usize];
                    if c != u32::MAX && !has_bit(&live_def, oi, c as usize) {
                        set_bit(&mut live_use, oi, c as usize);
                    }
                });
                if let Some(r) = inst.res {
                    let c = cb_id[r.0 as usize];
                    if c != u32::MAX {
                        set_bit(&mut live_def, oi, c as usize);
                    }
                }
                match inst.op {
                    Op::Call { rets, .. } | Op::CallIndirect { rets, .. } => {
                        for &r in ir.pool_slice(rets) {
                            let c = cb_id[r.0 as usize];
                            if c != u32::MAX {
                                set_bit(&mut live_def, oi, c as usize);
                            }
                        }
                    }
                    _ => {}
                }
            }
            bb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
                let c = cb_id[v.0 as usize];
                if c != u32::MAX && !has_bit(&live_def, oi, c as usize) {
                    set_bit(&mut live_use, oi, c as usize);
                }
            });
        }

        let mut work_units: usize = 0;
        let mut changed = true;
        // Walk blocks in reverse RPO (exact topological order on acyclic CFGs):
        // DAGs converge in 1 single pass with zero verification pass, and cyclic CFGs converge in
        // minimum loop-depth iterations while producing bit-identical `live_in`/`live_out`.
        while changed {
            if let Some(b) = budget {
                b.check_deadline()?;
            }
            changed = false;
            for &bid_u32 in rpo.iter().rev() {
                let bid = bid_u32 as usize;
                let oi = block_pos_in_order[bid] as usize;
                let ss = succ_offs[bid] as usize;
                let se = succ_offs[bid + 1] as usize;
                for &sid in &succ_data[ss..se] {
                    let soi = block_pos_in_order[sid as usize];
                    if soi == u32::MAX {
                        continue;
                    }
                    let soi = soi as usize;
                    work_units = work_units.saturating_add(words);
                    if work_units > MAX_LIVENESS_WORK_UNITS {
                        return Err(format!(
                            "x64: function {} liveness work ({work_units}) exceeds cap {MAX_LIVENESS_WORK_UNITS}",
                            ir.func_index
                        ));
                    }
                    for w in 0..words {
                        let old = live_out[oi * words + w];
                        let nv = old | live_in[soi * words + w];
                        if nv != old {
                            live_out[oi * words + w] = nv;
                            changed = true;
                        }
                    }
                }
                work_units = work_units.saturating_add(words);
                if work_units > MAX_LIVENESS_WORK_UNITS {
                    return Err(format!(
                        "x64: function {} liveness work ({work_units}) exceeds cap {MAX_LIVENESS_WORK_UNITS}",
                        ir.func_index
                    ));
                }
                for w in 0..words {
                    let nv = live_use[oi * words + w]
                        | (live_out[oi * words + w] & !live_def[oi * words + w]);
                    if nv != live_in[oi * words + w] {
                        live_in[oi * words + w] = nv;
                        changed = true;
                    }
                }
            }
            if !has_backedge {
                break;
            }
        }

        // Build exact sorted, merged per-block live-range segments within the budgeted `seg_cap_slots`
        // (preserving live-range holes while falling back any overflowing value to its exact hull).
        reset_fill(&mut scratch.cb_hull, n_cb, (u32::MAX, 0u32));
        if seg_cap_slots > 0 {
            has_exact_segs = true;
            cb_seg_first = std::mem::take(&mut scratch.cb_seg_first);
            cb_seg_first.clear();
            cb_seg_first.resize(n_cb, (u32::MAX, 0u32));
            cb_seg_extra_idx = std::mem::take(&mut scratch.cb_seg_extra_idx);
            cb_seg_extra_idx.clear();
            cb_seg_extra_idx.resize(n_cb, u32::MAX);
            cb_seg_overflow = std::mem::take(&mut scratch.cb_seg_overflow);
            cb_seg_overflow.clear();
            cb_seg_overflow.resize(n_cb, false);
        }
        let mut total_seg_cap_slots = 0usize;
        reset_fill(&mut scratch.last_use_in_blk, n_cb, (u32::MAX, 0u32));
        let cb_hull = &mut scratch.cb_hull;
        let last_use_in_blk = &mut scratch.last_use_in_blk;
        for (oi, &bid) in block_order.iter().enumerate() {
            if let Some(bg) = budget
                && (oi & 4095) == 4095
            {
                bg.check_deadline()?;
            }
            let oi_u32 = oi as u32;
            let bb = &ir.blocks[bid as usize];
            let bs = block_start[oi];
            let be = block_end[oi];
            let mut t = bs + 2;
            for inst in &bb.insts {
                inst.op.for_each_use(&ir.val_pool, |v| {
                    let c = cb_id[v.0 as usize];
                    if c != u32::MAX {
                        last_use_in_blk[c as usize] = (oi_u32, t);
                    }
                });
                t += 2;
            }
            let pos_term = t;
            match bb.term {
                Term::CallCatch {
                    args, catch_args, ..
                } => {
                    for &v in ir.pool_slice(args) {
                        let c = cb_id[v.0 as usize];
                        if c != u32::MAX {
                            last_use_in_blk[c as usize] = (oi_u32, pos_term);
                        }
                    }
                    for &v in ir.pool_slice(catch_args) {
                        let c = cb_id[v.0 as usize];
                        if c != u32::MAX {
                            last_use_in_blk[c as usize] = (oi_u32, pos_term + 1);
                        }
                    }
                }
                Term::CallIndirectCatch {
                    idx,
                    args,
                    catch_args,
                    ..
                } => {
                    let ci = cb_id[idx.0 as usize];
                    if ci != u32::MAX {
                        last_use_in_blk[ci as usize] = (oi_u32, pos_term);
                    }
                    for &v in ir.pool_slice(args) {
                        let c = cb_id[v.0 as usize];
                        if c != u32::MAX {
                            last_use_in_blk[c as usize] = (oi_u32, pos_term);
                        }
                    }
                    for &v in ir.pool_slice(catch_args) {
                        let c = cb_id[v.0 as usize];
                        if c != u32::MAX {
                            last_use_in_blk[c as usize] = (oi_u32, pos_term + 1);
                        }
                    }
                }
                _ => {
                    bb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
                        let c = cb_id[v.0 as usize];
                        if c != u32::MAX {
                            last_use_in_blk[c as usize] = (oi_u32, pos_term);
                        }
                    });
                }
            }
            for w in 0..words {
                let mut bits =
                    live_in[oi * words + w] | live_out[oi * words + w] | live_def[oi * words + w];
                while bits != 0 {
                    let tz = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let c = (w << 6) + tz;
                    let vi = cb_vals[c] as usize;
                    let s = if def_block[vi] == oi_u32 {
                        def_pos[vi]
                    } else {
                        bs
                    };
                    let is_out = (live_out[oi * words + w] & (1u64 << tz)) != 0;
                    let e = if is_out {
                        be
                    } else if last_use_in_blk[c].0 == oi_u32 {
                        last_use_in_blk[c].1.max(s + 1)
                    } else {
                        s + 1
                    };
                    if s < cb_hull[c].0 {
                        cb_hull[c].0 = s;
                    }
                    if e > cb_hull[c].1 {
                        cb_hull[c].1 = e;
                    }
                    if !has_exact_segs || cb_seg_overflow[c] {
                        continue;
                    }
                    let first = &mut cb_seg_first[c];
                    if first.0 == u32::MAX {
                        *first = (s, e);
                    } else if cb_seg_extra_idx[c] == u32::MAX {
                        if s <= first.1 + 1 {
                            if e > first.1 {
                                first.1 = e;
                            }
                        } else if 4 > MAX_SEGS_PER_VAL
                            || total_seg_cap_slots.saturating_add(4) > seg_cap_slots
                        {
                            cb_seg_overflow[c] = true;
                        } else {
                            let mut v = Vec::new();
                            v.reserve_exact(4);
                            v.push(*first);
                            v.push((s, e));
                            total_seg_cap_slots += 4;
                            cb_seg_extra_idx[c] = cb_extra_vecs.len() as u32;
                            cb_extra_vecs.push(v);
                        }
                    } else {
                        let segs = &mut cb_extra_vecs[cb_seg_extra_idx[c] as usize];
                        let last = segs.last_mut().unwrap();
                        if s <= last.1 + 1 {
                            if e > last.1 {
                                last.1 = e;
                            }
                        } else {
                            if segs.len() == segs.capacity() {
                                let next_cap = segs.capacity().saturating_mul(2);
                                let add_slots = next_cap.saturating_sub(segs.capacity());
                                if next_cap > MAX_SEGS_PER_VAL
                                    || total_seg_cap_slots.saturating_add(add_slots) > seg_cap_slots
                                {
                                    cb_seg_overflow[c] = true;
                                    total_seg_cap_slots =
                                        total_seg_cap_slots.saturating_sub(segs.capacity());
                                    segs.clear();
                                    segs.shrink_to_fit();
                                    continue;
                                }
                                segs.reserve_exact(add_slots);
                                total_seg_cap_slots += add_slots;
                            }
                            segs.push((s, e));
                        }
                    }
                }
            }
        }
        // Recycle small scratch matrices immediately; oversized ones drop here before `_liveness_guard`.
        if live_use.capacity() <= SCRATCH_RECYCLE_CAP {
            scratch.live_use = live_use;
        }
        if live_def.capacity() <= SCRATCH_RECYCLE_CAP {
            scratch.live_def = live_def;
        }
        if live_out.capacity() <= SCRATCH_RECYCLE_CAP {
            scratch.live_out = live_out;
        }
        saved_live_in = live_in;
        saved_live_words = words;
    }

    // 4. Build O(log C) segment tree over clobbers and build exact intervals.
    let n_clob = clobbers.len();
    reset_fill(&mut scratch.clob_seg, n_clob * 2, (0u16, 0u16));
    let clob_seg = &mut scratch.clob_seg;
    for (i, c) in clobbers.iter().enumerate() {
        clob_seg[n_clob + i] = (c.gp_mask, c.xmm_mask);
    }
    for i in (1..n_clob).rev() {
        let a = clob_seg[i << 1];
        let b = clob_seg[(i << 1) | 1];
        clob_seg[i] = (a.0 | b.0, a.1 | b.1);
    }
    let query_clobbers = |mut l: usize, mut r: usize| -> (u16, u16) {
        if l >= r || n_clob == 0 {
            return (0, 0);
        }
        l += n_clob;
        r += n_clob;
        let (mut gp, mut xmm) = (0u16, 0u16);
        while l < r {
            if (l & 1) != 0 {
                gp |= clob_seg[l].0;
                xmm |= clob_seg[l].1;
                l += 1;
            }
            if (r & 1) != 0 {
                r -= 1;
                gp |= clob_seg[r].0;
                xmm |= clob_seg[r].1;
            }
            l >>= 1;
            r >>= 1;
        }
        (gp, xmm)
    };

    // 4b. Compute loop-depth weighting (strictly in RPO so hot-fallthrough block layout
    // never turns a cold rejoin edge into a false backedge) and ABI / phi / two-address hints.
    // Only build flat CSR `rpo_preds` when backedges actually exist!
    reset_fill(&mut scratch.loop_depth, n_ord, 0u8);
    if n_ord > 1 || scratch.succ_offs[1] > 0 {
        scratch.backedges.clear();
        for (ri, &bid) in scratch.postorder.iter().enumerate() {
            let ss = scratch.succ_offs[bid as usize] as usize;
            let se = scratch.succ_offs[bid as usize + 1] as usize;
            for &sid in &scratch.succ_data[ss..se] {
                let sri = scratch.rpo_pos[sid as usize];
                if sri != u32::MAX && sri <= ri as u32 {
                    scratch.backedges.push((ri as u32, sri));
                }
            }
        }
        if !scratch.backedges.is_empty() {
            reset_fill(&mut scratch.rpo_pred_counts, n_ord, 0u32);
            let mut total_rp = 0usize;
            for &bid in &scratch.postorder {
                let ss = scratch.succ_offs[bid as usize] as usize;
                let se = scratch.succ_offs[bid as usize + 1] as usize;
                for &sid in &scratch.succ_data[ss..se] {
                    let sri = scratch.rpo_pos[sid as usize];
                    if sri != u32::MAX {
                        scratch.rpo_pred_counts[sri as usize] += 1;
                        total_rp += 1;
                    }
                }
            }
            scratch.rpo_pred_offs.clear();
            scratch.rpo_pred_offs.reserve(n_ord + 1);
            let mut off = 0u32;
            for ri in 0..n_ord {
                scratch.rpo_pred_offs.push(off);
                off += scratch.rpo_pred_counts[ri];
                scratch.rpo_pred_counts[ri] = scratch.rpo_pred_offs[ri];
            }
            scratch.rpo_pred_offs.push(off);
            scratch.rpo_pred_data.clear();
            scratch.rpo_pred_data.resize(total_rp, 0u32);
            for (ri, &bid) in scratch.postorder.iter().enumerate() {
                let ss = scratch.succ_offs[bid as usize] as usize;
                let se = scratch.succ_offs[bid as usize + 1] as usize;
                for &sid in &scratch.succ_data[ss..se] {
                    let sri = scratch.rpo_pos[sid as usize];
                    if sri != u32::MAX {
                        let pos = scratch.rpo_pred_counts[sri as usize] as usize;
                        scratch.rpo_pred_counts[sri as usize] = (pos + 1) as u32;
                        scratch.rpo_pred_data[pos] = ri as u32;
                    }
                }
            }
            reset_fill(&mut scratch.rpo_depth, n_ord, 0u8);
            reset_fill(&mut scratch.loop_seen, n_ord, u32::MAX);
            for (li, &(latch, header)) in scratch.backedges.iter().take(512).enumerate() {
                let stamp = li as u32;
                scratch.loop_seen[header as usize] = stamp;
                scratch.rpo_depth[header as usize] =
                    scratch.rpo_depth[header as usize].saturating_add(1).min(4);
                if latch != header {
                    scratch.loop_seen[latch as usize] = stamp;
                    scratch.rpo_depth[latch as usize] =
                        scratch.rpo_depth[latch as usize].saturating_add(1).min(4);
                    scratch.loop_wl.clear();
                    scratch.loop_wl.push(latch);
                    while let Some(cur) = scratch.loop_wl.pop() {
                        let ps = scratch.rpo_pred_offs[cur as usize] as usize;
                        let pe = scratch.rpo_pred_offs[cur as usize + 1] as usize;
                        for &p in &scratch.rpo_pred_data[ps..pe] {
                            if p > header && scratch.loop_seen[p as usize] != stamp {
                                scratch.loop_seen[p as usize] = stamp;
                                scratch.rpo_depth[p as usize] =
                                    scratch.rpo_depth[p as usize].saturating_add(1).min(4);
                                scratch.loop_wl.push(p);
                            }
                        }
                    }
                }
            }
            for (ri, &bid) in scratch.postorder.iter().enumerate() {
                let oi = scratch.block_pos_in_order[bid as usize] as usize;
                scratch.loop_depth[oi] = scratch.rpo_depth[ri];
            }
        }
    }
    let loop_depth = &scratch.loop_depth;

    reset_fill(&mut scratch.val_weight, n_vals, 0u32);
    reset_fill(&mut scratch.max_use_loop_depth, n_vals, 0u8);
    reset_fill(&mut scratch.hint_gp, n_vals, None);
    reset_fill(&mut scratch.hint_xmm, n_vals, None);
    reset_fill(&mut scratch.hint_peers, n_vals, [u32::MAX; 4]);
    reset_fill(&mut scratch.first_edge_arg, n_vals, u32::MAX);
    let val_weight = &mut scratch.val_weight;
    let max_use_loop_depth = &mut scratch.max_use_loop_depth;
    let hint_gp = &mut scratch.hint_gp;
    let hint_xmm = &mut scratch.hint_xmm;
    let hint_peers = &mut scratch.hint_peers;
    let first_edge_arg = &mut scratch.first_edge_arg;
    let add_peer = |peers: &mut Vec<[u32; 4]>, a: u32, b: u32| {
        if a == b {
            return;
        }
        let slot = &mut peers[a as usize];
        for entry in slot.iter_mut() {
            if *entry == b {
                return;
            }
            if *entry == u32::MAX {
                *entry = b;
                return;
            }
        }
    };

    // Hint entry params with their incoming ABI registers (zero heap allocation).
    if !ir.params.is_empty() {
        let _ = for_each_arg_slot(
            ir.params.iter().map(|&pv| ir.val_tys[pv.0 as usize]),
            |idx, slot| {
                let pv = ir.params[idx];
                match slot {
                    AbiSlot::Gp(g) => hint_gp[pv.0 as usize] = Some(g),
                    AbiSlot::Xmm(x) => hint_xmm[pv.0 as usize] = Some(x),
                    AbiSlot::Stack(_) => {}
                }
            },
        );
    }

    let mut hint_steps: usize = 0;
    let is_trap_blk_ref = &scratch.is_trap_blk;
    reset_fill(&mut scratch.val_non_trap, n_vals, false);
    let val_non_trap = &mut scratch.val_non_trap;
    for (oi, &bid) in block_order.iter().enumerate() {
        let bb = &ir.blocks[bid as usize];
        let is_cold_trap =
            n_blocks > 1 && is_trap_blk_ref.get(bid as usize).copied().unwrap_or(false);
        let w = if is_cold_trap {
            1u32
        } else {
            match loop_depth[oi] {
                0 => 4u32,
                1 => 64,
                2 => 1024,
                3 => 16_384,
                _ => 131_072,
            }
        };
        for &p in &bb.params {
            val_weight[p.0 as usize] = val_weight[p.0 as usize].saturating_add(w);
            if !is_cold_trap {
                val_non_trap[p.0 as usize] = true;
            }
        }
        for inst in &bb.insts {
            hint_steps += 1;
            if let Some(bg) = budget
                && (hint_steps & 4095) == 0
            {
                bg.check_deadline()?;
            }
            let cur_ld = loop_depth[oi];
            inst.op.for_each_use(&ir.val_pool, |v| {
                let vu = v.0 as usize;
                val_weight[vu] = val_weight[vu].saturating_add(w);
                if cur_ld > max_use_loop_depth[vu] {
                    max_use_loop_depth[vu] = cur_ld;
                }
                if !is_cold_trap {
                    val_non_trap[vu] = true;
                }
            });
            if let Some(r) = inst.res {
                val_weight[r.0 as usize] = val_weight[r.0 as usize].saturating_add(w);
                if !is_cold_trap {
                    val_non_trap[r.0 as usize] = true;
                }
                match inst.op {
                    Op::I32Bin(bop, a, b) | Op::I64Bin(bop, a, b) => {
                        add_peer(hint_peers, r.0, a.0);
                        if matches!(
                            bop,
                            IntBinOp::Add
                                | IntBinOp::Mul
                                | IntBinOp::And
                                | IntBinOp::Or
                                | IntBinOp::Xor
                        ) {
                            add_peer(hint_peers, r.0, b.0);
                        }
                    }
                    Op::I32BinImm(_, a, _)
                    | Op::I64BinImm(_, a, _)
                    | Op::I32WrapI64(a)
                    | Op::I64ExtendI32U(a) => {
                        add_peer(hint_peers, r.0, a.0);
                    }
                    Op::V128ExtractLane {
                        kind: super::ir::SimdLaneKind::F64x2 | super::ir::SimdLaneKind::F32x4,
                        v,
                        lane: 0,
                    } => {
                        add_peer(hint_peers, r.0, v.0);
                    }
                    Op::F32Bin(bop, a, b) | Op::F64Bin(bop, a, b) => {
                        add_peer(hint_peers, r.0, a.0);
                        if matches!(bop, super::ir::FloatBinOp::Add | super::ir::FloatBinOp::Mul) {
                            add_peer(hint_peers, r.0, b.0);
                        }
                    }
                    Op::Select { a, b, .. } => {
                        add_peer(hint_peers, r.0, a.0);
                        add_peer(hint_peers, r.0, b.0);
                    }
                    Op::CmpSelect { tv, fv, .. } => {
                        add_peer(hint_peers, r.0, tv.0);
                        add_peer(hint_peers, r.0, fv.0);
                    }
                    _ => {}
                }
            }
            match inst.op {
                Op::Call { args, rets, .. } | Op::CallIndirect { args, rets, .. } => {
                    let arg_slice = ir.pool_slice(args);
                    let _ = for_each_arg_slot(
                        arg_slice.iter().map(|&av| ir.val_tys[av.0 as usize]),
                        |idx, slot| {
                            let av = arg_slice[idx];
                            match slot {
                                AbiSlot::Gp(g) if hint_gp[av.0 as usize].is_none() => {
                                    hint_gp[av.0 as usize] = Some(g);
                                }
                                AbiSlot::Xmm(x) if hint_xmm[av.0 as usize].is_none() => {
                                    hint_xmm[av.0 as usize] = Some(x);
                                }
                                _ => {}
                            }
                        },
                    );
                    let ret_slice = ir.pool_slice(rets);
                    for &rv in ret_slice {
                        val_weight[rv.0 as usize] = val_weight[rv.0 as usize].saturating_add(w);
                        if !is_cold_trap {
                            val_non_trap[rv.0 as usize] = true;
                        }
                    }
                    let _ = for_each_ret_slot(
                        ret_slice.iter().map(|&rv| ir.val_tys[rv.0 as usize]),
                        |idx, slot| {
                            let rv = ret_slice[idx];
                            match slot {
                                AbiSlot::Gp(g) => hint_gp[rv.0 as usize] = Some(g),
                                AbiSlot::Xmm(x) => hint_xmm[rv.0 as usize] = Some(x),
                                AbiSlot::Stack(_) => {}
                            }
                        },
                    );
                }
                _ => {}
            }
        }
        let cur_ld = loop_depth[oi];
        bb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
            let vu = v.0 as usize;
            val_weight[vu] = val_weight[vu].saturating_add(w);
            if cur_ld > max_use_loop_depth[vu] {
                max_use_loop_depth[vu] = cur_ld;
            }
            if !is_cold_trap {
                val_non_trap[vu] = true;
            }
        });
        let mut wire_edge = |target_bb: u32, args: super::ir::ValRange| {
            let params = &ir.blocks[target_bb as usize].params;
            let avals = ir.pool_slice(args);
            for (&p, &a) in params.iter().zip(avals) {
                add_peer(hint_peers, p.0, a.0);
                add_peer(hint_peers, a.0, p.0);
                if a != p {
                    let prev = first_edge_arg[p.0 as usize];
                    if prev == u32::MAX {
                        first_edge_arg[p.0 as usize] = a.0;
                    } else if prev != a.0 {
                        add_peer(hint_peers, a.0, prev);
                        add_peer(hint_peers, prev, a.0);
                    }
                }
            }
        };
        match bb.term {
            Term::Jump { target, args } => wire_edge(target.0, args),
            Term::BranchIf {
                then_bb,
                then_args,
                else_bb,
                else_args,
                ..
            }
            | Term::CmpBranch {
                then_bb,
                then_args,
                else_bb,
                else_args,
                ..
            } => {
                wire_edge(then_bb.0, then_args);
                wire_edge(else_bb.0, else_args);
            }
            Term::BrTable { table_idx, .. } => {
                let td = &ir.br_tables[table_idx as usize];
                for &(t, a) in &td.unique_edges {
                    wire_edge(t.0, a);
                }
            }
            Term::Return { vals } => {
                let ret_slice = ir.pool_slice(vals);
                let _ = for_each_ret_slot(
                    ret_slice.iter().map(|&rv| ir.val_tys[rv.0 as usize]),
                    |idx, slot| {
                        let rv = ret_slice[idx];
                        match slot {
                            AbiSlot::Gp(g) if hint_gp[rv.0 as usize].is_none() => {
                                hint_gp[rv.0 as usize] = Some(g);
                            }
                            AbiSlot::Xmm(x) if hint_xmm[rv.0 as usize].is_none() => {
                                hint_xmm[rv.0 as usize] = Some(x);
                            }
                            _ => {}
                        }
                    },
                );
            }
            Term::ReturnCall { args, .. } | Term::ReturnCallIndirect { args, .. } => {
                let arg_slice = ir.pool_slice(args);
                let _ = for_each_arg_slot(
                    arg_slice.iter().map(|&av| ir.val_tys[av.0 as usize]),
                    |idx, slot| {
                        let av = arg_slice[idx];
                        match slot {
                            AbiSlot::Gp(g) if hint_gp[av.0 as usize].is_none() => {
                                hint_gp[av.0 as usize] = Some(g);
                            }
                            AbiSlot::Xmm(x) if hint_xmm[av.0 as usize].is_none() => {
                                hint_xmm[av.0 as usize] = Some(x);
                            }
                            _ => {}
                        }
                    },
                );
            }
            Term::CallCatch {
                args,
                ok_bb,
                catch_bb,
                catch_args,
                ..
            }
            | Term::CallIndirectCatch {
                args,
                ok_bb,
                catch_bb,
                catch_args,
                ..
            } => {
                wire_edge(catch_bb.0, catch_args);
                let arg_slice = ir.pool_slice(args);
                let _ = for_each_arg_slot(
                    arg_slice.iter().map(|&av| ir.val_tys[av.0 as usize]),
                    |idx, slot| {
                        let av = arg_slice[idx];
                        match slot {
                            AbiSlot::Gp(g) if hint_gp[av.0 as usize].is_none() => {
                                hint_gp[av.0 as usize] = Some(g);
                            }
                            AbiSlot::Xmm(x) if hint_xmm[av.0 as usize].is_none() => {
                                hint_xmm[av.0 as usize] = Some(x);
                            }
                            _ => {}
                        }
                    },
                );
                let ret_slice = &ir.blocks[ok_bb.0 as usize].params;
                let _ = for_each_ret_slot(
                    ret_slice.iter().map(|&rv| ir.val_tys[rv.0 as usize]),
                    |idx, slot| {
                        let rv = ret_slice[idx];
                        match slot {
                            AbiSlot::Gp(g) => hint_gp[rv.0 as usize] = Some(g),
                            AbiSlot::Xmm(x) => hint_xmm[rv.0 as usize] = Some(x),
                            AbiSlot::Stack(_) => {}
                        }
                    },
                );
            }
            Term::ThrowActive | Term::Unreachable => {}
        }
    }

    let mut locs = vec![Loc::Spill(0); n_vals];
    reset_fill(&mut scratch.assigned, n_vals, false);
    let assigned = &mut scratch.assigned;
    let total_exact_seg_pairs: usize = cb_extra_vecs
        .iter()
        .filter(|s| s.len() > 1 && s.len() <= u16::MAX as usize)
        .map(Vec::len)
        .sum();
    let mut segs_pool = std::mem::take(&mut scratch.segs_pool);
    segs_pool.clear();
    if segs_pool.capacity() < total_exact_seg_pairs {
        segs_pool.reserve_exact(total_exact_seg_pairs - segs_pool.len());
    }
    reset_fill(&mut scratch.val_clob_gp, n_vals, 0u16);
    reset_fill(&mut scratch.val_clob_xmm, n_vals, 0u16);
    scratch.gp_intervals.clear();
    scratch.fp_intervals.clear();
    let val_clob_gp = &mut scratch.val_clob_gp;
    let val_clob_xmm = &mut scratch.val_clob_xmm;
    let gp_intervals = &mut scratch.gp_intervals;
    let fp_intervals = &mut scratch.fp_intervals;
    let def_pos = &scratch.def_pos;
    let def_block = &scratch.def_block;
    let last_use = &scratch.last_use;
    let cb_id = &scratch.cb_id;
    let cb_hull = &scratch.cb_hull;
    let val_remat = &scratch.val_remat;
    let val_weight = &scratch.val_weight;
    let max_use_loop_depth = &scratch.max_use_loop_depth;
    let val_non_trap = &scratch.val_non_trap;
    let clobbers = &scratch.clobbers;
    for vi in 0..n_vals {
        if let Some(bg) = budget
            && (vi & 4095) == 4095
        {
            bg.check_deadline()?;
        }
        let dp = def_pos[vi];
        if dp == u32::MAX {
            continue;
        }
        let c = cb_id[vi];
        let (start, end, seg_off, seg_len, active_len, clob_gp, clob_xmm) =
            if c != u32::MAX && cb_hull[c as usize].0 != u32::MAX {
                let ci = c as usize;
                if has_exact_segs && !cb_seg_overflow[ci] && cb_seg_first[ci].0 != u32::MAX {
                    let ex_idx = cb_seg_extra_idx[ci];
                    if ex_idx == u32::MAX {
                        let (s0, e_last) = cb_seg_first[ci];
                        let lo = clobbers.partition_point(|cl| cl.pos <= s0);
                        let hi = clobbers.partition_point(|cl| cl.pos < e_last);
                        let (cg, cx) = query_clobbers(lo, hi);
                        (
                            s0,
                            e_last,
                            0,
                            0,
                            u64::from(e_last.saturating_sub(s0).max(1)),
                            cg,
                            cx,
                        )
                    } else {
                        let segs = &cb_extra_vecs[ex_idx as usize];
                        let s0 = segs[0].0;
                        let e_last = segs[segs.len() - 1].1;
                        let (mut cg, mut cx) = (0u16, 0u16);
                        let mut act = 0u64;
                        for &(s, e) in segs {
                            act += u64::from(e.saturating_sub(s).max(1));
                            let lo = clobbers.partition_point(|cl| cl.pos <= s);
                            let hi = clobbers.partition_point(|cl| cl.pos < e);
                            let (g, x) = query_clobbers(lo, hi);
                            cg |= g;
                            cx |= x;
                        }
                        let (so, sl) = if segs.len() > 1 && segs.len() <= u16::MAX as usize {
                            let off = segs_pool.len() as u32;
                            segs_pool.extend_from_slice(segs);
                            (off, segs.len() as u16)
                        } else {
                            (0, 0)
                        };
                        (s0, e_last, so, sl, act.max(1), cg, cx)
                    }
                } else {
                    // Conservative exact hull fallback when segment budget is exceeded.
                    let (s0, e_last) = cb_hull[ci];
                    let lo = clobbers.partition_point(|cl| cl.pos <= s0);
                    let hi = clobbers.partition_point(|cl| cl.pos < e_last);
                    let (cg, cx) = query_clobbers(lo, hi);
                    (
                        s0,
                        e_last,
                        0,
                        0,
                        u64::from(e_last.saturating_sub(s0).max(1)),
                        cg,
                        cx,
                    )
                }
            } else {
                let s = dp;
                let e = last_use[vi].max(s + 1);
                let clob_end = last_use[vi].max(s);
                let lo = clobbers.partition_point(|cl| cl.pos <= s);
                let hi = clobbers.partition_point(|cl| cl.pos < clob_end);
                let (cg, cx) = query_clobbers(lo, hi);
                (s, e, 0, 0, u64::from((e - s).max(1)), cg, cx)
            };
        val_clob_gp[vi] = clob_gp;
        val_clob_xmm[vi] = clob_xmm;
        let ty = ir.val_tys[vi];
        let mut remat_hoisted = false;
        if let Some(rc) = val_remat[vi] {
            // Always rematerialize zeros (0-cycle `xor`/`xorps`, direct immediate stores — eliminates all
            // `bb0` `cached_zeros` live ranges and stack spills!), call/clobber-crossing constants, and
            // non-loop constants (`val_weight <= 8`). Hot non-zero loop-invariant GP constants (defined
            // outside a loop and used inside) may also occupy a free callee-saved GP register (still at
            // score 0, so they never evict or spill real values).
            let crosses_clobber = if ty.is_fp() {
                (clob_xmm & MASK_XMM_CALLER) == MASK_XMM_CALLER
            } else {
                (clob_gp & MASK_GP_CALLER) == MASK_GP_CALLER
            };
            let def_ld = loop_depth[def_block[vi] as usize];
            if !rc.is_zero() && !ty.is_fp() && !crosses_clobber && def_ld < max_use_loop_depth[vi] {
                remat_hoisted = true;
            }
            if rc.is_zero() || crosses_clobber || val_weight[vi] <= 8 {
                locs[vi] = Loc::Remat(rc);
                assigned[vi] = true;
                continue;
            }
        }
        let score = if val_remat[vi].is_some() {
            // Rematerializable constants are always preferred eviction victims over real values.
            0
        } else if !val_non_trap[vi] {
            // Cold trap-only values never evict hot normal or loop-carried values!
            1
        } else {
            ((u64::from(val_weight[vi].max(1)) << 12) / active_len).max(2)
        };
        let iv = Interval {
            val: vi as u32,
            start,
            end,
            seg_off,
            seg_len,
            ty,
            clob_gp,
            clob_xmm,
            score,
            remat: val_remat[vi],
            remat_hoisted,
        };
        if iv.ty.is_fp() {
            fp_intervals.push(iv);
        } else {
            gp_intervals.push(iv);
        }
    }
    drop(cb_extra_vecs);
    if cb_seg_first.capacity() <= SCRATCH_RECYCLE_CAP {
        scratch.cb_seg_first = cb_seg_first;
    } else {
        drop(cb_seg_first);
    }
    if cb_seg_extra_idx.capacity() <= SCRATCH_RECYCLE_CAP {
        scratch.cb_seg_extra_idx = cb_seg_extra_idx;
    } else {
        drop(cb_seg_extra_idx);
    }
    if cb_seg_overflow.capacity() <= SCRATCH_RECYCLE_CAP {
        scratch.cb_seg_overflow = cb_seg_overflow;
    } else {
        drop(cb_seg_overflow);
    }

    if let Some(bg) = budget {
        bg.check_deadline()?;
    }
    if gp_intervals.len() > 1
        && !gp_intervals
            .windows(2)
            .all(|w| (w[0].start, w[0].end) <= (w[1].start, w[1].end))
    {
        gp_intervals.sort_unstable_by_key(|iv| (iv.start, iv.end, iv.val));
    }
    if let Some(bg) = budget {
        bg.check_deadline()?;
    }
    if fp_intervals.len() > 1
        && !fp_intervals
            .windows(2)
            .all(|w| (w[0].start, w[0].end) <= (w[1].start, w[1].end))
    {
        fp_intervals.sort_unstable_by_key(|iv| (iv.start, iv.end, iv.val));
    }

    let mut next_spill_off: i32 = 0;
    // Exact current max `end` for each 8-byte spill slot (`idx = off / 8`); `u32::MAX` marks non-8B/padding slots.
    scratch.slot8_end.clear();
    scratch.free_spills8.clear();
    let slot8_end = &mut scratch.slot8_end;
    let free_spills8 = &mut scratch.free_spills8;

    // Allocates an 8-byte spill slot for `val` over `[start, end]`, first attempting O(1) spill-slot
    // coalescing with any already-spilled non-interfering `hint_peers` value to eliminate edge-copy
    // trampolines, then reusing the earliest expired 8-byte slot, else bumping `next_spill_off`.
    let alloc_spill8 = |val: u32,
                        start: u32,
                        end: u32,
                        locs: &[Loc],
                        assigned: &[bool],
                        slot8_end: &mut Vec<u32>,
                        free: &mut BinaryHeap<Reverse<(u32, i32)>>,
                        next: &mut i32|
     -> Result<i32, String> {
        for &peer in &hint_peers[val as usize] {
            if peer != u32::MAX
                && assigned[peer as usize]
                && ir.val_tys[peer as usize] != Ty::V128
                && let Loc::Spill(poff) = locs[peer as usize]
            {
                let si = (poff / 8) as usize;
                if si < slot8_end.len() && slot8_end[si] <= start {
                    slot8_end[si] = end;
                    free.push(Reverse((end, poff)));
                    return Ok(poff);
                }
            }
        }
        while let Some(&Reverse((e, off))) = free.peek() {
            if e > start {
                break;
            }
            free.pop();
            let si = (off / 8) as usize;
            if si < slot8_end.len() && slot8_end[si] == e {
                slot8_end[si] = end;
                free.push(Reverse((end, off)));
                return Ok(off);
            }
        }
        let off = *next;
        let after = off.checked_add(8).ok_or("x64: spill stack size overflow")?;
        if after > MAX_FRAME_BYTES {
            return Err(format!(
                "x64: spill area ({after} B) exceeds frame cap {MAX_FRAME_BYTES} B"
            ));
        }
        *next = after;
        let si = (off / 8) as usize;
        if slot8_end.len() <= si {
            slot8_end.resize(si + 1, u32::MAX);
        }
        slot8_end[si] = end;
        free.push(Reverse((end, off)));
        Ok(off)
    };

    // 5a. Linear scan for GP intervals with exact live-range holes, dynamic R12/R13, remat,
    // forward peer clobber avoidance, coalescing, and spill-slot reuse.
    scratch.active_gp.clear();
    let active_gp = &mut scratch.active_gp;
    const GP_CALLER_COUNT: usize = 7;
    // Preferred free-reg order (indices into `gp_alloc`): non-fixed caller-saved first (`R8, R9, Rsi, Rdi`),
    // then `Rcx, Rdx, Rax`, then callee-saved (`Rbx, Rbp, R12, R13`).
    const GP_PREF_ORDER: [usize; 11] = [5, 6, 3, 4, 1, 2, 0, 7, 8, 9, 10];

    for (idx, iv) in gp_intervals.iter().enumerate() {
        if let Some(bg) = budget
            && (idx & 4095) == 4095
        {
            bg.check_deadline()?;
        }
        active_gp.retain(|a| a.end > iv.start);
        let mut used_regs = 0u16;
        for a in active_gp.iter() {
            if active_overlaps_iv(a, iv, &segs_pool) {
                used_regs |= 1 << gp_alloc[a.reg_idx as usize].id();
            }
        }
        let forbidden = used_regs | iv.clob_gp;
        let max_ri = if iv.remat.is_some() && !iv.remat_hoisted {
            GP_CALLER_COUNT
        } else {
            gp_alloc.len()
        };

        // 1. Coalescing peer already assigned a non-interfering register.
        let mut chosen: Option<(usize, Gp)> = None;
        for &peer in &hint_peers[iv.val as usize] {
            if peer != u32::MAX
                && assigned[peer as usize]
                && let Loc::Gp(g) = locs[peer as usize]
                && (forbidden & (1 << g.id())) == 0
                && let Some(ri) = gp_alloc[..max_ri].iter().position(|&x| x == g)
            {
                chosen = Some((ri, g));
                break;
            }
        }
        // 2. Direct ABI hint register.
        if chosen.is_none()
            && let Some(g) = hint_gp[iv.val as usize]
            && (forbidden & (1 << g.id())) == 0
            && let Some(ri) = gp_alloc[..max_ri].iter().position(|&x| x == g)
        {
            chosen = Some((ri, g));
        }
        // 3. Forward peer ABI hint & peer clobber avoidance (so loop header params avoid regs clobbered by latch).
        if chosen.is_none() {
            let mut peer_avoid = 0u16;
            let mut peer_hint: Option<Gp> = None;
            for &peer in &hint_peers[iv.val as usize] {
                if peer != u32::MAX && !assigned[peer as usize] {
                    peer_avoid |= val_clob_gp[peer as usize];
                    if peer_hint.is_none() {
                        peer_hint = hint_gp[peer as usize];
                    }
                }
            }
            if let Some(g) = peer_hint
                && ((forbidden | peer_avoid) & (1 << g.id())) == 0
                && let Some(ri) = gp_alloc[..max_ri].iter().position(|&x| x == g)
            {
                chosen = Some((ri, g));
            }
            if chosen.is_none() && peer_avoid != 0 {
                let avoid_mask = forbidden | peer_avoid;
                for &ri in &GP_PREF_ORDER {
                    if ri < max_ri && (avoid_mask & (1 << gp_alloc[ri].id())) == 0 {
                        chosen = Some((ri, gp_alloc[ri]));
                        break;
                    }
                }
            }
            if chosen.is_none() {
                for &ri in &GP_PREF_ORDER {
                    if ri < max_ri && (forbidden & (1 << gp_alloc[ri].id())) == 0 {
                        chosen = Some((ri, gp_alloc[ri]));
                        break;
                    }
                }
            }
        }

        if let Some((ri, g)) = chosen {
            locs[iv.val as usize] = Loc::Gp(g);
            assigned[iv.val as usize] = true;
            active_gp.push(ActiveEntry {
                start: iv.start,
                end: iv.end,
                seg_off: iv.seg_off,
                seg_len: iv.seg_len,
                reg_idx: ri as u8,
                val: iv.val,
                score: iv.score,
                remat: iv.remat,
            });
        } else if let Some(rc) = iv.remat {
            locs[iv.val as usize] = Loc::Remat(rc);
            assigned[iv.val as usize] = true;
        } else {
            // Find the lowest-cost evictable register (evicting ALL active entries on `ri` overlapping `iv`).
            let mut best_ri: Option<(usize, u64, usize, u32)> = None;
            for (ri, &g) in gp_alloc.iter().enumerate() {
                if (iv.clob_gp & (1 << g.id())) != 0 {
                    continue;
                }
                let (mut cnt, mut max_sc, mut max_end) = (0usize, 0u64, 0u32);
                let mut ok = true;
                for a in active_gp.iter() {
                    if a.reg_idx as usize == ri && active_overlaps_iv(a, iv, &segs_pool) {
                        cnt += 1;
                        if a.score > max_sc {
                            max_sc = a.score;
                        }
                        if a.end > max_end {
                            max_end = a.end;
                        }
                        if a.score > iv.score {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok || cnt == 0 {
                    continue;
                }
                if max_sc == iv.score && !(cnt == 1 && max_end > iv.end) {
                    continue;
                }
                let better = match best_ri {
                    None => true,
                    Some((_, bsc, bcnt, bend)) => {
                        (max_sc, cnt, Reverse(max_end)) < (bsc, bcnt, Reverse(bend))
                    }
                };
                if better {
                    best_ri = Some((ri, max_sc, cnt, max_end));
                }
            }
            if let Some((ri, _, _, _)) = best_ri {
                let g = gp_alloc[ri];
                let mut i = active_gp.len();
                while i > 0 {
                    i -= 1;
                    if active_gp[i].reg_idx as usize == ri
                        && active_overlaps_iv(&active_gp[i], iv, &segs_pool)
                    {
                        let ev = active_gp.swap_remove(i);
                        if let Some(rc) = ev.remat {
                            locs[ev.val as usize] = Loc::Remat(rc);
                        } else {
                            let sp = alloc_spill8(
                                ev.val,
                                ev.start,
                                ev.end,
                                &locs,
                                assigned,
                                slot8_end,
                                free_spills8,
                                &mut next_spill_off,
                            )?;
                            locs[ev.val as usize] = Loc::Spill(sp);
                        }
                    }
                }
                locs[iv.val as usize] = Loc::Gp(g);
                assigned[iv.val as usize] = true;
                active_gp.push(ActiveEntry {
                    start: iv.start,
                    end: iv.end,
                    seg_off: iv.seg_off,
                    seg_len: iv.seg_len,
                    reg_idx: ri as u8,
                    val: iv.val,
                    score: iv.score,
                    remat: iv.remat,
                });
            } else {
                let sp = alloc_spill8(
                    iv.val,
                    iv.start,
                    iv.end,
                    &locs,
                    assigned,
                    slot8_end,
                    free_spills8,
                    &mut next_spill_off,
                )?;
                locs[iv.val as usize] = Loc::Spill(sp);
                assigned[iv.val as usize] = true;
            }
        }
    }

    // 5b. Linear scan for FP/V128 intervals (caller-saved Xmm0..Xmm7 + callee-saved Xmm8..Xmm13).
    scratch.free_spills16.clear();
    scratch.active_fp.clear();
    let free_spills16 = &mut scratch.free_spills16;
    let active_fp = &mut scratch.active_fp;
    const XMM_CALLER_COUNT: usize = 8;
    const XMM_PREF_ORDER: [usize; 14] = [7, 6, 5, 4, 3, 2, 1, 0, 8, 9, 10, 11, 12, 13];

    let alloc_spill_fp = |val: u32,
                          ty: Ty,
                          start: u32,
                          end: u32,
                          locs: &[Loc],
                          assigned: &[bool],
                          slot8_end: &mut Vec<u32>,
                          free8: &mut BinaryHeap<Reverse<(u32, i32)>>,
                          free16: &mut BinaryHeap<Reverse<(u32, i32)>>,
                          next: &mut i32|
     -> Result<i32, String> {
        if ty == Ty::V128 {
            if let Some(&Reverse((e, _))) = free16.peek()
                && e <= start
            {
                let Reverse((_, off)) = free16.pop().unwrap();
                free16.push(Reverse((end, off)));
                Ok(off)
            } else {
                let off = checked_align_i32(*next, 16)?;
                let after = off
                    .checked_add(16)
                    .ok_or("x64: spill stack size overflow")?;
                if after > MAX_FRAME_BYTES {
                    return Err(format!(
                        "x64: spill area ({after} B) exceeds frame cap {MAX_FRAME_BYTES} B"
                    ));
                }
                *next = after;
                free16.push(Reverse((end, off)));
                Ok(off)
            }
        } else {
            alloc_spill8(val, start, end, locs, assigned, slot8_end, free8, next)
        }
    };

    for (idx, iv) in fp_intervals.iter().enumerate() {
        if let Some(bg) = budget
            && (idx & 4095) == 4095
        {
            bg.check_deadline()?;
        }
        active_fp.retain(|a| a.end > iv.start);
        let mut used_regs = 0u16;
        for a in active_fp.iter() {
            if active_overlaps_iv(a, iv, &segs_pool) {
                used_regs |= 1 << XMM_ALLOC[a.reg_idx as usize].id();
            }
        }
        let forbidden = used_regs | iv.clob_xmm;
        let max_ri = if iv.remat.is_some() {
            XMM_CALLER_COUNT
        } else {
            XMM_ALLOC.len()
        };

        let mut chosen: Option<(usize, Xmm)> = None;
        for &peer in &hint_peers[iv.val as usize] {
            if peer != u32::MAX
                && assigned[peer as usize]
                && let Loc::Xmm(x) = locs[peer as usize]
                && (forbidden & (1 << x.id())) == 0
                && let Some(ri) = XMM_ALLOC[..max_ri].iter().position(|&r| r == x)
            {
                chosen = Some((ri, x));
                break;
            }
        }
        if chosen.is_none()
            && let Some(x) = hint_xmm[iv.val as usize]
            && (forbidden & (1 << x.id())) == 0
            && let Some(ri) = XMM_ALLOC[..max_ri].iter().position(|&r| r == x)
        {
            chosen = Some((ri, x));
        }
        if chosen.is_none() {
            let mut peer_avoid = 0u16;
            let mut peer_hint: Option<Xmm> = None;
            for &peer in &hint_peers[iv.val as usize] {
                if peer != u32::MAX && !assigned[peer as usize] {
                    peer_avoid |= val_clob_xmm[peer as usize];
                    if peer_hint.is_none() {
                        peer_hint = hint_xmm[peer as usize];
                    }
                }
            }
            if let Some(x) = peer_hint
                && ((forbidden | peer_avoid) & (1 << x.id())) == 0
                && let Some(ri) = XMM_ALLOC[..max_ri].iter().position(|&r| r == x)
            {
                chosen = Some((ri, x));
            }
            if chosen.is_none() && peer_avoid != 0 {
                let avoid_mask = forbidden | peer_avoid;
                for &ri in &XMM_PREF_ORDER {
                    if ri < max_ri && (avoid_mask & (1 << XMM_ALLOC[ri].id())) == 0 {
                        chosen = Some((ri, XMM_ALLOC[ri]));
                        break;
                    }
                }
            }
            if chosen.is_none() {
                for &ri in &XMM_PREF_ORDER {
                    if ri < max_ri && (forbidden & (1 << XMM_ALLOC[ri].id())) == 0 {
                        chosen = Some((ri, XMM_ALLOC[ri]));
                        break;
                    }
                }
            }
        }

        if let Some((ri, x)) = chosen {
            locs[iv.val as usize] = Loc::Xmm(x);
            assigned[iv.val as usize] = true;
            active_fp.push(ActiveEntry {
                start: iv.start,
                end: iv.end,
                seg_off: iv.seg_off,
                seg_len: iv.seg_len,
                reg_idx: ri as u8,
                val: iv.val,
                score: iv.score,
                remat: iv.remat,
            });
        } else if let Some(rc) = iv.remat {
            locs[iv.val as usize] = Loc::Remat(rc);
            assigned[iv.val as usize] = true;
        } else {
            let mut best_ri: Option<(usize, u64, usize, u32)> = None;
            for (ri, &x) in XMM_ALLOC.iter().enumerate() {
                if (iv.clob_xmm & (1 << x.id())) != 0 {
                    continue;
                }
                let (mut cnt, mut max_sc, mut max_end) = (0usize, 0u64, 0u32);
                let mut ok = true;
                for a in active_fp.iter() {
                    if a.reg_idx as usize == ri && active_overlaps_iv(a, iv, &segs_pool) {
                        cnt += 1;
                        if a.score > max_sc {
                            max_sc = a.score;
                        }
                        if a.end > max_end {
                            max_end = a.end;
                        }
                        if a.score > iv.score {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok || cnt == 0 {
                    continue;
                }
                if max_sc == iv.score && !(cnt == 1 && max_end > iv.end) {
                    continue;
                }
                let better = match best_ri {
                    None => true,
                    Some((_, bsc, bcnt, bend)) => {
                        (max_sc, cnt, Reverse(max_end)) < (bsc, bcnt, Reverse(bend))
                    }
                };
                if better {
                    best_ri = Some((ri, max_sc, cnt, max_end));
                }
            }
            if let Some((ri, _, _, _)) = best_ri {
                let x = XMM_ALLOC[ri];
                let mut i = active_fp.len();
                while i > 0 {
                    i -= 1;
                    if active_fp[i].reg_idx as usize == ri
                        && active_overlaps_iv(&active_fp[i], iv, &segs_pool)
                    {
                        let ev = active_fp.swap_remove(i);
                        if let Some(rc) = ev.remat {
                            locs[ev.val as usize] = Loc::Remat(rc);
                        } else {
                            let ev_ty = ir.val_tys[ev.val as usize];
                            let sp = alloc_spill_fp(
                                ev.val,
                                ev_ty,
                                ev.start,
                                ev.end,
                                &locs,
                                assigned,
                                slot8_end,
                                free_spills8,
                                free_spills16,
                                &mut next_spill_off,
                            )?;
                            locs[ev.val as usize] = Loc::Spill(sp);
                        }
                    }
                }
                locs[iv.val as usize] = Loc::Xmm(x);
                assigned[iv.val as usize] = true;
                active_fp.push(ActiveEntry {
                    start: iv.start,
                    end: iv.end,
                    seg_off: iv.seg_off,
                    seg_len: iv.seg_len,
                    reg_idx: ri as u8,
                    val: iv.val,
                    score: iv.score,
                    remat: iv.remat,
                });
            } else {
                let sp = alloc_spill_fp(
                    iv.val,
                    iv.ty,
                    iv.start,
                    iv.end,
                    &locs,
                    assigned,
                    slot8_end,
                    free_spills8,
                    free_spills16,
                    &mut next_spill_off,
                )?;
                locs[iv.val as usize] = Loc::Spill(sp);
                assigned[iv.val as usize] = true;
            }
        }
    }

    // 6. Compute exact used callee-saved sets from final assignments and aligned stack frame layout:
    //   [rsp + 0 .. outgoing_arg_bytes]                     : outgoing stack args (16-byte aligned)
    //   [rsp + ret_buf_off .. ret_buf_off + ret_buf_bytes]  : callee overflow return buffer (16-byte aligned)
    //   [rsp + spill_base .. spill_base + next_spill_off]   : SSA spills (16-byte aligned base for V128)
    //   [rsp + saved_xmm_base .. + 16*used_callee_xmm.len()]: saved callee-saved Xmm8..Xmm13 (16-byte aligned)
    //   [rsp + saved_ret_ptr_off]                           : optional saved caller ret_spill_ptr (8 bytes)
    let mut used_callee_gp_mask = 0u16;
    let mut used_callee_xmm_mask = 0u16;
    for (vi, &loc) in locs.iter().enumerate() {
        if !assigned[vi] {
            continue;
        }
        match loc {
            Loc::Gp(g) if cfg.is_callee_saved_gp(g) => used_callee_gp_mask |= 1 << g.id(),
            Loc::Xmm(x) if x.id() >= 8 => used_callee_xmm_mask |= 1 << x.id(),
            _ => {}
        }
    }
    let mut used_callee_gp = Vec::new();
    for &g in gp_callee {
        if (used_callee_gp_mask & (1 << g.id())) != 0 {
            used_callee_gp.push(g);
        }
    }
    let mut used_callee_xmm = Vec::new();
    for &x in &XMM_CALLEE_SAVED {
        if (used_callee_xmm_mask & (1 << x.id())) != 0 {
            used_callee_xmm.push(x);
        }
    }
    let outgoing_arg_bytes = max_outgoing_arg_bytes;
    let ret_buf_off = outgoing_arg_bytes;
    let ret_buf_bytes = max_ret_buf_bytes;
    let spill_base = ret_buf_off
        .checked_add(ret_buf_bytes)
        .and_then(|v| v.checked_add(15))
        .ok_or("x64: stack frame size overflow")?
        & !15;
    if spill_base > 0 {
        for loc in &mut locs {
            if let Loc::Spill(off) = loc {
                *off = off
                    .checked_add(spill_base)
                    .ok_or("x64: spill offset overflow")?;
            }
        }
    }
    let mut raw_frame = spill_base
        .checked_add(next_spill_off)
        .ok_or("x64: stack frame size overflow")?;
    let saved_xmm_base = if !used_callee_xmm.is_empty() {
        let base = checked_align_i32(raw_frame, 16)?;
        let xmm_bytes = (used_callee_xmm.len() as i32) * 16;
        raw_frame = base
            .checked_add(xmm_bytes)
            .ok_or("x64: stack frame size overflow")?;
        base
    } else {
        0
    };
    let self_ret_stack_bytes = layout_rets_stack_bytes(ir.result_tys.iter().copied())?;
    let saved_ret_ptr_off = if self_ret_stack_bytes > 0 {
        raw_frame = raw_frame
            .checked_add(7)
            .ok_or("x64: stack frame size overflow")?
            & !7;
        let off = raw_frame;
        raw_frame = raw_frame
            .checked_add(8)
            .ok_or("x64: stack frame size overflow")?;
        Some(off)
    } else {
        None
    };

    // On function entry, `call` pushed an 8-byte return address so `rsp % 16 == 8`.
    // Pushing `k` callee-saved regs shifts `rsp` by `8*k`. We want `(8 + 8*k + frame_size) % 16 == 0`.
    let pushes_bytes = (1 + used_callee_gp.len()) as i32 * 8;
    let frame_size = if raw_frame == 0 && clobbers.is_empty() {
        0
    } else {
        let total = pushes_bytes
            .checked_add(raw_frame)
            .and_then(|v| v.checked_add(15))
            .ok_or("x64: stack frame size overflow")?
            & !15;
        total - pushes_bytes
    };
    if frame_size > MAX_FRAME_BYTES {
        return Err(format!(
            "x64: function {} stack frame ({frame_size} B) exceeds cap {MAX_FRAME_BYTES} B",
            ir.func_index
        ));
    }
    let incoming_stack_base = frame_size
        .checked_add(pushes_bytes)
        .and_then(|v| v.checked_add(self_param_stack_bytes))
        .ok_or("x64: incoming stack parameter offset overflow")?;
    if incoming_stack_base > MAX_FRAME_BYTES.saturating_mul(2) {
        return Err(format!(
            "x64: function {} incoming stack offset ({incoming_stack_base} B) exceeds cap",
            ir.func_index
        ));
    }

    let mut block_live_in_gp = vec![0u16; n_blocks];
    let mut block_live_in_xmm = vec![0u16; n_blocks];
    if saved_live_words > 0 {
        for (oi, &bid) in block_order.iter().enumerate() {
            let mut gp_m = 0u16;
            let mut xmm_m = 0u16;
            let base = oi * saved_live_words;
            for w in 0..saved_live_words {
                let mut bits = saved_live_in[base + w];
                while bits != 0 {
                    let tz = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let c = (w << 6) + tz;
                    let vi = cb_vals[c] as usize;
                    if assigned[vi] {
                        match locs[vi] {
                            Loc::Gp(g) => gp_m |= 1u16 << g.id(),
                            Loc::Xmm(x) => xmm_m |= 1u16 << x.id(),
                            Loc::Spill(_) | Loc::Remat(_) => {}
                        }
                    }
                }
            }
            block_live_in_gp[bid as usize] = gp_m;
            block_live_in_xmm[bid as usize] = xmm_m;
        }
    }
    if saved_live_in.capacity() <= SCRATCH_RECYCLE_CAP {
        scratch.live_in = saved_live_in;
    }
    if segs_pool.capacity() <= SCRATCH_RECYCLE_CAP {
        scratch.segs_pool = segs_pool;
    }

    Ok(AllocResult {
        locs,
        block_order,
        used_callee_gp,
        used_callee_xmm,
        saved_xmm_base,
        frame_size,
        outgoing_arg_bytes,
        ret_buf_off,
        ret_buf_bytes,
        saved_ret_ptr_off,
        block_live_in_gp,
        block_live_in_xmm,
    })
}
