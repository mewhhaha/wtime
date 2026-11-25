//! AArch64 linear-scan register allocator & internal Wasm ABI layout.
//!
//! Register partition:
//! - Pinned:
//!   - `SP`: 16-byte aligned native stack pointer
//!   - `FP` (`X29`), `LR` (`X30`): frame & link registers
//!   - `X28`: `Ctx*`
//!   - `X27`: `mem_base`
//!   - `X26`: `mem_len`
//!   - `X25`: optional pinned hottest scalar global
//! - Scratch (never allocated):
//!   - `X16`: primary scratch & trap status return (`W16`: `0`=ok, `1..=22`=trap)
//!   - `X17`: secondary scratch & parallel-move cycle break
//!   - `X24`: tertiary scratch (memory-to-memory move temporary & wide offset materialization)
//!   - `V30`, `V31`: FP/V128 scratch registers
//! - Allocatable GP (21 regs!):
//!   - Caller-saved (16): `X0..X15`
//!   - Callee-saved (5): `X19..X23`
//! - Allocatable FP/V128 (30 regs!):
//!   - Caller-saved (22): `V0..V7`, `V16..V29`
//!   - Callee-saved (8): `V8..V15` (full 128-bit saved in internal Wasm convention!)

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::asm::{Reg, VReg};
use crate::x64::ir::{IrFunc, Op, Term, Ty};
use crate::x64::module::{CompileBudget, MAX_FRAME_BYTES};

pub const CALL_GP_ARGS: [Reg; 8] = [
    Reg::X0,
    Reg::X1,
    Reg::X2,
    Reg::X3,
    Reg::X4,
    Reg::X5,
    Reg::X6,
    Reg::X7,
];
pub const CALL_FP_ARGS: [VReg; 8] = [
    VReg::V0,
    VReg::V1,
    VReg::V2,
    VReg::V3,
    VReg::V4,
    VReg::V5,
    VReg::V6,
    VReg::V7,
];

pub const GP_ALLOC: [Reg; 21] = [
    Reg::X0,
    Reg::X1,
    Reg::X2,
    Reg::X3,
    Reg::X4,
    Reg::X5,
    Reg::X6,
    Reg::X7,
    Reg::X8,
    Reg::X9,
    Reg::X10,
    Reg::X11,
    Reg::X12,
    Reg::X13,
    Reg::X14,
    Reg::X15,
    Reg::X19,
    Reg::X20,
    Reg::X21,
    Reg::X22,
    Reg::X23,
];

pub const FP_ALLOC: [VReg; 30] = [
    VReg::V0,
    VReg::V1,
    VReg::V2,
    VReg::V3,
    VReg::V4,
    VReg::V5,
    VReg::V6,
    VReg::V7,
    VReg::V16,
    VReg::V17,
    VReg::V18,
    VReg::V19,
    VReg::V20,
    VReg::V21,
    VReg::V22,
    VReg::V23,
    VReg::V24,
    VReg::V25,
    VReg::V26,
    VReg::V27,
    VReg::V28,
    VReg::V29,
    VReg::V8,
    VReg::V9,
    VReg::V10,
    VReg::V11,
    VReg::V12,
    VReg::V13,
    VReg::V14,
    VReg::V15,
];

