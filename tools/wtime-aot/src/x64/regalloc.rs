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

/// Physical location assigned to an SSA `Val`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loc {
    Gp(Gp),
    Xmm(Xmm),
    /// Stack spill slot at `[rsp + offset]` (8-byte aligned for scalars, 16-byte aligned for V128).
    Spill(i32),
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
pub const GP_CALLEE_SAVED: [Gp; 2] = [Gp::Rbx, Gp::Rbp];
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

pub fn layout_args(tys: &[Ty]) -> Result<AbiLayout, String> {
    let mut slots = Vec::with_capacity(tys.len());
    let (mut ng, mut nf, mut off) = (0usize, 0usize, 0i32);
    for &ty in tys {
        if ty.is_fp() {
            if nf < CALL_FP_ARGS.len() {
                slots.push(AbiSlot::Xmm(CALL_FP_ARGS[nf]));
                nf += 1;
            } else {
                let (sz, align) = if ty == Ty::V128 { (16, 16) } else { (8, 8) };
                off = checked_align_i32(off, align)?;
                slots.push(AbiSlot::Stack(off));
                off = off.checked_add(sz).ok_or("x64: arg stack size overflow")?;
            }
        } else if ng < CALL_GP_ARGS.len() {
            slots.push(AbiSlot::Gp(CALL_GP_ARGS[ng]));
            ng += 1;
        } else {
            off = checked_align_i32(off, 8)?;
            slots.push(AbiSlot::Stack(off));
            off = off.checked_add(8).ok_or("x64: arg stack size overflow")?;
        }
    }
    let stack_bytes = checked_align_i32(off, 16)?;
    Ok(AbiLayout { slots, stack_bytes })
}

pub fn layout_rets(tys: &[Ty]) -> Result<AbiLayout, String> {
    let mut slots = Vec::with_capacity(tys.len());
    let (mut ng, mut nf, mut off) = (0usize, 0usize, 0i32);
    for &ty in tys {
        if ty.is_fp() {
            if nf < CALL_FP_RETS.len() {
                slots.push(AbiSlot::Xmm(CALL_FP_RETS[nf]));
                nf += 1;
            } else {
                let (sz, align) = if ty == Ty::V128 { (16, 16) } else { (8, 8) };
                off = checked_align_i32(off, align)?;
                slots.push(AbiSlot::Stack(off));
                off = off.checked_add(sz).ok_or("x64: ret stack size overflow")?;
            }
        } else if ng < CALL_GP_RETS.len() {
            slots.push(AbiSlot::Gp(CALL_GP_RETS[ng]));
            ng += 1;
        } else {
            off = checked_align_i32(off, 8)?;
            slots.push(AbiSlot::Stack(off));
            off = off.checked_add(8).ok_or("x64: ret stack size overflow")?;
        }
    }
    let stack_bytes = checked_align_i32(off, 16)?;
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
    ty: Ty,
    clob_gp: u16,
    clob_xmm: u16,
    score: u64,
}

/// Computes RPO order of reachable blocks, exact cross-block liveness, and linear-scan allocation.
#[allow(dead_code)]
pub fn allocate(ir: &IrFunc) -> Result<AllocResult, String> {
    allocate_with_budget(ir, None)
}

