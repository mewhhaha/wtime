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
use crate::x64::ir::{FloatBinOp, IntBinOp, IrFunc, Op, SimdLaneKind, Term, Ty, Val, ValRange};
use crate::x64::module::{
    CompileBudget, MAX_FRAME_BYTES, MAX_LIVENESS_BYTES, MAX_LIVENESS_WORK_UNITS,
};

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

/// Constant that can be rematerialized directly into a scratch GP register without occupying
/// a register or stack spill slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum A64RematConst {
    I32(i32),
    I64(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum A64Loc {
    Gp(Reg),
    Fp(VReg),
    Spill(i32),
    Remat(A64RematConst),
    RematAddr {
        base_val: Val,
        base_reg: Reg,
        disp: i32,
        is64: bool,
    },
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
    score: u64,
    remat: Option<A64RematConst>,
    remat_addr: Option<(u32, i32, bool)>,
    remat_hoisted: bool,
}

#[derive(Clone, Copy)]
struct ActiveEntry {
    end: u32,
    reg_idx: usize,
    val: u32,
    start: u32,
    ty: Ty,
    score: u64,
    remat: Option<A64RematConst>,
    remat_addr: Option<(u32, i32, bool)>,
    pinned_by_remat: bool,
}

const GP_CALLER_COUNT: usize = 16;

pub fn allocate(ir: &IrFunc, budget: Option<&CompileBudget>) -> Result<A64AllocResult, String> {
    let n_blocks = ir.blocks.len();
    let n_vals = ir.val_tys.len();

    // 1. RPO over reachable blocks + 2-pass hot-fallthrough trace layout (sinking cold trap/abort
    //    blocks to the tail and linearizing single-pred triangle `if` diamonds).
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
            Term::CallCatch {
                ok_bb, catch_bb, ..
            }
            | Term::CallIndirectCatch {
                ok_bb, catch_bb, ..
            } => {
                succs[bi].push(ok_bb.0);
                if catch_bb != ok_bb {
                    succs[bi].push(catch_bb.0);
                }
            }
            Term::Return { .. }
            | Term::ReturnCall { .. }
            | Term::ReturnCallIndirect { .. }
            | Term::ThrowActive
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
    let mut rpo_pos = vec![u32::MAX; n_blocks];
    for (ri, &b) in post.iter().enumerate() {
        rpo_pos[b as usize] = ri as u32;
    }

    // Cold trap/unreachable block identification + hotness metadata for trace layout.
    let mut is_trap_blk = vec![false; n_blocks];
    let mut has_call_blk = vec![false; n_blocks];
    let mut has_store_blk = vec![false; n_blocks];
    let mut has_unchecked_mem_blk = vec![false; n_blocks];
    for &b in &post {
        let bu = b as usize;
        let bb = &ir.blocks[bu];
        let mut trap = matches!(bb.term, Term::Unreachable | Term::ThrowActive);
        if matches!(
            bb.term,
            Term::CallCatch { .. } | Term::CallIndirectCatch { .. }
        ) {
            has_call_blk[bu] = true;
        }
        for inst in &bb.insts {
            match inst.op {
                Op::Trap(_) | Op::MemGrow(_) | Op::MemGrowIdx { .. } | Op::TableGrow { .. } => {
                    trap = true;
                }
                Op::Call { .. } | Op::CallIndirect { .. } => {
                    has_call_blk[bu] = true;
                }
                Op::Store { checked, .. } => {
                    has_store_blk[bu] = true;
                    if !checked {
                        has_unchecked_mem_blk[bu] = true;
                    }
                }
                Op::StoreScaled { .. } => {
                    has_store_blk[bu] = true;
                }
                Op::Load { checked: false, .. } => {
                    has_unchecked_mem_blk[bu] = true;
                }
                _ => {}
            }
        }
        is_trap_blk[bu] = trap;
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

    let cand_score = |cand: u32, placed: &[bool]| -> i32 {
        let cu = cand as usize;
        let mut s = 0i32;
        if is_trap_blk[cu] {
            s += 1000;
        }
        let cbb = &ir.blocks[cu];
        let backedges_to_placed_hdr = succs[cu]
            .iter()
            .any(|&x| placed[x as usize] && ir.blocks[x as usize].is_loop_header);
        if cbb.is_loop_header || backedges_to_placed_hdr {
            s -= 100;
        }
        if has_call_blk[cu] {
            s += 40;
        }
        if has_store_blk[cu] {
            s += 20;
        }
        if !cbb.is_loop_header && cbb.insts.len() > 4 {
            s += 10;
        }
        let mut unchecked = has_unchecked_mem_blk[cu];
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

    let mut block_order = Vec::with_capacity(post.len());
    let mut placed = vec![false; n_blocks];
    for pass in 0..2 {
        for &seed in &post {
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
                                let mut ts = cand_score(then_bb.0, &placed);
                                let mut es = cand_score(else_bb.0, &placed);
                                let resolve_merge = |b: u32| -> u32 {
                                    let bb = &ir.blocks[b as usize];
                                    if bb.params.is_empty()
                                        && bb.insts.is_empty()
                                        && let Term::Jump { target, .. } = bb.term
                                    {
                                        target.0
                                    } else {
                                        b
                                    }
                                };
                                let is_tri = |arm: u32, merge: u32| -> bool {
                                    let eff_merge = resolve_merge(merge);
                                    !is_trap_blk[arm as usize]
                                        && !has_call_blk[arm as usize]
                                        && !ir.blocks[arm as usize].is_loop_header
                                        && ir.blocks[arm as usize].preds == 1
                                        && matches!(
                                            ir.blocks[arm as usize].term,
                                            Term::Jump { target, .. }
                                                if target.0 == merge || target.0 == eff_merge
                                        )
                                };
                                if is_tri(then_bb.0, else_bb.0) {
                                    ts -= 150;
                                } else if is_tri(else_bb.0, then_bb.0) {
                                    es -= 150;
                                }
                                let best = if (ts, rpo_pos[then_bb.0 as usize])
                                    <= (es, rpo_pos[else_bb.0 as usize])
                                {
                                    then_bb.0
                                } else {
                                    else_bb.0
                                };
                                (pass == 1 || !is_trap_blk[best as usize]).then_some(best)
                            }
                            (true, false) => {
                                (pass == 1 || !is_trap_blk[then_bb.0 as usize]).then_some(then_bb.0)
                            }
                            (false, true) => {
                                (pass == 1 || !is_trap_blk[else_bb.0 as usize]).then_some(else_bb.0)
                            }
                            (false, false) => None,
                        }
                    }
                    Term::CallCatch { ok_bb, .. } | Term::CallIndirectCatch { ok_bb, .. } => {
                        (!placed[ok_bb.0 as usize] && (pass == 1 || !is_trap_blk[ok_bb.0 as usize]))
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
    let n_ord = block_order.len();
    let mut ord_of = vec![u32::MAX; n_blocks];
    for (oi, &b) in block_order.iter().enumerate() {
        ord_of[b as usize] = oi as u32;
    }

    // 2. Linear program points, call/helper clobbers, remat constants, and outgoing stack-arg/return-buffer sizes.
    let mut block_start = vec![0u32; n_ord];
    let mut block_end = vec![0u32; n_ord];
    let mut def_pos = vec![u32::MAX; n_vals];
    let mut def_block = vec![u32::MAX; n_vals];
    let mut cross_block = vec![false; n_vals];
    let mut last_use = vec![0u32; n_vals];
    let mut val_remat: Vec<Option<A64RematConst>> = vec![None; n_vals];
    let mut val_remat_addr: Vec<Option<(u32, i32, bool)>> = vec![None; n_vals];
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
            def_block[p.0 as usize] = oi as u32;
            last_use[p.0 as usize] = last_use[p.0 as usize].max(pos + 1);
        }
        pos += 2;
        for inst in &bb.insts {
            inst.op.for_each_use(&ir.val_pool, |v| {
                let vi = v.0 as usize;
                last_use[vi] = last_use[vi].max(pos);
                if def_block[vi] != oi as u32 {
                    cross_block[vi] = true;
                }
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
                        def_block[rv.0 as usize] = oi as u32;
                        last_use[rv.0 as usize] = last_use[rv.0 as usize].max(pos + 2);
                    }
                }
                Op::MemGrow(_)
                | Op::MemGrowIdx { .. }
                | Op::MemCopy { .. }
                | Op::MemCopyIdx { .. }
                | Op::MemFill { .. }
                | Op::MemFillIdx { .. }
                | Op::MemInit { .. }
                | Op::TableInit { .. }
                | Op::TableCopy { .. }
                | Op::TableGrow { .. }
                | Op::TableFill { .. } => {
                    has_calls = true;
                    clobbers.push((pos, MASK_GP_CALLER, 0));
                }
                Op::VecElemLoop { .. } | Op::VecReduceLoop { .. } => {
                    clobbers.push((pos, 0x000f, 0x0007));
                }
                Op::PgoRecordCi { .. } => {
                    clobbers.push((pos, 1 << 0, 0));
                }
                _ => {}
            }
            if let Some(r) = inst.res {
                def_pos[r.0 as usize] = pos + 1;
                def_block[r.0 as usize] = oi as u32;
                last_use[r.0 as usize] = last_use[r.0 as usize].max(pos + 2);
                match inst.op {
                    Op::ConstI32(c) => val_remat[r.0 as usize] = Some(A64RematConst::I32(c)),
                    Op::ConstI64(c) => val_remat[r.0 as usize] = Some(A64RematConst::I64(c)),
                    Op::I32BinImm(IntBinOp::Add, base, disp)
                        if disp != 0 && disp.unsigned_abs() < 4096 =>
                    {
                        val_remat_addr[r.0 as usize] = Some((base.0, disp, false));
                    }
                    Op::I64BinImm(IntBinOp::Add, base, disp)
                        if disp != 0 && disp.unsigned_abs() < 4096 =>
                    {
                        val_remat_addr[r.0 as usize] = Some((base.0, disp, true));
                    }
                    _ => {}
                }
            }
            pos += 2;
        }
        match bb.term {
            Term::CallCatch {
                args,
                ok_bb,
                catch_args,
                ..
            } => {
                for &v in ir.pool_slice(args) {
                    let vi = v.0 as usize;
                    last_use[vi] = last_use[vi].max(pos);
                    if def_block[vi] != oi as u32 {
                        cross_block[vi] = true;
                    }
                }
                for &v in ir.pool_slice(catch_args) {
                    let vi = v.0 as usize;
                    last_use[vi] = last_use[vi].max(pos + 1);
                    if def_block[vi] != oi as u32 {
                        cross_block[vi] = true;
                    }
                }
                has_calls = true;
                clobbers.push((pos, MASK_GP_CALLER, MASK_FP_CALLER));
                let arg_tys: Vec<Ty> = ir
                    .pool_slice(args)
                    .iter()
                    .map(|&v| ir.val_tys[v.0 as usize])
                    .collect();
                let ret_tys: Vec<Ty> = ir.blocks[ok_bb.0 as usize]
                    .params
                    .iter()
                    .map(|&v| ir.val_tys[v.0 as usize])
                    .collect();
                max_out_args = max_out_args.max(layout_args(&arg_tys)?.stack_bytes);
                max_ret_buf = max_ret_buf.max(layout_rets(&ret_tys)?.stack_bytes);
            }
            Term::CallIndirectCatch {
                idx,
                args,
                ok_bb,
                catch_args,
                ..
            } => {
                let ii = idx.0 as usize;
                last_use[ii] = last_use[ii].max(pos);
                if def_block[ii] != oi as u32 {
                    cross_block[ii] = true;
                }
                for &v in ir.pool_slice(args) {
                    let vi = v.0 as usize;
                    last_use[vi] = last_use[vi].max(pos);
                    if def_block[vi] != oi as u32 {
                        cross_block[vi] = true;
                    }
                }
                for &v in ir.pool_slice(catch_args) {
                    let vi = v.0 as usize;
                    last_use[vi] = last_use[vi].max(pos + 1);
                    if def_block[vi] != oi as u32 {
                        cross_block[vi] = true;
                    }
                }
                has_calls = true;
                clobbers.push((pos, MASK_GP_CALLER, MASK_FP_CALLER));
                let arg_tys: Vec<Ty> = ir
                    .pool_slice(args)
                    .iter()
                    .map(|&v| ir.val_tys[v.0 as usize])
                    .collect();
                let ret_tys: Vec<Ty> = ir.blocks[ok_bb.0 as usize]
                    .params
                    .iter()
                    .map(|&v| ir.val_tys[v.0 as usize])
                    .collect();
                max_out_args = max_out_args.max(layout_args(&arg_tys)?.stack_bytes);
                max_ret_buf = max_ret_buf.max(layout_rets(&ret_tys)?.stack_bytes);
            }
            Term::ReturnCall { args, .. } | Term::ReturnCallIndirect { args, .. } => {
                bb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
                    let vi = v.0 as usize;
                    last_use[vi] = last_use[vi].max(pos);
                    if def_block[vi] != oi as u32 {
                        cross_block[vi] = true;
                    }
                });
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
            _ => {
                bb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
                    let vi = v.0 as usize;
                    last_use[vi] = last_use[vi].max(pos);
                    if def_block[vi] != oi as u32 {
                        cross_block[vi] = true;
                    }
                });
            }
        }
        pos += 2;
        block_end[oi] = pos;
    }

    // 3. Exact backward dataflow liveness over cross-block values, bounded by
    //    `MAX_LIVENESS_BYTES`, `MAX_LIVENESS_WORK_UNITS`, and `CompileBudget`.
    let mut cb_id = vec![u32::MAX; n_vals];
    let mut cb_vals: Vec<u32> = Vec::new();
    for vi in 0..n_vals {
        if cross_block[vi] && def_pos[vi] != u32::MAX {
            cb_id[vi] = cb_vals.len() as u32;
            cb_vals.push(vi as u32);
        }
    }
    let mut iv_start = def_pos.clone();
    let mut iv_end = last_use;
    let n_cb = cb_vals.len();
    if n_cb > 0 {
        let words = n_cb.div_ceil(64);
        let matrix_words = n_ord
            .checked_mul(words)
            .ok_or("aarch64: liveness table size overflow")?;
        let matrix_bytes = matrix_words
            .checked_mul(4 * std::mem::size_of::<u64>())
            .ok_or("aarch64: liveness table size overflow")?;
        if matrix_bytes > MAX_LIVENESS_BYTES {
            return Err(format!(
                "aarch64: function {} liveness matrix ({matrix_bytes} B) exceeds cap {MAX_LIVENESS_BYTES} B",
                ir.func_index
            ));
        }
        // Declare `_liveness_guard` BEFORE the four matrices so LIFO drop order guarantees all
        // liveness buffers are freed before releasing the budget reservation.
        let _liveness_guard = match budget {
            Some(b) => Some(b.reserve_liveness(matrix_bytes)?),
            None => None,
        };
        let alloc_zero_u64 = |len: usize| -> Result<Vec<u64>, String> {
            let mut v = Vec::new();
            v.try_reserve_exact(len)
                .map_err(|_| "aarch64: liveness table allocation failed".to_string())?;
            v.resize(len, 0u64);
            Ok(v)
        };
        let mut live_use = alloc_zero_u64(matrix_words)?;
        let mut live_def = alloc_zero_u64(matrix_words)?;
        let mut live_in = alloc_zero_u64(matrix_words)?;
        let mut live_out = alloc_zero_u64(matrix_words)?;
        for (oi, &bid) in block_order.iter().enumerate() {
            let bb = &ir.blocks[bid as usize];
            let base = oi * words;
            for &p in &bb.params {
                let ci = cb_id[p.0 as usize];
                if ci != u32::MAX {
                    let ci = ci as usize;
                    live_def[base + (ci >> 6)] |= 1u64 << (ci & 63);
                }
            }
            for inst in &bb.insts {
                inst.op.for_each_use(&ir.val_pool, |v| {
                    let ci = cb_id[v.0 as usize];
                    if ci != u32::MAX {
                        let ci = ci as usize;
                        if (live_def[base + (ci >> 6)] & (1u64 << (ci & 63))) == 0 {
                            live_use[base + (ci >> 6)] |= 1u64 << (ci & 63);
                        }
                    }
                });
                if let Some(r) = inst.res {
                    let ci = cb_id[r.0 as usize];
                    if ci != u32::MAX {
                        let ci = ci as usize;
                        live_def[base + (ci >> 6)] |= 1u64 << (ci & 63);
                    }
                }
                match inst.op {
                    Op::Call { rets, .. } | Op::CallIndirect { rets, .. } => {
                        for &r in ir.pool_slice(rets) {
                            let ci = cb_id[r.0 as usize];
                            if ci != u32::MAX {
                                let ci = ci as usize;
                                live_def[base + (ci >> 6)] |= 1u64 << (ci & 63);
                            }
                        }
                    }
                    _ => {}
                }
            }
            bb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
                let ci = cb_id[v.0 as usize];
                if ci != u32::MAX {
                    let ci = ci as usize;
                    if (live_def[base + (ci >> 6)] & (1u64 << (ci & 63))) == 0 {
                        live_use[base + (ci >> 6)] |= 1u64 << (ci & 63);
                    }
                }
            });
        }
        let mut work_units: usize = 0;
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
                    work_units = work_units.saturating_add(words);
                    if work_units > MAX_LIVENESS_WORK_UNITS {
                        return Err(format!(
                            "aarch64: function {} liveness work ({work_units}) exceeds cap {MAX_LIVENESS_WORK_UNITS}",
                            ir.func_index
                        ));
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
                work_units = work_units.saturating_add(words);
                if work_units > MAX_LIVENESS_WORK_UNITS {
                    return Err(format!(
                        "aarch64: function {} liveness work ({work_units}) exceeds cap {MAX_LIVENESS_WORK_UNITS}",
                        ir.func_index
                    ));
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
        for oi in 0..n_ord {
            let base = oi * words;
            let bs = block_start[oi];
            let be = block_end[oi];
            for w in 0..words {
                let mut bits = live_in[base + w] | live_out[base + w];
                while bits != 0 {
                    let tz = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let ci = (w << 6) + tz;
                    if ci < n_cb {
                        let vi = cb_vals[ci] as usize;
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
    }

    // 3b. Loop depth, spill-weight estimation, ABI register hints, and coalescing peer hints.
    let mut loop_depth = vec![0u8; n_ord];
    if n_ord > 1 {
        let mut preds_rpo: Vec<Vec<u32>> = vec![Vec::new(); post.len()];
        let mut backedges: Vec<(u32, u32)> = Vec::new();
        for (ri, &bid) in post.iter().enumerate() {
            for &sid in &succs[bid as usize] {
                let sri = rpo_pos[sid as usize];
                if sri != u32::MAX {
                    preds_rpo[sri as usize].push(ri as u32);
                    if sri <= ri as u32 {
                        backedges.push((ri as u32, sri));
                    }
                }
            }
        }
        if !backedges.is_empty() {
            let mut rpo_depth = vec![0u8; post.len()];
            let mut seen = vec![0u32; post.len()];
            let mut wl = Vec::new();
            let mut stamp = 0u32;
            for (latch, header) in backedges {
                stamp = stamp.wrapping_add(1);
                seen[header as usize] = stamp;
                rpo_depth[header as usize] = rpo_depth[header as usize].saturating_add(1).min(4);
                if latch != header {
                    seen[latch as usize] = stamp;
                    rpo_depth[latch as usize] = rpo_depth[latch as usize].saturating_add(1).min(4);
                    wl.push(latch);
                    while let Some(u) = wl.pop() {
                        for &p in &preds_rpo[u as usize] {
                            if p > header && seen[p as usize] != stamp {
                                seen[p as usize] = stamp;
                                rpo_depth[p as usize] =
                                    rpo_depth[p as usize].saturating_add(1).min(4);
                                wl.push(p);
                            }
                        }
                    }
                }
            }
            for (ri, &bid) in post.iter().enumerate() {
                let oi = ord_of[bid as usize] as usize;
                if oi < n_ord {
                    loop_depth[oi] = rpo_depth[ri];
                }
            }
        }
    }

    let mut val_weight = vec![0u32; n_vals];
    let mut max_use_loop_depth = vec![0u8; n_vals];
    let mut hint_gp: Vec<Option<Reg>> = vec![None; n_vals];
    let mut hint_fp: Vec<Option<VReg>> = vec![None; n_vals];
    let mut hint_peers: Vec<[u32; 4]> = vec![[u32::MAX; 4]; n_vals];
    let mut first_edge_arg = vec![u32::MAX; n_vals];
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

    if !ir.params.is_empty() {
        let p_layout = layout_args(&self_param_tys)?;
        for (&pv, slot) in ir.params.iter().zip(p_layout.slots) {
            match slot {
                A64AbiSlot::Gp(g) => hint_gp[pv.0 as usize] = Some(g),
                A64AbiSlot::Fp(f) => hint_fp[pv.0 as usize] = Some(f),
                A64AbiSlot::Stack(_) => {}
            }
        }
    }

    let hint_arg_slice = |slice: &[crate::x64::ir::Val],
                          hint_gp: &mut [Option<Reg>],
                          hint_fp: &mut [Option<VReg>]| {
        let (mut ng, mut nf) = (0usize, 0usize);
        for &v in slice {
            let ty = ir.val_tys[v.0 as usize];
            if ty.is_fp() {
                if nf < CALL_FP_ARGS.len() {
                    if hint_fp[v.0 as usize].is_none() {
                        hint_fp[v.0 as usize] = Some(CALL_FP_ARGS[nf]);
                    }
                    nf += 1;
                }
            } else if ng < CALL_GP_ARGS.len() {
                if hint_gp[v.0 as usize].is_none() {
                    hint_gp[v.0 as usize] = Some(CALL_GP_ARGS[ng]);
                }
                ng += 1;
            }
        }
    };
    let hint_ret_def_slice = |slice: &[crate::x64::ir::Val],
                              hint_gp: &mut [Option<Reg>],
                              hint_fp: &mut [Option<VReg>]| {
        let (mut ng, mut nf) = (0usize, 0usize);
        for &v in slice {
            let ty = ir.val_tys[v.0 as usize];
            if ty.is_fp() {
                if nf < CALL_FP_ARGS.len() {
                    hint_fp[v.0 as usize] = Some(CALL_FP_ARGS[nf]);
                    nf += 1;
                }
            } else if ng < CALL_GP_ARGS.len() {
                hint_gp[v.0 as usize] = Some(CALL_GP_ARGS[ng]);
                ng += 1;
            }
        }
    };

    for (oi, &bid) in block_order.iter().enumerate() {
        let bb = &ir.blocks[bid as usize];
        let is_cold_trap = n_blocks > 1 && is_trap_blk[bid as usize];
        let cur_ld = loop_depth[oi];
        let w = if is_cold_trap {
            1u32
        } else {
            match cur_ld {
                0 => 4u32,
                1 => 64,
                2 => 1024,
                3 => 16_384,
                _ => 131_072,
            }
        };
        for &p in &bb.params {
            val_weight[p.0 as usize] = val_weight[p.0 as usize].saturating_add(w);
        }
        for inst in &bb.insts {
            inst.op.for_each_use(&ir.val_pool, |v| {
                let vu = v.0 as usize;
                val_weight[vu] = val_weight[vu].saturating_add(w);
                if cur_ld > max_use_loop_depth[vu] {
                    max_use_loop_depth[vu] = cur_ld;
                }
            });
            if let Some(r) = inst.res {
                val_weight[r.0 as usize] = val_weight[r.0 as usize].saturating_add(w);
                match inst.op {
                    Op::Copy(a) => {
                        add_peer(&mut hint_peers, r.0, a.0);
                        add_peer(&mut hint_peers, a.0, r.0);
                    }
                    Op::I32Bin(bop, a, b) | Op::I64Bin(bop, a, b) => {
                        add_peer(&mut hint_peers, r.0, a.0);
                        if matches!(
                            bop,
                            IntBinOp::Add
                                | IntBinOp::Mul
                                | IntBinOp::And
                                | IntBinOp::Or
                                | IntBinOp::Xor
                        ) {
                            add_peer(&mut hint_peers, r.0, b.0);
                        }
                    }
                    Op::I32BinImm(_, a, _)
                    | Op::I64BinImm(_, a, _)
                    | Op::I32WrapI64(a)
                    | Op::I64ExtendI32U(a) => {
                        add_peer(&mut hint_peers, r.0, a.0);
                    }
                    Op::V128ExtractLane {
                        kind: SimdLaneKind::F64x2 | SimdLaneKind::F32x4,
                        v,
                        lane: 0,
                    } => {
                        add_peer(&mut hint_peers, r.0, v.0);
                    }
                    Op::F32Bin(bop, a, b) | Op::F64Bin(bop, a, b) => {
                        add_peer(&mut hint_peers, r.0, a.0);
                        if matches!(bop, FloatBinOp::Add | FloatBinOp::Mul) {
                            add_peer(&mut hint_peers, r.0, b.0);
                        }
                    }
                    Op::Select { a, b, .. } => {
                        add_peer(&mut hint_peers, r.0, a.0);
                        add_peer(&mut hint_peers, r.0, b.0);
                    }
                    Op::CmpSelect { tv, fv, .. } => {
                        add_peer(&mut hint_peers, r.0, fv.0);
                        add_peer(&mut hint_peers, r.0, tv.0);
                    }
                    _ => {}
                }
            }
            match inst.op {
                Op::Call { args, rets, .. } | Op::CallIndirect { args, rets, .. } => {
                    hint_arg_slice(ir.pool_slice(args), &mut hint_gp, &mut hint_fp);
                    let ret_slice = ir.pool_slice(rets);
                    for &rv in ret_slice {
                        val_weight[rv.0 as usize] = val_weight[rv.0 as usize].saturating_add(w);
                    }
                    hint_ret_def_slice(ret_slice, &mut hint_gp, &mut hint_fp);
                }
                _ => {}
            }
        }
        bb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
            let vu = v.0 as usize;
            val_weight[vu] = val_weight[vu].saturating_add(w);
            if cur_ld > max_use_loop_depth[vu] {
                max_use_loop_depth[vu] = cur_ld;
            }
        });
        let mut wire_edge = |target_bb: u32, args: ValRange| {
            let params = &ir.blocks[target_bb as usize].params;
            let avals = ir.pool_slice(args);
            for (&p, &a) in params.iter().zip(avals) {
                add_peer(&mut hint_peers, p.0, a.0);
                add_peer(&mut hint_peers, a.0, p.0);
                if a != p {
                    let prev = first_edge_arg[p.0 as usize];
                    if prev == u32::MAX {
                        first_edge_arg[p.0 as usize] = a.0;
                    } else if prev != a.0 {
                        add_peer(&mut hint_peers, a.0, prev);
                        add_peer(&mut hint_peers, prev, a.0);
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
                for &(t, a) in &ir.br_tables[table_idx as usize].unique_edges {
                    wire_edge(t.0, a);
                }
            }
            Term::Return { vals }
            | Term::ReturnCall { args: vals, .. }
            | Term::ReturnCallIndirect { args: vals, .. } => {
                hint_arg_slice(ir.pool_slice(vals), &mut hint_gp, &mut hint_fp);
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
                hint_arg_slice(ir.pool_slice(args), &mut hint_gp, &mut hint_fp);
                hint_ret_def_slice(
                    &ir.blocks[ok_bb.0 as usize].params,
                    &mut hint_gp,
                    &mut hint_fp,
                );
            }
            Term::ThrowActive | Term::Unreachable => {}
        }
    }

    // 4. Build GP and FP intervals with exact clobber masks, spill weights, and remat metadata.
    let mut val_clob_gp = vec![0u32; n_vals];
    let mut val_clob_fp = vec![0u32; n_vals];
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
        val_clob_gp[vi] = cgp;
        val_clob_fp[vi] = cfp;
        let ty = ir.val_tys[vi];
        let len = u64::from((e - s).max(1));
        let score = (u64::from(val_weight[vi].max(1)) << 16) / len;
        let remat = val_remat[vi];
        let def_ld = if def_block[vi] != u32::MAX {
            loop_depth[def_block[vi] as usize]
        } else {
            0
        };
        let remat_hoisted = remat.is_some() && max_use_loop_depth[vi] > def_ld;
        let iv = Interval {
            val: vi as u32,
            start: s,
            end: e,
            ty,
            clob_gp: cgp,
            clob_fp: cfp,
            score,
            remat,
            remat_addr: val_remat_addr[vi],
            remat_hoisted,
        };
        if ty.is_fp() {
            fp_ivs.push(iv);
        } else {
            gp_ivs.push(iv);
        }
    }
    gp_ivs.sort_unstable_by_key(|iv| (iv.start, iv.end, iv.val));
    fp_ivs.sort_unstable_by_key(|iv| (iv.start, iv.end, iv.val));

    // 5. Linear scan for GP and FP/V128 with vote-counted coalescing, ABI hints, remat, and weighted eviction.
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

    // 5a. GP scan
    let mut active_gp: Vec<ActiveEntry> = Vec::new();
    for iv in &gp_ivs {
        active_gp.retain(|a| a.end > iv.start);
        let mut used = 0u32;
        for a in &active_gp {
            used |= 1u32 << GP_ALLOC[a.reg_idx].0;
        }
        let forbid = used | iv.clob_gp;
        let max_ri = if iv.remat.is_some() && !iv.remat_hoisted {
            GP_CALLER_COUNT
        } else {
            GP_ALLOC.len()
        };

        // 1. Vote-counted coalescing peer already assigned a non-interfering reg.
        let mut chosen: Option<(usize, Reg)> = None;
        {
            let mut cands: [(usize, Reg, u8); 4] = [(0, Reg::X0, 0); 4];
            let mut n_cands = 0usize;
            for &peer in &hint_peers[iv.val as usize] {
                if peer != u32::MAX
                    && assigned[peer as usize]
                    && let A64Loc::Gp(g) = locs[peer as usize]
                    && (forbid & (1u32 << g.0)) == 0
                    && let Some(ri) = GP_ALLOC[..max_ri].iter().position(|&x| x == g)
                {
                    if let Some(slot) = cands[..n_cands].iter_mut().find(|c| c.0 == ri) {
                        slot.2 += 1;
                    } else if n_cands < 4 {
                        cands[n_cands] = (ri, g, 1);
                        n_cands += 1;
                    }
                }
            }
            let mut best_votes = 0u8;
            for &(ri, g, votes) in &cands[..n_cands] {
                if votes > best_votes {
                    best_votes = votes;
                    chosen = Some((ri, g));
                }
            }
        }
        // 2. Direct ABI hint register.
        if chosen.is_none()
            && let Some(g) = hint_gp[iv.val as usize]
            && (forbid & (1u32 << g.0)) == 0
            && let Some(ri) = GP_ALLOC[..max_ri].iter().position(|&x| x == g)
        {
            chosen = Some((ri, g));
        }
        // 3. Forward peer ABI hint & peer clobber avoidance.
        if chosen.is_none() {
            let mut peer_avoid = 0u32;
            let mut peer_hint: Option<Reg> = None;
            for &peer in &hint_peers[iv.val as usize] {
                if peer != u32::MAX && !assigned[peer as usize] {
                    peer_avoid |= val_clob_gp[peer as usize];
                    if peer_hint.is_none() {
                        peer_hint = hint_gp[peer as usize];
                    }
                }
            }
            if let Some(g) = peer_hint
                && ((forbid | peer_avoid) & (1u32 << g.0)) == 0
                && let Some(ri) = GP_ALLOC[..max_ri].iter().position(|&x| x == g)
            {
                chosen = Some((ri, g));
            }
            if chosen.is_none() && peer_avoid != 0 {
                let avoid_mask = forbid | peer_avoid;
                if let Some((ri, &r)) = GP_ALLOC[..max_ri]
                    .iter()
                    .enumerate()
                    .find(|&(_, &r)| (avoid_mask & (1u32 << r.0)) == 0)
                {
                    chosen = Some((ri, r));
                }
            }
            if chosen.is_none()
                && let Some((ri, &r)) = GP_ALLOC[..max_ri]
                    .iter()
                    .enumerate()
                    .find(|&(_, &r)| (forbid & (1u32 << r.0)) == 0)
            {
                chosen = Some((ri, r));
            }
        }

        if let Some((ri, r)) = chosen {
            locs[iv.val as usize] = A64Loc::Gp(r);
            assigned[iv.val as usize] = true;
            active_gp.push(ActiveEntry {
                end: iv.end,
                reg_idx: ri,
                val: iv.val,
                start: iv.start,
                ty: iv.ty,
                score: iv.score,
                remat: iv.remat,
                remat_addr: iv.remat_addr,
                pinned_by_remat: false,
            });
        } else if let Some(rc) = iv.remat {
            locs[iv.val as usize] = A64Loc::Remat(rc);
            assigned[iv.val as usize] = true;
        } else {
            let try_remat_addr = |cand_start: u32,
                                  cand_end: u32,
                                  cand_clob_gp: u32,
                                  ra: Option<(u32, i32, bool)>,
                                  locs: &[A64Loc],
                                  assigned: &[bool],
                                  active: &mut [ActiveEntry]|
             -> Option<A64Loc> {
                let (base_u, disp, is64) = ra?;
                let bi = base_u as usize;
                if !assigned[bi] {
                    return None;
                }
                let A64Loc::Gp(bg) = locs[bi] else {
                    return None;
                };
                if (cand_clob_gp & (1u32 << bg.0)) != 0 {
                    return None;
                }
                if iv_start[bi] > cand_start || cand_end > iv_end[bi].max(iv_start[bi] + 1) {
                    return None;
                }
                let entry = active.iter_mut().find(|a| a.val == base_u)?;
                entry.pinned_by_remat = true;
                Some(A64Loc::RematAddr {
                    base_val: Val(base_u),
                    base_reg: bg,
                    disp,
                    is64,
                })
            };
            // Evict a rematerializable constant or strictly lower-score active entry if beneficial.
            let mut best_ai: Option<(usize, bool, u64, u32)> = None;
            for (ai, a) in active_gp.iter().enumerate() {
                if a.pinned_by_remat {
                    continue;
                }
                let r = GP_ALLOC[a.reg_idx];
                if (iv.clob_gp & (1u32 << r.0)) != 0 {
                    continue;
                }
                let is_remat = a.remat.is_some();
                if !is_remat && (a.score > iv.score || (a.score == iv.score && a.end <= iv.end)) {
                    continue;
                }
                // Prefer evicting remat first, then lowest score, then furthest end.
                let key = (!is_remat, a.score, Reverse(a.end));
                let better = match best_ai {
                    None => true,
                    Some((_, br, bsc, bend)) => key < (!br, bsc, Reverse(bend)),
                };
                if better {
                    best_ai = Some((ai, is_remat, a.score, a.end));
                }
            }
            let avoid_real_spill = best_ai.is_none_or(|(_, is_remat, _, _)| !is_remat);
            if avoid_real_spill
                && let Some(ra_loc) = try_remat_addr(
                    iv.start,
                    iv.end,
                    iv.clob_gp,
                    iv.remat_addr,
                    &locs,
                    &assigned,
                    &mut active_gp,
                )
            {
                locs[iv.val as usize] = ra_loc;
                assigned[iv.val as usize] = true;
            } else if let Some((ai, _, _, _)) = best_ai {
                let ev = active_gp.swap_remove(ai);
                let r = GP_ALLOC[ev.reg_idx];
                if let Some(rc) = ev.remat {
                    locs[ev.val as usize] = A64Loc::Remat(rc);
                } else if let Some(ra_loc) = try_remat_addr(
                    ev.start,
                    ev.end,
                    val_clob_gp[ev.val as usize],
                    ev.remat_addr,
                    &locs,
                    &assigned,
                    &mut active_gp,
                ) {
                    locs[ev.val as usize] = ra_loc;
                } else {
                    let sp = alloc_spill(
                        ev.ty,
                        ev.start,
                        ev.end,
                        &mut free8,
                        &mut free16,
                        &mut next_spill,
                    )?;
                    locs[ev.val as usize] = A64Loc::Spill(sp);
                }
                locs[iv.val as usize] = A64Loc::Gp(r);
                assigned[iv.val as usize] = true;
                active_gp.push(ActiveEntry {
                    end: iv.end,
                    reg_idx: ev.reg_idx,
                    val: iv.val,
                    start: iv.start,
                    ty: iv.ty,
                    score: iv.score,
                    remat: None,
                    remat_addr: iv.remat_addr,
                    pinned_by_remat: false,
                });
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
    }

    // 5b. FP / V128 scan
    let mut active_fp: Vec<ActiveEntry> = Vec::new();
    for iv in &fp_ivs {
        active_fp.retain(|a| a.end > iv.start);
        let mut used = 0u32;
        for a in &active_fp {
            used |= 1u32 << FP_ALLOC[a.reg_idx].0;
        }
        let forbid = used | iv.clob_fp;

        let mut chosen: Option<(usize, VReg)> = None;
        {
            let mut cands: [(usize, VReg, u8); 4] = [(0, VReg::V0, 0); 4];
            let mut n_cands = 0usize;
            for &peer in &hint_peers[iv.val as usize] {
                if peer != u32::MAX
                    && assigned[peer as usize]
                    && let A64Loc::Fp(v) = locs[peer as usize]
                    && (forbid & (1u32 << v.0)) == 0
                    && let Some(ri) = FP_ALLOC.iter().position(|&x| x == v)
                {
                    if let Some(slot) = cands[..n_cands].iter_mut().find(|c| c.0 == ri) {
                        slot.2 += 1;
                    } else if n_cands < 4 {
                        cands[n_cands] = (ri, v, 1);
                        n_cands += 1;
                    }
                }
            }
            let mut best_votes = 0u8;
            for &(ri, v, votes) in &cands[..n_cands] {
                if votes > best_votes {
                    best_votes = votes;
                    chosen = Some((ri, v));
                }
            }
        }
        if chosen.is_none()
            && let Some(v) = hint_fp[iv.val as usize]
            && (forbid & (1u32 << v.0)) == 0
            && let Some(ri) = FP_ALLOC.iter().position(|&x| x == v)
        {
            chosen = Some((ri, v));
        }
        if chosen.is_none() {
            let mut peer_avoid = 0u32;
            let mut peer_hint: Option<VReg> = None;
            for &peer in &hint_peers[iv.val as usize] {
                if peer != u32::MAX && !assigned[peer as usize] {
                    peer_avoid |= val_clob_fp[peer as usize];
                    if peer_hint.is_none() {
                        peer_hint = hint_fp[peer as usize];
                    }
                }
            }
            if let Some(v) = peer_hint
                && ((forbid | peer_avoid) & (1u32 << v.0)) == 0
                && let Some(ri) = FP_ALLOC.iter().position(|&x| x == v)
            {
                chosen = Some((ri, v));
            }
            if chosen.is_none() && peer_avoid != 0 {
                let avoid_mask = forbid | peer_avoid;
                if let Some((ri, &v)) = FP_ALLOC
                    .iter()
                    .enumerate()
                    .find(|&(_, &v)| (avoid_mask & (1u32 << v.0)) == 0)
                {
                    chosen = Some((ri, v));
                }
            }
            if chosen.is_none()
                && let Some((ri, &v)) = FP_ALLOC
                    .iter()
                    .enumerate()
                    .find(|&(_, &v)| (forbid & (1u32 << v.0)) == 0)
            {
                chosen = Some((ri, v));
            }
        }

        if let Some((ri, v)) = chosen {
            locs[iv.val as usize] = A64Loc::Fp(v);
            assigned[iv.val as usize] = true;
            active_fp.push(ActiveEntry {
                end: iv.end,
                reg_idx: ri,
                val: iv.val,
                start: iv.start,
                ty: iv.ty,
                score: iv.score,
                remat: None,
                remat_addr: None,
                pinned_by_remat: false,
            });
        } else {
            let mut best_ai: Option<(usize, u64, u32)> = None;
            for (ai, a) in active_fp.iter().enumerate() {
                let v = FP_ALLOC[a.reg_idx];
                if (iv.clob_fp & (1u32 << v.0)) != 0 {
                    continue;
                }
                if a.score > iv.score || (a.score == iv.score && a.end <= iv.end) {
                    continue;
                }
                let better = match best_ai {
                    None => true,
                    Some((_, bsc, bend)) => (a.score, Reverse(a.end)) < (bsc, Reverse(bend)),
                };
                if better {
                    best_ai = Some((ai, a.score, a.end));
                }
            }
            if let Some((ai, _, _)) = best_ai {
                let ev = active_fp.swap_remove(ai);
                let v = FP_ALLOC[ev.reg_idx];
                let sp = alloc_spill(
                    ev.ty,
                    ev.start,
                    ev.end,
                    &mut free8,
                    &mut free16,
                    &mut next_spill,
                )?;
                locs[ev.val as usize] = A64Loc::Spill(sp);
                locs[iv.val as usize] = A64Loc::Fp(v);
                assigned[iv.val as usize] = true;
                active_fp.push(ActiveEntry {
                    end: iv.end,
                    reg_idx: ev.reg_idx,
                    val: iv.val,
                    start: iv.start,
                    ty: iv.ty,
                    score: iv.score,
                    remat: None,
                    remat_addr: None,
                    pinned_by_remat: false,
                });
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