/// Mask of caller-saved GP regs (`X0..X15`).
pub const MASK_GP_CALLER: u32 = 0x0000_ffff;
/// Mask of caller-saved FP regs (`V0..V7`, `V16..V29`).
pub const MASK_FP_CALLER: u32 = 0x3fff_00ff;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum A64Loc {
    Gp(Reg),
    Fp(VReg),
    Spill(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum A64AbiSlot {
    Gp(Reg),
    Fp(VReg),
    Stack(i32),
}

pub struct A64AbiLayout {
    pub slots: Vec<A64AbiSlot>,
    pub stack_bytes: i32,
}

fn align_i32(v: i32, align: i32) -> Result<i32, String> {
    let m = align - 1;
    v.checked_add(m)
        .map(|x| x & !m)
        .ok_or_else(|| "aarch64: frame offset overflow".to_string())
}

pub fn layout_args(tys: &[Ty]) -> Result<A64AbiLayout, String> {
    let mut slots = Vec::with_capacity(tys.len());
    let (mut ng, mut nf, mut off) = (0usize, 0usize, 0i32);
    for &ty in tys {
        if ty.is_fp() {
            if nf < CALL_FP_ARGS.len() {
                slots.push(A64AbiSlot::Fp(CALL_FP_ARGS[nf]));
                nf += 1;
            } else {
                let (sz, al) = if ty == Ty::V128 { (16, 16) } else { (8, 8) };
                off = align_i32(off, al)?;
                slots.push(A64AbiSlot::Stack(off));
                off = off.checked_add(sz).ok_or("aarch64: arg stack overflow")?;
            }
        } else if ng < CALL_GP_ARGS.len() {
            slots.push(A64AbiSlot::Gp(CALL_GP_ARGS[ng]));
            ng += 1;
        } else {
            off = align_i32(off, 8)?;
            slots.push(A64AbiSlot::Stack(off));
            off = off.checked_add(8).ok_or("aarch64: arg stack overflow")?;
        }
    }
    Ok(A64AbiLayout {
        slots,
        stack_bytes: align_i32(off, 16)?,
    })
}

pub fn layout_rets(tys: &[Ty]) -> Result<A64AbiLayout, String> {
    layout_args(tys)
}

pub struct A64AllocResult {
    pub locs: Vec<A64Loc>,
    pub block_order: Vec<u32>,
    pub used_callee_gp: Vec<Reg>,
    pub used_callee_fp: Vec<VReg>,
    pub saved_gp_base: i32,
    pub saved_fp_base: i32,
    pub ret_buf_off: i32,
    pub saved_ret_ptr_off: Option<i32>,
    pub frame_size: i32,
    pub has_calls: bool,
}

#[derive(Clone, Copy)]
struct Interval {
    val: u32,
    start: u32,
    end: u32,
    ty: Ty,
    clob_gp: u32,
    clob_fp: u32,
}

pub fn allocate(ir: &IrFunc, budget: Option<&CompileBudget>) -> Result<A64AllocResult, String> {
    let n_blocks = ir.blocks.len();
    let n_vals = ir.val_tys.len();

    // 1. RPO block order over reachable blocks.
    let mut succs: Vec<Vec<u32>> = vec![Vec::new(); n_blocks];
    for (bi, bb) in ir.blocks.iter().enumerate() {
        match bb.term {
            Term::Jump { target, .. } => succs[bi].push(target.0),
            Term::BranchIf {
                then_bb, else_bb, ..
            }
            | Term::CmpBranch {
                then_bb, else_bb, ..
            } => {
                succs[bi].push(then_bb.0);
                if else_bb != then_bb {
                    succs[bi].push(else_bb.0);
                }
            }
            Term::BrTable { table_idx, .. } => {
                for &(t, _) in &ir.br_tables[table_idx as usize].unique_edges {
                    if !succs[bi].contains(&t.0) {
                        succs[bi].push(t.0);
                    }
                }
            }
            Term::Return { .. }
            | Term::ReturnCall { .. }
            | Term::ReturnCallIndirect { .. }
            | Term::Unreachable => {}
        }
    }
    let mut visited = vec![false; n_blocks];
    let mut post = Vec::with_capacity(n_blocks);
    let mut dfs = vec![(0u32, 0usize)];
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
            post.push(*b);
            dfs.pop();
        }
    }
    post.reverse();
    // Cold trap/unreachable block identification + backward propagation in reverse RPO,
    // sinking cold trap/abort blocks out of hot loops to the tail of the function.
    let mut is_trap_blk = vec![false; n_blocks];
    for &b in &post {
        let bb = &ir.blocks[b as usize];
        let mut trap = matches!(bb.term, Term::Unreachable);
        if !trap {
            for inst in &bb.insts {
                if matches!(inst.op, Op::Trap(_) | Op::MemGrow(_)) {
                    trap = true;
                    break;
                }
            }
        }
        is_trap_blk[b as usize] = trap;
    }
    for &b in post.iter().rev() {
        let bu = b as usize;
        if b == 0 || is_trap_blk[bu] {
            continue;
        }
        if !succs[bu].is_empty() && succs[bu].iter().all(|&s| is_trap_blk[s as usize]) {
            is_trap_blk[bu] = true;
        }
    }
    let mut block_order = Vec::with_capacity(post.len());
    for &b in &post {
        if b == 0 || !is_trap_blk[b as usize] {
            block_order.push(b);
        }
    }
    for &b in &post {
        if b != 0 && is_trap_blk[b as usize] {
            block_order.push(b);
        }
    }
    let n_ord = block_order.len();
    let mut ord_of = vec![u32::MAX; n_blocks];
    for (oi, &b) in block_order.iter().enumerate() {
        ord_of[b as usize] = oi as u32;
    }

    // 2. Linear program points, call/helper clobbers, and outgoing stack-arg/return-buffer sizes.
    let mut block_start = vec![0u32; n_ord];
    let mut block_end = vec![0u32; n_ord];
    let mut def_pos = vec![u32::MAX; n_vals];
    let mut last_use = vec![0u32; n_vals];
    let mut clobbers: Vec<(u32, u32, u32)> = Vec::new();
    let mut max_out_args = 0i32;
    let mut max_ret_buf = 0i32;
    let mut has_calls = false;
    let self_param_tys: Vec<Ty> = ir
        .params
        .iter()
        .map(|&pv| ir.val_tys[pv.0 as usize])
        .collect();
    let self_param_stack = layout_args(&self_param_tys)?.stack_bytes;
    let mut pos = 0u32;

    for (oi, &bid) in block_order.iter().enumerate() {
        block_start[oi] = pos;
        let bb = &ir.blocks[bid as usize];
        for &p in &bb.params {
            def_pos[p.0 as usize] = pos;
            last_use[p.0 as usize] = last_use[p.0 as usize].max(pos + 1);
        }
        pos += 2;
        for inst in &bb.insts {
            inst.op.for_each_use(&ir.val_pool, |v| {
                last_use[v.0 as usize] = last_use[v.0 as usize].max(pos);
            });
            match inst.op {
                Op::Call { args, rets, .. } | Op::CallIndirect { args, rets, .. } => {
                    has_calls = true;
                    clobbers.push((pos, MASK_GP_CALLER, MASK_FP_CALLER));
                    let arg_tys: Vec<Ty> = ir
                        .pool_slice(args)
                        .iter()
                        .map(|&v| ir.val_tys[v.0 as usize])
                        .collect();
                    let ret_tys: Vec<Ty> = ir
                        .pool_slice(rets)
                        .iter()
                        .map(|&v| ir.val_tys[v.0 as usize])
                        .collect();
                    max_out_args = max_out_args.max(layout_args(&arg_tys)?.stack_bytes);
                    max_ret_buf = max_ret_buf.max(layout_rets(&ret_tys)?.stack_bytes);
                    for &rv in ir.pool_slice(rets) {
                        def_pos[rv.0 as usize] = pos + 1;
                        last_use[rv.0 as usize] = last_use[rv.0 as usize].max(pos + 2);
                    }
                }
                Op::MemGrow(_) | Op::MemCopy { .. } | Op::MemFill { .. } => {
                    has_calls = true;
                    clobbers.push((pos, MASK_GP_CALLER, 0));
                }
                _ => {}
            }
            if let Some(r) = inst.res {
                def_pos[r.0 as usize] = pos + 1;
                last_use[r.0 as usize] = last_use[r.0 as usize].max(pos + 2);
            }
            pos += 2;
        }
        bb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
            last_use[v.0 as usize] = last_use[v.0 as usize].max(pos);
        });
        match bb.term {
            Term::ReturnCall { args, .. } | Term::ReturnCallIndirect { args, .. } => {
                let arg_tys: Vec<Ty> = ir
                    .pool_slice(args)
                    .iter()
                    .map(|&v| ir.val_tys[v.0 as usize])
                    .collect();
                let arg_stack = layout_args(&arg_tys)?.stack_bytes;
                if arg_stack > self_param_stack {
                    has_calls = true;
                    max_out_args = max_out_args.max(arg_stack);
                }
            }
            _ => {}
        }
        pos += 2;
        block_end[oi] = pos;
    }

    // 3. Exact backward dataflow liveness over all values to extend intervals across loops/branches.
    let words = n_vals.div_ceil(64).max(1);
    let mut live_use = vec![0u64; n_ord * words];
    let mut live_def = vec![0u64; n_ord * words];
    let mut live_in = vec![0u64; n_ord * words];
    let mut live_out = vec![0u64; n_ord * words];
    for (oi, &bid) in block_order.iter().enumerate() {
        let bb = &ir.blocks[bid as usize];
        let base = oi * words;
        for &p in &bb.params {
            let v = p.0 as usize;
            live_def[base + (v >> 6)] |= 1u64 << (v & 63);
        }
        for inst in &bb.insts {
            inst.op.for_each_use(&ir.val_pool, |v| {
                let vi = v.0 as usize;
                if (live_def[base + (vi >> 6)] & (1u64 << (vi & 63))) == 0 {
                    live_use[base + (vi >> 6)] |= 1u64 << (vi & 63);
                }
            });
            if let Some(r) = inst.res {
                let vi = r.0 as usize;
                live_def[base + (vi >> 6)] |= 1u64 << (vi & 63);
            }
            match inst.op {
                Op::Call { rets, .. } | Op::CallIndirect { rets, .. } => {
                    for &r in ir.pool_slice(rets) {
                        let vi = r.0 as usize;
                        live_def[base + (vi >> 6)] |= 1u64 << (vi & 63);
                    }
                }
                _ => {}
            }
        }
        bb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
            let vi = v.0 as usize;
            if (live_def[base + (vi >> 6)] & (1u64 << (vi & 63))) == 0 {
                live_use[base + (vi >> 6)] |= 1u64 << (vi & 63);
            }
        });
    }
    let mut changed = true;
    while changed {
        if let Some(bg) = budget {
            bg.check_deadline()?;
        }
        changed = false;
        for oi in (0..n_ord).rev() {
            let bid = block_order[oi] as usize;
            let base = oi * words;
            for &sid in &succs[bid] {
                let soi = ord_of[sid as usize];
                if soi == u32::MAX {
                    continue;
                }
                let sbase = (soi as usize) * words;
                for w in 0..words {
                    let nv = live_out[base + w] | live_in[sbase + w];
                    if nv != live_out[base + w] {
                        live_out[base + w] = nv;
                        changed = true;
                    }
                }
            }
            for w in 0..words {
                let nv = live_use[base + w] | (live_out[base + w] & !live_def[base + w]);
                if nv != live_in[base + w] {
                    live_in[base + w] = nv;
                    changed = true;
                }
            }
        }
    }
    let mut iv_start = def_pos.clone();
    let mut iv_end = last_use;
    for oi in 0..n_ord {
        let base = oi * words;
        let bs = block_start[oi];
        let be = block_end[oi];
        for w in 0..words {
            let mut bits = live_in[base + w] | live_out[base + w];
            while bits != 0 {
                let tz = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let vi = (w << 6) + tz;
                if vi < n_vals && def_pos[vi] != u32::MAX {
                    if (live_in[base + w] & (1u64 << tz)) != 0 && bs < iv_start[vi] {
                        iv_start[vi] = bs;
                    }
                    if (live_out[base + w] & (1u64 << tz)) != 0 && be > iv_end[vi] {
                        iv_end[vi] = be;
                    }
                }
            }
        }
    }

    // 4. Build GP and FP intervals with exact clobber masks.
    let mut gp_ivs = Vec::new();
    let mut fp_ivs = Vec::new();
    for vi in 0..n_vals {
        let s = iv_start[vi];
        if s == u32::MAX {
            continue;
        }
        let e = iv_end[vi].max(s + 1);
        let mut cgp = 0u32;
        let mut cfp = 0u32;
        let lo = clobbers.partition_point(|&(cp, _, _)| cp <= s);
        for &(cp, gm, fm) in &clobbers[lo..] {
            if cp >= e {
                break;
            }
            cgp |= gm;
            cfp |= fm;
        }
        let ty = ir.val_tys[vi];
        let iv = Interval {
            val: vi as u32,
            start: s,
            end: e,
            ty,
            clob_gp: cgp,
            clob_fp: cfp,
        };
        if ty.is_fp() {
            fp_ivs.push(iv);
        } else {
            gp_ivs.push(iv);
        }
    }
    gp_ivs.sort_unstable_by_key(|iv| (iv.start, iv.end, iv.val));
    fp_ivs.sort_unstable_by_key(|iv| (iv.start, iv.end, iv.val));

    // 5. Linear scan for GP and FP/V128.
    let mut locs = vec![A64Loc::Gp(Reg::X0); n_vals];
    let mut assigned = vec![false; n_vals];
    let mut next_spill = 0i32;
    let mut free8: BinaryHeap<Reverse<(u32, i32)>> = BinaryHeap::new();
    let mut free16: BinaryHeap<Reverse<(u32, i32)>> = BinaryHeap::new();

    let alloc_spill = |ty: Ty,
                       start: u32,
                       end: u32,
                       free8: &mut BinaryHeap<Reverse<(u32, i32)>>,
                       free16: &mut BinaryHeap<Reverse<(u32, i32)>>,
                       next: &mut i32|
     -> Result<i32, String> {
        let (heap, sz, al) = if ty == Ty::V128 {
            (free16, 16, 16)
        } else {
            (free8, 8, 8)
        };
        if let Some(&Reverse((e, _))) = heap.peek()
            && e <= start
        {
            let Reverse((_, off)) = heap.pop().unwrap();
            heap.push(Reverse((end, off)));
            Ok(off)
        } else {
            let off = align_i32(*next, al)?;
            let after = off.checked_add(sz).ok_or("aarch64: spill overflow")?;
            if after > MAX_FRAME_BYTES {
                return Err(format!("aarch64: spill area exceeds {MAX_FRAME_BYTES} B"));
            }
            *next = after;
            heap.push(Reverse((end, off)));
            Ok(off)
        }
    };

    // GP scan
    let mut active_gp: Vec<(u32, usize, u32)> = Vec::new(); // (end, reg_idx, val)
    for iv in &gp_ivs {
        active_gp.retain(|a| a.0 > iv.start);
        let mut used = 0u32;
        for &(_, ri, _) in &active_gp {
            used |= 1u32 << GP_ALLOC[ri].0;
        }
        let forbid = used | iv.clob_gp;
        if let Some((ri, &r)) = GP_ALLOC
            .iter()
            .enumerate()
            .find(|&(_, &r)| (forbid & (1u32 << r.0)) == 0)
        {
            locs[iv.val as usize] = A64Loc::Gp(r);
            assigned[iv.val as usize] = true;
            active_gp.push((iv.end, ri, iv.val));
        } else {
            let sp = alloc_spill(
                iv.ty,
                iv.start,
                iv.end,
                &mut free8,
                &mut free16,
                &mut next_spill,
            )?;
            locs[iv.val as usize] = A64Loc::Spill(sp);
            assigned[iv.val as usize] = true;
        }
    }

    // FP / V128 scan
    let mut active_fp: Vec<(u32, usize, u32)> = Vec::new();
    for iv in &fp_ivs {
        active_fp.retain(|a| a.0 > iv.start);
        let mut used = 0u32;
        for &(_, ri, _) in &active_fp {
            used |= 1u32 << FP_ALLOC[ri].0;
        }
        let forbid = used | iv.clob_fp;
        if let Some((ri, &v)) = FP_ALLOC
            .iter()
            .enumerate()
            .find(|&(_, &v)| (forbid & (1u32 << v.0)) == 0)
        {
            locs[iv.val as usize] = A64Loc::Fp(v);
            assigned[iv.val as usize] = true;
            active_fp.push((iv.end, ri, iv.val));
        } else {
            let sp = alloc_spill(
                iv.ty,
                iv.start,
                iv.end,
                &mut free8,
                &mut free16,
                &mut next_spill,
            )?;
            locs[iv.val as usize] = A64Loc::Spill(sp);
            assigned[iv.val as usize] = true;
        }
    }

    // 6. Compute exact callee-saved GP/FP sets and 16-byte aligned stack frame layout:
    //   [sp + 0 .. max_out_args]                  : outgoing stack args
    //   [sp + ret_buf_off .. + max_ret_buf]       : overflow return buffer
    //   [sp + spill_base .. + next_spill]         : SSA spills
    //   [sp + saved_ret_ptr_off]                  : optional saved caller ret_spill_ptr
    //   [sp + saved_gp_base ..]                   : saved FP/LR (if has_calls) + X19..X23
    //   [sp + saved_fp_base ..]                   : saved V8..V15 (16 bytes each)
    let mut used_gp_mask = 0u32;
    let mut used_fp_mask = 0u32;
    for (vi, &loc) in locs.iter().enumerate() {
        if !assigned[vi] {
            continue;
        }
        match loc {
            A64Loc::Gp(r) if (19..=23).contains(&r.0) => used_gp_mask |= 1u32 << r.0,
            A64Loc::Fp(v) if (8..=15).contains(&v.0) => used_fp_mask |= 1u32 << v.0,
            _ => {}
        }
    }
    let mut used_callee_gp = Vec::new();
    for r in [Reg::X19, Reg::X20, Reg::X21, Reg::X22, Reg::X23] {
        if (used_gp_mask & (1u32 << r.0)) != 0 {
            used_callee_gp.push(r);
        }
    }
    let mut used_callee_fp = Vec::new();
    for v in [
        VReg::V8,
        VReg::V9,
        VReg::V10,
        VReg::V11,
        VReg::V12,
        VReg::V13,
        VReg::V14,
        VReg::V15,
    ] {
        if (used_fp_mask & (1u32 << v.0)) != 0 {
            used_callee_fp.push(v);
        }
    }

    let ret_buf_off = max_out_args;
    let spill_base = align_i32(ret_buf_off + max_ret_buf, 16)?;
    if spill_base > 0 {
        for loc in &mut locs {
            if let A64Loc::Spill(off) = loc {
                *off = off
                    .checked_add(spill_base)
                    .ok_or("aarch64: spill base overflow")?;
            }
        }
    }
    let mut cur = spill_base + next_spill;
    let self_ret_layout = layout_rets(&ir.result_tys)?;
    let saved_ret_ptr_off = if self_ret_layout.stack_bytes > 0 {
        cur = align_i32(cur, 8)?;
        let off = cur;
        cur += 8;
        Some(off)
    } else {
        None
    };

    cur = align_i32(cur, 16)?;
    let saved_gp_base = cur;
    let gp_save_count = used_callee_gp.len() + if has_calls { 2 } else { 0 };
    cur += (gp_save_count as i32) * 8;
    cur = align_i32(cur, 16)?;
    let saved_fp_base = cur;
    cur += (used_callee_fp.len() as i32) * 16;
    let frame_size = align_i32(cur, 16)?;
    if frame_size > MAX_FRAME_BYTES {
        return Err(format!(
            "aarch64: stack frame ({frame_size} B) exceeds {MAX_FRAME_BYTES} B"
        ));
    }

    Ok(A64AllocResult {
        locs,
        block_order,
        used_callee_gp,
        used_callee_fp,
        saved_gp_base,
        saved_fp_base,
        ret_buf_off,
        saved_ret_ptr_off,
        frame_size,
        has_calls,
    })
}