/// Computes RPO order, exact cross-block liveness (charging concurrent peak liveness memory BEFORE
/// allocation), and linear-scan allocation while polling `budget`'s cooperative deadline.
pub fn allocate_with_budget(ir: &IrFunc, budget: Option<&CompileBudget>) -> Result<AllocResult, String> {
    if let Some(b) = budget {
        b.check_deadline()?;
    }
    let n_blocks = ir.blocks.len();
    let n_vals = ir.val_tys.len();

    // 1. Compute reachable block order (iterative DFS postorder -> reversed = RPO)
    // with O(1) per-edge successor deduplication.
    let mut visited = vec![false; n_blocks];
    let mut postorder = Vec::with_capacity(n_blocks);
    let mut dfs: Vec<(u32, usize)> = vec![(0, 0)];
    let mut succs: Vec<Vec<u32>> = vec![Vec::new(); n_blocks];
    let mut succ_seen = vec![u32::MAX; n_blocks];
    for (bi, bb) in ir.blocks.iter().enumerate() {
        if let Some(b) = budget
            && (bi & 4095) == 4095
        {
            b.check_deadline()?;
        }
        let stamp = bi as u32;
        bb.term.for_each_succ(&ir.br_tables, |s| {
            let sid = s.0 as usize;
            if succ_seen[sid] != stamp {
                succ_seen[sid] = stamp;
                succs[bi].push(s.0);
            }
        });
    }
    visited[0] = true;
    let mut dfs_steps: usize = 0;
    while let Some((b, si)) = dfs.last_mut() {
        dfs_steps += 1;
        if let Some(bg) = budget
            && (dfs_steps & 4095) == 0
        {
            bg.check_deadline()?;
        }
        let cur = *b as usize;
        if *si < succs[cur].len() {
            let nxt = succs[cur][*si];
            *si += 1;
            if !visited[nxt as usize] {
                visited[nxt as usize] = true;
                dfs.push((nxt, 0));
            }
        } else {
            postorder.push(*b);
            dfs.pop();
        }
    }
    postorder.reverse();
    let block_order = postorder;
    let n_ord = block_order.len();
    let mut block_pos_in_order = vec![u32::MAX; n_blocks];
    for (oi, &b) in block_order.iter().enumerate() {
        block_pos_in_order[b as usize] = oi as u32;
    }

    // 2. Assign linear program positions (2 ticks per instruction: even=use, odd=def).
    let mut block_start = vec![0u32; n_ord];
    let mut block_end = vec![0u32; n_ord];
    let mut def_pos = vec![u32::MAX; n_vals];
    let mut def_block = vec![u32::MAX; n_vals];
    let mut last_use = vec![0u32; n_vals];
    let mut cross_block = vec![false; n_vals];
    let mut clobbers: Vec<Clobber> = Vec::new();
    let mut max_outgoing_arg_bytes = 0i32;
    let mut max_ret_buf_bytes = 0i32;
    let mut scratch_tys: Vec<Ty> = Vec::new();

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
                    clobbers.push(Clobber { pos: pos_use, gp_mask: MASK_GP_CALLER, xmm_mask: MASK_XMM_CALLER });
                    scratch_tys.clear();
                    for &av in ir.pool_slice(args) {
                        scratch_tys.push(ir.val_tys[av.0 as usize]);
                    }
                    let arg_layout = layout_args(&scratch_tys)?;
                    if arg_layout.stack_bytes > max_outgoing_arg_bytes {
                        max_outgoing_arg_bytes = arg_layout.stack_bytes;
                    }
                    scratch_tys.clear();
                    for &rv in ir.pool_slice(rets) {
                        let vi = rv.0 as usize;
                        scratch_tys.push(ir.val_tys[vi]);
                        def_pos[vi] = pos_def;
                        def_block[vi] = oi as u32;
                        if last_use[vi] < pos_def + 1 {
                            last_use[vi] = pos_def + 1;
                        }
                    }
                    let ret_layout = layout_rets(&scratch_tys)?;
                    if ret_layout.stack_bytes > max_ret_buf_bytes {
                        max_ret_buf_bytes = ret_layout.stack_bytes;
                    }
                }
                Op::MemCopy { .. } => {
                    let (gp_mask, xmm_mask) = helper_clobbers(HelperKind::MemCopy);
                    clobbers.push(Clobber { pos: pos_use, gp_mask, xmm_mask });
                }
                Op::MemFill { .. } => {
                    let (gp_mask, xmm_mask) = helper_clobbers(HelperKind::MemFill);
                    clobbers.push(Clobber { pos: pos_use, gp_mask, xmm_mask });
                }
                Op::MemGrow(_) => {
                    let (gp_mask, xmm_mask) = helper_clobbers(HelperKind::MemGrow);
                    clobbers.push(Clobber { pos: pos_use, gp_mask, xmm_mask });
                }
                Op::Trunc(_, _) => {
                    let (gp_mask, xmm_mask) = helper_clobbers(HelperKind::Trunc);
                    clobbers.push(Clobber { pos: pos_use, gp_mask, xmm_mask });
                }
                Op::I32Bin(op, _, _) | Op::I64Bin(op, _, _) => match op {
                    IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU => {
                        clobbers.push(Clobber { pos: pos_use, gp_mask: MASK_RAX_RDX, xmm_mask: 0 });
                    }
                    IntBinOp::Shl | IntBinOp::ShrS | IntBinOp::ShrU | IntBinOp::Rotl | IntBinOp::Rotr => {
                        clobbers.push(Clobber { pos: pos_use, gp_mask: MASK_RCX, xmm_mask: 0 });
                    }
                    _ => {}
                },
                Op::I64BinImm(IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU, _, _) => {
                    clobbers.push(Clobber { pos: pos_use, gp_mask: MASK_RAX_RDX, xmm_mask: 0 });
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
            }
            tick += 2;
        }
        let pos_term = tick;
        bb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
            let vi = v.0 as usize;
            if def_block[vi] != oi as u32 {
                cross_block[vi] = true;
            }
            if pos_term > last_use[vi] {
                last_use[vi] = pos_term;
            }
        });
        block_end[oi] = pos_term + 1;
        tick += 2;
    }

    // 3. Exact cross-block liveness ONLY for values with `cross_block[v] == true`.
    let mut cb_id = vec![u32::MAX; n_vals];
    let mut cb_vals: Vec<u32> = Vec::new();
    for vi in 0..n_vals {
        if cross_block[vi] && def_pos[vi] != u32::MAX {
            cb_id[vi] = cb_vals.len() as u32;
            cb_vals.push(vi as u32);
        }
    }
    let n_cb = cb_vals.len();
    if n_cb > 0 {
        let words = n_cb.div_ceil(64);
        let liveness_bytes = n_ord
            .checked_mul(words)
            .and_then(|n| n.checked_mul(4 * 8))
            .ok_or("x64: liveness table size overflow")?;
        if liveness_bytes > MAX_LIVENESS_BYTES {
            return Err(format!(
                "x64: function {} liveness matrix ({liveness_bytes} B) exceeds cap {MAX_LIVENESS_BYTES} B",
                ir.func_index
            ));
        }
        let _liveness_guard = match budget {
            Some(b) => Some(b.reserve_liveness(liveness_bytes)?),
            None => None,
        };
        let mut live_use = vec![0u64; n_ord * words];
        let mut live_def = vec![0u64; n_ord * words];
        let mut live_in = vec![0u64; n_ord * words];
        let mut live_out = vec![0u64; n_ord * words];

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
        while changed {
            if let Some(b) = budget {
                b.check_deadline()?;
            }
            changed = false;
            for oi in (0..n_ord).rev() {
                let bid = block_order[oi] as usize;
                for &sid in &succs[bid] {
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
                    let nv = live_use[oi * words + w] | (live_out[oi * words + w] & !live_def[oi * words + w]);
                    if nv != live_in[oi * words + w] {
                        live_in[oi * words + w] = nv;
                        changed = true;
                    }
                }
            }
        }

        for oi in 0..n_ord {
            if let Some(bg) = budget
                && (oi & 4095) == 4095
            {
                bg.check_deadline()?;
            }
            let bs = block_start[oi];
            let be = block_end[oi];
            for w in 0..words {
                let mut in_bits = live_in[oi * words + w];
                while in_bits != 0 {
                    let tz = in_bits.trailing_zeros() as usize;
                    in_bits &= in_bits - 1;
                    let vi = cb_vals[(w << 6) + tz] as usize;
                    if bs < def_pos[vi] {
                        def_pos[vi] = bs;
                    }
                }
                let mut out_bits = live_out[oi * words + w];
                while out_bits != 0 {
                    let tz = out_bits.trailing_zeros() as usize;
                    out_bits &= out_bits - 1;
                    let vi = cb_vals[(w << 6) + tz] as usize;
                    if be > last_use[vi] {
                        last_use[vi] = be;
                    }
                }
            }
        }
    }

    // 4. Build O(log C) segment tree over clobbers and build exact intervals.
    let n_clob = clobbers.len();
    let mut clob_seg = vec![(0u16, 0u16); n_clob * 2];
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

    // 4b. Compute loop-depth weighting and ABI / loop-phi / two-address register hints.
    let mut loop_depth = vec![0u8; n_ord];
    let mut rpo_preds: Vec<Vec<u32>> = vec![Vec::new(); n_ord];
    let mut backedges: Vec<(u32, u32)> = Vec::new();
    for (oi, &bid) in block_order.iter().enumerate() {
        for &sid in &succs[bid as usize] {
            let soi = block_pos_in_order[sid as usize];
            if soi == u32::MAX {
                continue;
            }
            rpo_preds[soi as usize].push(oi as u32);
            if soi <= oi as u32 {
                backedges.push((oi as u32, soi));
            }
        }
    }
    if !backedges.is_empty() {
        let mut loop_seen = vec![u32::MAX; n_ord];
        let mut worklist: Vec<u32> = Vec::new();
        for (li, &(latch, header)) in backedges.iter().take(512).enumerate() {
            let stamp = li as u32;
            loop_seen[header as usize] = stamp;
            loop_depth[header as usize] = loop_depth[header as usize].saturating_add(1).min(4);
            if latch != header {
                loop_seen[latch as usize] = stamp;
                loop_depth[latch as usize] = loop_depth[latch as usize].saturating_add(1).min(4);
                worklist.clear();
                worklist.push(latch);
                while let Some(cur) = worklist.pop() {
                    for &p in &rpo_preds[cur as usize] {
                        if p > header && loop_seen[p as usize] != stamp {
                            loop_seen[p as usize] = stamp;
                            loop_depth[p as usize] = loop_depth[p as usize].saturating_add(1).min(4);
                            worklist.push(p);
                        }
                    }
                }
            }
        }
    }

    let mut val_weight = vec![0u32; n_vals];
    let mut hint_gp: Vec<Option<Gp>> = vec![None; n_vals];
    let mut hint_xmm: Vec<Option<Xmm>> = vec![None; n_vals];
    // For coalescing: up to 2 preferred peer values (phi/loop-backedge/two-addr LHS).
    let mut hint_peers: Vec<[u32; 2]> = vec![[u32::MAX, u32::MAX]; n_vals];
    let add_peer = |peers: &mut Vec<[u32; 2]>, a: u32, b: u32| {
        if a == b {
            return;
        }
        let slot = &mut peers[a as usize];
        if slot[0] == u32::MAX || slot[0] == b {
            slot[0] = b;
        } else if slot[1] == u32::MAX {
            slot[1] = b;
        }
    };

    // Hint entry params with their incoming ABI registers.
    if !ir.params.is_empty() {
        let p_tys: Vec<Ty> = ir.params.iter().map(|&pv| ir.val_tys[pv.0 as usize]).collect();
        if let Ok(al) = layout_args(&p_tys) {
            for (&pv, slot) in ir.params.iter().zip(al.slots) {
                match slot {
                    AbiSlot::Gp(g) => hint_gp[pv.0 as usize] = Some(g),
                    AbiSlot::Xmm(x) => hint_xmm[pv.0 as usize] = Some(x),
                    AbiSlot::Stack(_) => {}
                }
            }
        }
    }

    let mut hint_steps: usize = 0;
    for (oi, &bid) in block_order.iter().enumerate() {
        let bb = &ir.blocks[bid as usize];
        let w = match loop_depth[oi] {
            0 => 1u32,
            1 => 10,
            2 => 100,
            3 => 1000,
            _ => 10_000,
        };
        for &p in &bb.params {
            val_weight[p.0 as usize] = val_weight[p.0 as usize].saturating_add(w);
        }
        for inst in &bb.insts {
            hint_steps += 1;
            if let Some(bg) = budget
                && (hint_steps & 4095) == 0
            {
                bg.check_deadline()?;
            }
            inst.op.for_each_use(&ir.val_pool, |v| {
                val_weight[v.0 as usize] = val_weight[v.0 as usize].saturating_add(w);
            });
            if let Some(r) = inst.res {
                val_weight[r.0 as usize] = val_weight[r.0 as usize].saturating_add(w);
                match inst.op {
                    Op::I32Bin(_, a, _)
                    | Op::I64Bin(_, a, _)
                    | Op::I32BinImm(_, a, _)
                    | Op::I64BinImm(_, a, _)
                    | Op::F32Bin(_, a, _)
                    | Op::F64Bin(_, a, _)
                    | Op::Select { a, .. }
                    | Op::CmpSelect { tv: a, .. } => {
                        add_peer(&mut hint_peers, r.0, a.0);
                    }
                    _ => {}
                }
            }
            match inst.op {
                Op::Call { args, rets, .. } | Op::CallIndirect { args, rets, .. } => {
                    scratch_tys.clear();
                    for &av in ir.pool_slice(args) {
                        scratch_tys.push(ir.val_tys[av.0 as usize]);
                    }
                    if let Ok(al) = layout_args(&scratch_tys) {
                        for (&av, slot) in ir.pool_slice(args).iter().zip(al.slots) {
                            match slot {
                                AbiSlot::Gp(g) if hint_gp[av.0 as usize].is_none() => {
                                    hint_gp[av.0 as usize] = Some(g);
                                }
                                AbiSlot::Xmm(x) if hint_xmm[av.0 as usize].is_none() => {
                                    hint_xmm[av.0 as usize] = Some(x);
                                }
                                _ => {}
                            }
                        }
                    }
                    scratch_tys.clear();
                    for &rv in ir.pool_slice(rets) {
                        val_weight[rv.0 as usize] = val_weight[rv.0 as usize].saturating_add(w);
                        scratch_tys.push(ir.val_tys[rv.0 as usize]);
                    }
                    if let Ok(rl) = layout_rets(&scratch_tys) {
                        for (&rv, slot) in ir.pool_slice(rets).iter().zip(rl.slots) {
                            match slot {
                                AbiSlot::Gp(g) => hint_gp[rv.0 as usize] = Some(g),
                                AbiSlot::Xmm(x) => hint_xmm[rv.0 as usize] = Some(x),
                                AbiSlot::Stack(_) => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        bb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
            val_weight[v.0 as usize] = val_weight[v.0 as usize].saturating_add(w);
        });
        let mut wire_edge = |target_bb: u32, args: super::ir::ValRange| {
            let params = &ir.blocks[target_bb as usize].params;
            let avals = ir.pool_slice(args);
            for (&p, &a) in params.iter().zip(avals) {
                add_peer(&mut hint_peers, p.0, a.0);
                add_peer(&mut hint_peers, a.0, p.0);
            }
        };
        match bb.term {
            Term::Jump { target, args } => wire_edge(target.0, args),
            Term::BranchIf { then_bb, then_args, else_bb, else_args, .. }
            | Term::CmpBranch { then_bb, then_args, else_bb, else_args, .. } => {
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
                scratch_tys.clear();
                for &rv in ir.pool_slice(vals) {
                    scratch_tys.push(ir.val_tys[rv.0 as usize]);
                }
                if let Ok(rl) = layout_rets(&scratch_tys) {
                    for (&rv, slot) in ir.pool_slice(vals).iter().zip(rl.slots) {
                        match slot {
                            AbiSlot::Gp(g) if hint_gp[rv.0 as usize].is_none() => {
                                hint_gp[rv.0 as usize] = Some(g);
                            }
                            AbiSlot::Xmm(x) if hint_xmm[rv.0 as usize].is_none() => {
                                hint_xmm[rv.0 as usize] = Some(x);
                            }
                            _ => {}
                        }
                    }
                }
            }
            Term::Unreachable => {}
        }
    }

    let mut gp_intervals: Vec<Interval> = Vec::new();
    let mut fp_intervals: Vec<Interval> = Vec::new();
    for vi in 0..n_vals {
        if let Some(bg) = budget
            && (vi & 4095) == 4095
        {
            bg.check_deadline()?;
        }
        let start = def_pos[vi];
        if start == u32::MAX {
            continue;
        }
        let end = last_use[vi].max(start + 1);
        let clob_end = last_use[vi].max(start);
        let lo = clobbers.partition_point(|c| c.pos <= start);
        let hi = clobbers.partition_point(|c| c.pos < clob_end);
        let (clob_gp, clob_xmm) = query_clobbers(lo, hi);
        let len = (end - start).max(1) as u64;
        let score = (u64::from(val_weight[vi].max(1)) << 12) / len;
        let iv = Interval { val: vi as u32, start, end, ty: ir.val_tys[vi], clob_gp, clob_xmm, score };
        if iv.ty.is_fp() {
            fp_intervals.push(iv);
        } else {
            gp_intervals.push(iv);
        }
    }

    if let Some(bg) = budget {
        bg.check_deadline()?;
    }
    gp_intervals.sort_by_key(|iv| (iv.start, iv.end));
    if let Some(bg) = budget {
        bg.check_deadline()?;
    }
    fp_intervals.sort_by_key(|iv| (iv.start, iv.end));

    let mut locs = vec![Loc::Spill(0); n_vals];
    let mut assigned = vec![false; n_vals];
    let mut next_spill_off: i32 = 0;

    // 5a. Linear scan for GP intervals with hinting, loop-weighted eviction, & O(log S) spill expiry.
    let mut free_spills8: BinaryHeap<Reverse<(u32, i32)>> = BinaryHeap::new();
    // Active entries: `(start, end, reg_idx, val, score)`.
    let mut active_gp: Vec<(u32, u32, usize, u32, u64)> = Vec::with_capacity(GP_ALLOC.len());

    let alloc_spill8 = |start: u32,
                        end: u32,
                        free: &mut BinaryHeap<Reverse<(u32, i32)>>,
                        next: &mut i32|
     -> Result<i32, String> {
        if let Some(&Reverse((e, _))) = free.peek()
            && e <= start
        {
            let Reverse((_, off)) = free.pop().unwrap();
            free.push(Reverse((end, off)));
            Ok(off)
        } else {
            let off = *next;
            let after = off.checked_add(8).ok_or("x64: spill stack size overflow")?;
            if after > MAX_FRAME_BYTES {
                return Err(format!("x64: spill area ({after} B) exceeds frame cap {MAX_FRAME_BYTES} B"));
            }
            *next = after;
            free.push(Reverse((end, off)));
            Ok(off)
        }
    };

    for (idx, iv) in gp_intervals.iter().enumerate() {
        if let Some(bg) = budget
            && (idx & 4095) == 4095
        {
            bg.check_deadline()?;
        }
        active_gp.retain(|&(_, e, _, _, _)| e > iv.start);
        let mut used_regs = 0u16;
        for &(_, _, ri, _, _) in &active_gp {
            used_regs |= 1 << GP_ALLOC[ri].id();
        }
        let forbidden = used_regs | iv.clob_gp;

        // Try coalescing peer register first, then ABI hint register, then first free in GP_ALLOC.
        let mut chosen: Option<(usize, Gp)> = None;
        for &peer in &hint_peers[iv.val as usize] {
            if peer != u32::MAX && assigned[peer as usize]
                && let Loc::Gp(g) = locs[peer as usize]
                && (forbidden & (1 << g.id())) == 0
                && let Some(ri) = GP_ALLOC.iter().position(|&x| x == g)
            {
                chosen = Some((ri, g));
                break;
            }
        }
        if chosen.is_none()
            && let Some(g) = hint_gp[iv.val as usize]
            && (forbidden & (1 << g.id())) == 0
            && let Some(ri) = GP_ALLOC.iter().position(|&x| x == g)
        {
            chosen = Some((ri, g));
        }
        if chosen.is_none() {
            chosen = GP_ALLOC
                .iter()
                .enumerate()
                .find(|&(_, &g)| (forbidden & (1 << g.id())) == 0)
                .map(|(ri, &g)| (ri, g));
        }

        if let Some((ri, g)) = chosen {
            locs[iv.val as usize] = Loc::Gp(g);
            assigned[iv.val as usize] = true;
            active_gp.push((iv.start, iv.end, ri, iv.val, iv.score));
        } else {
            // Evict the active candidate with lowest spill score (tie-break: furthest end).
            let best_evict = active_gp
                .iter()
                .enumerate()
                .filter(|&(_, &(_, e, ri, _, sc))| {
                    (iv.clob_gp & (1 << GP_ALLOC[ri].id())) == 0
                        && (sc < iv.score || (sc == iv.score && e > iv.end))
                })
                .min_by(|&(_, &(_, ea, _, _, sa)), &(_, &(_, eb, _, _, sb))| {
                    sa.cmp(&sb).then_with(|| eb.cmp(&ea))
                })
                .map(|(ai, _)| ai);
            if let Some(ai) = best_evict {
                let (ev_start, ev_end, ri, ev_val, _) = active_gp.swap_remove(ai);
                let g = GP_ALLOC[ri];
                let sp = alloc_spill8(ev_start, ev_end, &mut free_spills8, &mut next_spill_off)?;
                locs[ev_val as usize] = Loc::Spill(sp);
                locs[iv.val as usize] = Loc::Gp(g);
                assigned[iv.val as usize] = true;
                active_gp.push((iv.start, iv.end, ri, iv.val, iv.score));
            } else {
                let sp = alloc_spill8(iv.start, iv.end, &mut free_spills8, &mut next_spill_off)?;
                locs[iv.val as usize] = Loc::Spill(sp);
                assigned[iv.val as usize] = true;
            }
        }
    }

    // 5b. Linear scan for FP/V128 intervals (caller-saved Xmm0..Xmm7 + callee-saved Xmm8..Xmm13).
    let mut free_spills16: BinaryHeap<Reverse<(u32, i32)>> = BinaryHeap::new();
    let mut active_fp: Vec<(u32, u32, usize, u32, u64)> = Vec::with_capacity(XMM_ALLOC.len());

    let alloc_spill_fp = |ty: Ty,
                          start: u32,
                          end: u32,
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
                let after = off.checked_add(16).ok_or("x64: spill stack size overflow")?;
                if after > MAX_FRAME_BYTES {
                    return Err(format!("x64: spill area ({after} B) exceeds frame cap {MAX_FRAME_BYTES} B"));
                }
                *next = after;
                free16.push(Reverse((end, off)));
                Ok(off)
            }
        } else {
            alloc_spill8(start, end, free8, next)
        }
    };

    for (idx, iv) in fp_intervals.iter().enumerate() {
        if let Some(bg) = budget
            && (idx & 4095) == 4095
        {
            bg.check_deadline()?;
        }
        active_fp.retain(|&(_, e, _, _, _)| e > iv.start);
        let mut used_regs = 0u16;
        for &(_, _, ri, _, _) in &active_fp {
            used_regs |= 1 << XMM_ALLOC[ri].id();
        }
        let forbidden = used_regs | iv.clob_xmm;

        let mut chosen: Option<(usize, Xmm)> = None;
        for &peer in &hint_peers[iv.val as usize] {
            if peer != u32::MAX && assigned[peer as usize]
                && let Loc::Xmm(x) = locs[peer as usize]
                && (forbidden & (1 << x.id())) == 0
                && let Some(ri) = XMM_ALLOC.iter().position(|&r| r == x)
            {
                chosen = Some((ri, x));
                break;
            }
        }
        if chosen.is_none()
            && let Some(x) = hint_xmm[iv.val as usize]
            && (forbidden & (1 << x.id())) == 0
            && let Some(ri) = XMM_ALLOC.iter().position(|&r| r == x)
        {
            chosen = Some((ri, x));
        }
        if chosen.is_none() {
            chosen = XMM_ALLOC
                .iter()
                .enumerate()
                .find(|&(_, &x)| (forbidden & (1 << x.id())) == 0)
                .map(|(ri, &x)| (ri, x));
        }

        if let Some((ri, x)) = chosen {
            locs[iv.val as usize] = Loc::Xmm(x);
            assigned[iv.val as usize] = true;
            active_fp.push((iv.start, iv.end, ri, iv.val, iv.score));
        } else {
            let best_evict = active_fp
                .iter()
                .enumerate()
                .filter(|&(_, &(_, e, ri, _, sc))| {
                    (iv.clob_xmm & (1 << XMM_ALLOC[ri].id())) == 0
                        && (sc < iv.score || (sc == iv.score && e > iv.end))
                })
                .min_by(|&(_, &(_, ea, _, _, sa)), &(_, &(_, eb, _, _, sb))| {
                    sa.cmp(&sb).then_with(|| eb.cmp(&ea))
                })
                .map(|(ai, _)| ai);
            if let Some(ai) = best_evict {
                let (ev_start, ev_end, ri, ev_val, _) = active_fp.swap_remove(ai);
                let x = XMM_ALLOC[ri];
                let ev_ty = ir.val_tys[ev_val as usize];
                let sp = alloc_spill_fp(ev_ty, ev_start, ev_end, &mut free_spills8, &mut free_spills16, &mut next_spill_off)?;
                locs[ev_val as usize] = Loc::Spill(sp);
                locs[iv.val as usize] = Loc::Xmm(x);
                assigned[iv.val as usize] = true;
                active_fp.push((iv.start, iv.end, ri, iv.val, iv.score));
            } else {
                let sp = alloc_spill_fp(iv.ty, iv.start, iv.end, &mut free_spills8, &mut free_spills16, &mut next_spill_off)?;
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
            Loc::Gp(g) if matches!(g, Gp::Rbx | Gp::Rbp) => used_callee_gp_mask |= 1 << g.id(),
            Loc::Xmm(x) if x.id() >= 8 => used_callee_xmm_mask |= 1 << x.id(),
            _ => {}
        }
    }
    let mut used_callee_gp = Vec::new();
    for &g in &GP_CALLEE_SAVED {
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
                *off = off.checked_add(spill_base).ok_or("x64: spill offset overflow")?;
            }
        }
    }
    let mut raw_frame = spill_base.checked_add(next_spill_off).ok_or("x64: stack frame size overflow")?;
    let saved_xmm_base = if !used_callee_xmm.is_empty() {
        let base = checked_align_i32(raw_frame, 16)?;
        let xmm_bytes = (used_callee_xmm.len() as i32) * 16;
        raw_frame = base.checked_add(xmm_bytes).ok_or("x64: stack frame size overflow")?;
        base
    } else {
        0
    };
    let self_ret_layout = layout_rets(&ir.result_tys)?;
    let self_param_tys: Vec<Ty> = ir.params.iter().map(|&pv| ir.val_tys[pv.0 as usize]).collect();
    let self_param_layout = layout_args(&self_param_tys)?;
    let saved_ret_ptr_off = if self_ret_layout.stack_bytes > 0 {
        raw_frame = raw_frame.checked_add(7).ok_or("x64: stack frame size overflow")? & !7;
        let off = raw_frame;
        raw_frame = raw_frame.checked_add(8).ok_or("x64: stack frame size overflow")?;
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
        .and_then(|v| v.checked_add(self_param_layout.stack_bytes))
        .ok_or("x64: incoming stack parameter offset overflow")?;
    if incoming_stack_base > MAX_FRAME_BYTES.saturating_mul(2) {
        return Err(format!("x64: function {} incoming stack offset ({incoming_stack_base} B) exceeds cap", ir.func_index));
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
    })
}
