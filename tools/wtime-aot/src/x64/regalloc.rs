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
//! - Allocatable XMM (14 regs, all caller-saved in Stage 1): `Xmm0..Xmm13`

use super::asm::{Gp, Xmm};
use super::ir::{IntBinOp, IrFunc, Op, Ty};
use super::module::{MAX_FRAME_BYTES, MAX_LIVENESS_BYTES};

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
/// Bitmask over `Xmm::id()` (bits 0..13).
pub const MASK_XMM_CALLER: u16 = (1 << 14) - 1;

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
}

/// Computes RPO order of reachable blocks, exact cross-block liveness, and linear-scan allocation.
pub fn allocate(ir: &IrFunc) -> Result<AllocResult, String> {
    let n_blocks = ir.blocks.len();
    let n_vals = ir.val_tys.len();

    // 1. Compute reachable block order (iterative DFS postorder -> reversed = RPO).
    let mut visited = vec![false; n_blocks];
    let mut postorder = Vec::with_capacity(n_blocks);
    let mut dfs: Vec<(u32, usize)> = vec![(0, 0)];
    let mut succs: Vec<Vec<u32>> = vec![Vec::new(); n_blocks];
    for (bi, bb) in ir.blocks.iter().enumerate() {
        bb.term.for_each_succ(&ir.br_tables, |s| {
            let sid = s.0;
            if !succs[bi].contains(&sid) {
                succs[bi].push(sid);
            }
        });
    }
    visited[0] = true;
    while let Some((b, si)) = dfs.last_mut() {
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
                Op::I32BinImm(op, _, _) | Op::I64BinImm(op, _, _)
                    if matches!(op, IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU) =>
                {
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

        for (oi, &bid) in block_order.iter().enumerate() {
            let bb = &ir.blocks[bid as usize];
            for &p in &bb.params {
                let c = cb_id[p.0 as usize];
                if c != u32::MAX {
                    set_bit(&mut live_def, oi, c as usize);
                }
            }
            for inst in &bb.insts {
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

        let mut changed = true;
        while changed {
            changed = false;
            for oi in (0..n_ord).rev() {
                let bid = block_order[oi] as usize;
                for &sid in &succs[bid] {
                    let soi = block_pos_in_order[sid as usize];
                    if soi == u32::MAX {
                        continue;
                    }
                    let soi = soi as usize;
                    for w in 0..words {
                        let old = live_out[oi * words + w];
                        let nv = old | live_in[soi * words + w];
                        if nv != old {
                            live_out[oi * words + w] = nv;
                            changed = true;
                        }
                    }
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

    // 4. Build intervals and compute exact clobber masks for each interval.
    let mut gp_intervals: Vec<Interval> = Vec::new();
    let mut fp_intervals: Vec<Interval> = Vec::new();
    for vi in 0..n_vals {
        let start = def_pos[vi];
        if start == u32::MAX {
            continue;
        }
        let end = last_use[vi].max(start + 1);
        let clob_end = last_use[vi].max(start);
        let lo = clobbers.partition_point(|c| c.pos <= start);
        let mut clob_gp = 0u16;
        let mut clob_xmm = 0u16;
        for c in &clobbers[lo..] {
            if c.pos >= clob_end {
                break;
            }
            clob_gp |= c.gp_mask;
            clob_xmm |= c.xmm_mask;
        }
        let iv = Interval { val: vi as u32, start, end, ty: ir.val_tys[vi], clob_gp, clob_xmm };
        if iv.ty.is_fp() {
            fp_intervals.push(iv);
        } else {
            gp_intervals.push(iv);
        }
    }

    gp_intervals.sort_by_key(|iv| (iv.start, iv.end));
    fp_intervals.sort_by_key(|iv| (iv.start, iv.end));

    let mut locs = vec![Loc::Spill(0); n_vals];
    let mut next_spill_off: i32 = 0;
    let mut used_callee_mask: u16 = 0;

    // 5a. Linear scan for GP intervals.
    let mut free_spills8: Vec<(u32, i32)> = Vec::new();
    let mut active_gp: Vec<(u32, u32, usize, u32)> = Vec::with_capacity(GP_ALLOC.len());

    let alloc_spill8 = |start: u32, end: u32, free: &mut Vec<(u32, i32)>, next: &mut i32| -> Result<i32, String> {
        if let Some(pos) = free.iter().position(|&(e, _)| e <= start) {
            let (_, off) = free.swap_remove(pos);
            free.push((end, off));
            Ok(off)
        } else {
            let off = *next;
            let after = off.checked_add(8).ok_or("x64: spill stack size overflow")?;
            if after > MAX_FRAME_BYTES {
                return Err(format!("x64: spill area ({after} B) exceeds frame cap {MAX_FRAME_BYTES} B"));
            }
            *next = after;
            free.push((end, off));
            Ok(off)
        }
    };

    for iv in &gp_intervals {
        active_gp.retain(|&(_, e, _, _)| e > iv.start);
        let mut used_regs = 0u16;
        for &(_, _, ri, _) in &active_gp {
            used_regs |= 1 << GP_ALLOC[ri].id();
        }
        let forbidden = used_regs | iv.clob_gp;
        let chosen = GP_ALLOC.iter().enumerate().find(|&(_, &g)| (forbidden & (1 << g.id())) == 0);
        if let Some((ri, &g)) = chosen {
            locs[iv.val as usize] = Loc::Gp(g);
            if matches!(g, Gp::Rbx | Gp::Rbp) {
                used_callee_mask |= 1 << g.id();
            }
            active_gp.push((iv.start, iv.end, ri, iv.val));
        } else {
            let best_evict = active_gp
                .iter()
                .enumerate()
                .filter(|&(_, &(_, e, ri, _))| e > iv.end && (iv.clob_gp & (1 << GP_ALLOC[ri].id())) == 0)
                .max_by_key(|&(_, &(_, e, _, _))| e)
                .map(|(ai, _)| ai);
            if let Some(ai) = best_evict {
                let (ev_start, ev_end, ri, ev_val) = active_gp.swap_remove(ai);
                let g = GP_ALLOC[ri];
                let sp = alloc_spill8(ev_start, ev_end, &mut free_spills8, &mut next_spill_off)?;
                locs[ev_val as usize] = Loc::Spill(sp);
                locs[iv.val as usize] = Loc::Gp(g);
                if matches!(g, Gp::Rbx | Gp::Rbp) {
                    used_callee_mask |= 1 << g.id();
                }
                active_gp.push((iv.start, iv.end, ri, iv.val));
            } else {
                let sp = alloc_spill8(iv.start, iv.end, &mut free_spills8, &mut next_spill_off)?;
                locs[iv.val as usize] = Loc::Spill(sp);
            }
        }
    }

    // 5b. Linear scan for FP/V128 intervals.
    let mut free_spills16: Vec<(u32, i32)> = Vec::new();
    let mut active_fp: Vec<(u32, u32, usize, u32)> = Vec::with_capacity(XMM_ALLOC.len());

    let alloc_spill_fp = |ty: Ty,
                          start: u32,
                          end: u32,
                          free8: &mut Vec<(u32, i32)>,
                          free16: &mut Vec<(u32, i32)>,
                          next: &mut i32|
     -> Result<i32, String> {
        if ty == Ty::V128 {
            if let Some(pos) = free16.iter().position(|&(e, _)| e <= start) {
                let (_, off) = free16.swap_remove(pos);
                free16.push((end, off));
                Ok(off)
            } else {
                let off = checked_align_i32(*next, 16)?;
                let after = off.checked_add(16).ok_or("x64: spill stack size overflow")?;
                if after > MAX_FRAME_BYTES {
                    return Err(format!("x64: spill area ({after} B) exceeds frame cap {MAX_FRAME_BYTES} B"));
                }
                *next = after;
                free16.push((end, off));
                Ok(off)
            }
        } else {
            alloc_spill8(start, end, free8, next)
        }
    };

    for iv in &fp_intervals {
        active_fp.retain(|&(_, e, _, _)| e > iv.start);
        let mut used_regs = 0u16;
        for &(_, _, ri, _) in &active_fp {
            used_regs |= 1 << XMM_ALLOC[ri].id();
        }
        let forbidden = used_regs | iv.clob_xmm;
        let chosen = XMM_ALLOC.iter().enumerate().find(|&(_, &x)| (forbidden & (1 << x.id())) == 0);
        if let Some((ri, &x)) = chosen {
            locs[iv.val as usize] = Loc::Xmm(x);
            active_fp.push((iv.start, iv.end, ri, iv.val));
        } else {
            let best_evict = active_fp
                .iter()
                .enumerate()
                .filter(|&(_, &(_, e, ri, _))| e > iv.end && (iv.clob_xmm & (1 << XMM_ALLOC[ri].id())) == 0)
                .max_by_key(|&(_, &(_, e, _, _))| e)
                .map(|(ai, _)| ai);
            if let Some(ai) = best_evict {
                let (ev_start, ev_end, ri, ev_val) = active_fp.swap_remove(ai);
                let x = XMM_ALLOC[ri];
                let ev_ty = ir.val_tys[ev_val as usize];
                let sp = alloc_spill_fp(ev_ty, ev_start, ev_end, &mut free_spills8, &mut free_spills16, &mut next_spill_off)?;
                locs[ev_val as usize] = Loc::Spill(sp);
                locs[iv.val as usize] = Loc::Xmm(x);
                active_fp.push((iv.start, iv.end, ri, iv.val));
            } else {
                let sp = alloc_spill_fp(iv.ty, iv.start, iv.end, &mut free_spills8, &mut free_spills16, &mut next_spill_off)?;
                locs[iv.val as usize] = Loc::Spill(sp);
            }
        }
    }

    // 6. Compute aligned stack frame layout:
    //   [rsp + 0 .. outgoing_arg_bytes]                     : outgoing stack args (16-byte aligned)
    //   [rsp + ret_buf_off .. ret_buf_off + ret_buf_bytes]  : callee overflow return buffer (16-byte aligned)
    //   [rsp + spill_base .. spill_base + next_spill_off]   : SSA spills (16-byte aligned base for V128)
    //   [rsp + saved_ret_ptr_off]                           : optional saved caller ret_spill_ptr (8 bytes)
    let mut used_callee_gp = Vec::new();
    for &g in &GP_CALLEE_SAVED {
        if (used_callee_mask & (1 << g.id())) != 0 {
            used_callee_gp.push(g);
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
        frame_size,
        outgoing_arg_bytes,
        ret_buf_off,
        ret_buf_bytes,
        saved_ret_ptr_off,
    })
}
