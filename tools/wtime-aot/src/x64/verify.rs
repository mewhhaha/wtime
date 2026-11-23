//! Independent SSA IR and register-allocation verifiers for `--backend x64`.
//!
//! - `verify_ir`: independently recomputes CFG predecessors from terminators, computes
//!   exact dominators over reachable blocks, and verifies SSA single-definition, dominance
//!   of every use, block-parameter arity/type matching, and opcode/terminator type soundness.
//! - `verify_alloc`: independently recomputes cross-block liveness across the linear block
//!   order and verifies register class & allocatable-set membership, callee-save tracking,
//!   physical register & spill-range disjointness across all simultaneously live values,
//!   clobber safety across calls/helpers/div/shifts, distinct edge destinations, and frame sizing.

use std::collections::{HashMap, HashSet};

use wtime_types::ValType;

use super::asm::Gp;
use super::ir::{CmpBranchKind, IntBinOp, IrFunc, Op, Term, Ty, Val, ValRange};
use super::module::{MAX_FRAME_BYTES, X64Module};
use super::regalloc::{
    AllocResult, GP_ALLOC, GP_CALLEE_SAVED, HelperKind, Loc, MASK_GP_CALLER, MASK_RAX_RDX, MASK_RCX,
    MASK_XMM_CALLER, XMM_ALLOC, XMM_CALLEE_SAVED, helper_clobbers, layout_args, layout_rets,
};

fn check_val_range(ir: &IrFunc, r: ValRange, ctx: &str) -> Result<(), String> {
    let start = r.start as usize;
    let len = r.len as usize;
    let end = start
        .checked_add(len)
        .ok_or_else(|| format!("verify_ir fn {}: {ctx} ValRange overflow", ir.func_index))?;
    if end > ir.val_pool.len() {
        return Err(format!(
            "verify_ir fn {}: {ctx} ValRange {}..{} exceeds val_pool len {}",
            ir.func_index,
            start,
            end,
            ir.val_pool.len()
        ));
    }
    Ok(())
}

fn val_ty(ir: &IrFunc, v: Val, ctx: &str) -> Result<Ty, String> {
    ir.val_tys
        .get(v.0 as usize)
        .copied()
        .ok_or_else(|| format!("verify_ir fn {}: {ctx} val v{} out of range ({})", ir.func_index, v.0, ir.val_tys.len()))
}

fn check_edge(ir: &IrFunc, from_bb: usize, target_bb: u32, args: ValRange, ctx: &str) -> Result<(), String> {
    check_val_range(ir, args, ctx)?;
    let target = ir.blocks.get(target_bb as usize).ok_or_else(|| {
        format!(
            "verify_ir fn {} bb{from_bb}: {ctx} target bb{target_bb} out of range ({})",
            ir.func_index,
            ir.blocks.len()
        )
    })?;
    let arg_vals = ir.pool_slice(args);
    if arg_vals.len() != target.params.len() {
        return Err(format!(
            "verify_ir fn {} bb{from_bb}: {ctx} edge to bb{target_bb} arity mismatch ({} args vs {} params)",
            ir.func_index,
            arg_vals.len(),
            target.params.len()
        ));
    }
    for (idx, (&a, &p)) in arg_vals.iter().zip(&target.params).enumerate() {
        let at = val_ty(ir, a, ctx)?;
        let pt = val_ty(ir, p, ctx)?;
        if at != pt {
            return Err(format!(
                "verify_ir fn {} bb{from_bb}: {ctx} edge to bb{target_bb} arg {idx} type mismatch ({at:?} vs {pt:?})",
                ir.func_index
            ));
        }
    }
    Ok(())
}

/// Verifies SSA structure, CFG edge arities/types, dominator validity, and opcode type soundness.
/// Independently recomputes predecessors from terminators instead of trusting `bb.preds`.
pub fn verify_ir(ir: &IrFunc, module: &X64Module) -> Result<(), String> {
    if ir.blocks.is_empty() {
        return Err(format!("verify_ir fn {}: function has zero basic blocks", ir.func_index));
    }
    if ir.blocks[0].params != ir.params {
        return Err(format!("verify_ir fn {}: bb0.params does not match ir.params", ir.func_index));
    }
    let n_blocks = ir.blocks.len();
    let n_vals = ir.val_tys.len();

    // 1. Independently build successor lists and check every terminator's edge arity/types.
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n_blocks];
    for (bi, bb) in ir.blocks.iter().enumerate() {
        match bb.term {
            Term::Jump { target, args } => {
                check_edge(ir, bi, target.0, args, "Jump")?;
                succs[bi].push(target.0 as usize);
            }
            Term::BranchIf { cond, then_bb, then_args, else_bb, else_args } => {
                if val_ty(ir, cond, "BranchIf.cond")? != Ty::I32 {
                    return Err(format!("verify_ir fn {} bb{bi}: BranchIf cond must be I32", ir.func_index));
                }
                check_edge(ir, bi, then_bb.0, then_args, "BranchIf.then")?;
                check_edge(ir, bi, else_bb.0, else_args, "BranchIf.else")?;
                succs[bi].push(then_bb.0 as usize);
                if else_bb != then_bb {
                    succs[bi].push(else_bb.0 as usize);
                }
            }
            Term::CmpBranch { kind, a, b, then_bb, then_args, else_bb, else_args } => {
                let at = val_ty(ir, a, "CmpBranch.a")?;
                match kind {
                    CmpBranchKind::I32(_) => {
                        let bt = val_ty(ir, b, "CmpBranch.b")?;
                        if at != Ty::I32 || bt != Ty::I32 {
                            return Err(format!("verify_ir fn {} bb{bi}: CmpBranch::I32 type mismatch", ir.func_index));
                        }
                    }
                    CmpBranchKind::I32Imm(_, _) => {
                        if at != Ty::I32 {
                            return Err(format!("verify_ir fn {} bb{bi}: CmpBranch::I32Imm type mismatch", ir.func_index));
                        }
                    }
                    CmpBranchKind::I64(_) => {
                        let bt = val_ty(ir, b, "CmpBranch.b")?;
                        if at != Ty::I64 || bt != Ty::I64 {
                            return Err(format!("verify_ir fn {} bb{bi}: CmpBranch::I64 type mismatch", ir.func_index));
                        }
                    }
                    CmpBranchKind::I64Imm(_, _) => {
                        if at != Ty::I64 {
                            return Err(format!("verify_ir fn {} bb{bi}: CmpBranch::I64Imm type mismatch", ir.func_index));
                        }
                    }
                    CmpBranchKind::F32(_) => {
                        let bt = val_ty(ir, b, "CmpBranch.b")?;
                        if at != Ty::F32 || bt != Ty::F32 {
                            return Err(format!("verify_ir fn {} bb{bi}: CmpBranch::F32 type mismatch", ir.func_index));
                        }
                    }
                    CmpBranchKind::F64(_) => {
                        let bt = val_ty(ir, b, "CmpBranch.b")?;
                        if at != Ty::F64 || bt != Ty::F64 {
                            return Err(format!("verify_ir fn {} bb{bi}: CmpBranch::F64 type mismatch", ir.func_index));
                        }
                    }
                }
                check_edge(ir, bi, then_bb.0, then_args, "CmpBranch.then")?;
                check_edge(ir, bi, else_bb.0, else_args, "CmpBranch.else")?;
                succs[bi].push(then_bb.0 as usize);
                if else_bb != then_bb {
                    succs[bi].push(else_bb.0 as usize);
                }
            }
            Term::BrTable { idx, table_idx } => {
                if val_ty(ir, idx, "BrTable.idx")? != Ty::I32 {
                    return Err(format!("verify_ir fn {} bb{bi}: BrTable idx must be I32", ir.func_index));
                }
                let bt = ir.br_tables.get(table_idx as usize).ok_or_else(|| {
                    format!("verify_ir fn {} bb{bi}: br_table index {table_idx} out of range", ir.func_index)
                })?;
                let mut uniq_set: HashSet<(u32, u32, u32)> = HashSet::with_capacity(bt.unique_edges.len());
                for &(tbb, targs) in &bt.unique_edges {
                    check_edge(ir, bi, tbb.0, targs, "BrTable.unique")?;
                    if !uniq_set.insert((tbb.0, targs.start, targs.len)) {
                        return Err(format!(
                            "verify_ir fn {} bb{bi}: duplicate entry in BrTable::unique_edges",
                            ir.func_index
                        ));
                    }
                    if !succs[bi].contains(&(tbb.0 as usize)) {
                        succs[bi].push(tbb.0 as usize);
                    }
                }
                for &(tbb, targs) in bt.targets.iter().chain(std::iter::once(&bt.default)) {
                    if !uniq_set.contains(&(tbb.0, targs.start, targs.len)) {
                        return Err(format!(
                            "verify_ir fn {} bb{bi}: BrTable edge (bb{}, {}+{}) missing from unique_edges",
                            ir.func_index, tbb.0, targs.start, targs.len
                        ));
                    }
                }
            }
            Term::Return { vals } => {
                check_val_range(ir, vals, "Return")?;
                let rvals = ir.pool_slice(vals);
                if rvals.len() != ir.result_tys.len() {
                    return Err(format!(
                        "verify_ir fn {} bb{bi}: Return arity mismatch ({} vs {})",
                        ir.func_index,
                        rvals.len(),
                        ir.result_tys.len()
                    ));
                }
                for (idx, (&rv, &expected)) in rvals.iter().zip(&ir.result_tys).enumerate() {
                    let actual = val_ty(ir, rv, "Return")?;
                    if actual != expected {
                        return Err(format!(
                            "verify_ir fn {} bb{bi}: Return val {idx} type mismatch ({actual:?} vs {expected:?})",
                            ir.func_index
                        ));
                    }
                }
            }
            Term::Unreachable => {}
        }
    }

    // 2. Compute reachable blocks in RPO and independently build predecessor lists.
    let mut visited = vec![false; n_blocks];
    let mut postorder = Vec::with_capacity(n_blocks);
    let mut dfs: Vec<(usize, usize)> = vec![(0, 0)];
    visited[0] = true;
    while let Some((b, si)) = dfs.last_mut() {
        let cur = *b;
        if *si < succs[cur].len() {
            let nxt = succs[cur][*si];
            *si += 1;
            if !visited[nxt] {
                visited[nxt] = true;
                dfs.push((nxt, 0));
            }
        } else {
            postorder.push(cur);
            dfs.pop();
        }
    }
    postorder.reverse();
    let rpo = postorder;
    let n_rpo = rpo.len();
    let mut rpo_idx = vec![usize::MAX; n_blocks];
    for (oi, &b) in rpo.iter().enumerate() {
        rpo_idx[b] = oi;
    }
    let mut rpo_preds: Vec<Vec<usize>> = vec![Vec::new(); n_rpo];
    for (oi, &b) in rpo.iter().enumerate() {
        for &s in &succs[b] {
            let soi = rpo_idx[s];
            debug_assert!(soi != usize::MAX);
            rpo_preds[soi].push(oi);
        }
    }

    // 3. Cooper-Harvey-Kennedy exact immediate dominators over reachable RPO indices.
    let mut idom = vec![usize::MAX; n_rpo];
    idom[0] = 0;
    let intersect = |idom: &[usize], mut b1: usize, mut b2: usize| -> usize {
        while b1 != b2 {
            while b1 > b2 {
                b1 = idom[b1];
            }
            while b2 > b1 {
                b2 = idom[b2];
            }
        }
        b1
    };
    let mut changed = true;
    while changed {
        changed = false;
        for b in 1..n_rpo {
            let mut new_idom = usize::MAX;
            for &p in &rpo_preds[b] {
                if idom[p] != usize::MAX {
                    new_idom = if new_idom == usize::MAX {
                        p
                    } else {
                        intersect(&idom, p, new_idom)
                    };
                }
            }
            if new_idom != usize::MAX && idom[b] != new_idom {
                idom[b] = new_idom;
                changed = true;
            }
        }
    }
    let dominates = |a: usize, mut b: usize| -> bool {
        while b > a {
            b = idom[b];
        }
        b == a
    };

    // 4. Verify single static definition across reachable blocks and record `(rpo_block, slot)`
    // (`slot == 0` for block params, `inst_idx + 1` for instruction defs).
    let mut def_loc: Vec<Option<(usize, usize)>> = vec![None; n_vals];
    for (oi, &bi) in rpo.iter().enumerate() {
        let bb = &ir.blocks[bi];
        for &p in &bb.params {
            let _ = val_ty(ir, p, "block param")?;
            if def_loc[p.0 as usize].replace((oi, 0)).is_some() {
                return Err(format!("verify_ir fn {}: value v{} defined more than once", ir.func_index, p.0));
            }
        }
        for (ii, inst) in bb.insts.iter().enumerate() {
            let slot = ii + 1;
            if let Some(res) = inst.res {
                let _ = val_ty(ir, res, "inst.res")?;
                if def_loc[res.0 as usize].replace((oi, slot)).is_some() {
                    return Err(format!("verify_ir fn {}: value v{} defined more than once", ir.func_index, res.0));
                }
            }
            match inst.op {
                Op::Call { rets, .. } | Op::CallIndirect { rets, .. } => {
                    check_val_range(ir, rets, "call.rets")?;
                    for &rv in ir.pool_slice(rets) {
                        let _ = val_ty(ir, rv, "call.ret")?;
                        if def_loc[rv.0 as usize].replace((oi, slot)).is_some() {
                            return Err(format!(
                                "verify_ir fn {}: value v{} defined more than once",
                                ir.func_index, rv.0
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let check_dom_use = |v: Val, use_oi: usize, use_slot: usize, ctx: &str| -> Result<(), String> {
        let Some((def_oi, def_slot)) = def_loc.get(v.0 as usize).copied().flatten() else {
            return Err(format!(
                "verify_ir fn {} bb{}: {ctx} uses undefined value v{}",
                ir.func_index, rpo[use_oi], v.0
            ));
        };
        if def_oi == use_oi {
            if def_slot >= use_slot {
                return Err(format!(
                    "verify_ir fn {} bb{}: {ctx} uses v{} before its definition (def slot {def_slot} >= use slot {use_slot})",
                    ir.func_index, rpo[use_oi], v.0
                ));
            }
        } else if !dominates(def_oi, use_oi) {
            return Err(format!(
                "verify_ir fn {} bb{}: {ctx} uses v{} defined in non-dominating bb{}",
                ir.func_index, rpo[use_oi], v.0, rpo[def_oi]
            ));
        }
        Ok(())
    };

    // 5. Verify dominance and typing of every instruction and terminator in reachable blocks.
    for (oi, &bi) in rpo.iter().enumerate() {
        let bb = &ir.blocks[bi];
        for (ii, inst) in bb.insts.iter().enumerate() {
            let slot = ii + 1;
            let mut dom_err: Option<String> = None;
            inst.op.for_each_use(&ir.val_pool, |v| {
                if dom_err.is_none()
                    && let Err(e) = check_dom_use(v, oi, slot, "inst")
                {
                    dom_err = Some(e);
                }
            });
            if let Some(e) = dom_err {
                return Err(e);
            }
            verify_inst_types(ir, module, bi, inst.res, inst.op)?;
        }
        let term_slot = bb.insts.len() + 1;
        let mut dom_err: Option<String> = None;
        bb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
            if dom_err.is_none()
                && let Err(e) = check_dom_use(v, oi, term_slot, "term")
            {
                dom_err = Some(e);
            }
        });
        if let Some(e) = dom_err {
            return Err(e);
        }
    }

    Ok(())
}

fn expect_res(ir: &IrFunc, bi: usize, res: Option<Val>, expected: Ty, name: &str) -> Result<(), String> {
    let r = res.ok_or_else(|| format!("verify_ir fn {} bb{bi}: {name} missing result", ir.func_index))?;
    let actual = val_ty(ir, r, name)?;
    if actual != expected {
        return Err(format!(
            "verify_ir fn {} bb{bi}: {name} result type {actual:?} != expected {expected:?}",
            ir.func_index
        ));
    }
    Ok(())
}

fn expect_no_res(ir: &IrFunc, bi: usize, res: Option<Val>, name: &str) -> Result<(), String> {
    if res.is_some() {
        return Err(format!("verify_ir fn {} bb{bi}: {name} must not have inst.res", ir.func_index));
    }
    Ok(())
}

fn expect_ty(ir: &IrFunc, bi: usize, v: Val, expected: Ty, name: &str) -> Result<(), String> {
    let actual = val_ty(ir, v, name)?;
    if actual != expected {
        return Err(format!(
            "verify_ir fn {} bb{bi}: {name} operand type {actual:?} != expected {expected:?}",
            ir.func_index
        ));
    }
    Ok(())
}

fn verify_inst_types(ir: &IrFunc, module: &X64Module, bi: usize, res: Option<Val>, op: Op) -> Result<(), String> {
    match op {
        Op::ConstI32(_) | Op::MemSize => expect_res(ir, bi, res, Ty::I32, "i32 const/memsize")?,
        Op::ConstI64(_) => expect_res(ir, bi, res, Ty::I64, "ConstI64")?,
        Op::ConstF32(_) => expect_res(ir, bi, res, Ty::F32, "ConstF32")?,
        Op::ConstF64(_) => expect_res(ir, bi, res, Ty::F64, "ConstF64")?,
        Op::ConstV128(idx) => {
            if (idx as usize) >= ir.v128_consts.len() {
                return Err(format!("verify_ir fn {} bb{bi}: ConstV128 idx {idx} out of range", ir.func_index));
            }
            expect_res(ir, bi, res, Ty::V128, "ConstV128")?;
        }
        Op::I32Eqz(a) | Op::I32Clz(a) | Op::I32Ctz(a) | Op::I32Popcnt(a) | Op::I32Extend8S(a) | Op::I32Extend16S(a) => {
            expect_ty(ir, bi, a, Ty::I32, "i32 unop")?;
            expect_res(ir, bi, res, Ty::I32, "i32 unop")?;
        }
        Op::I64Eqz(a) | Op::I32WrapI64(a) => {
            expect_ty(ir, bi, a, Ty::I64, "i64->i32")?;
            expect_res(ir, bi, res, Ty::I32, "i64->i32")?;
        }
        Op::I64Clz(a)
        | Op::I64Ctz(a)
        | Op::I64Popcnt(a)
        | Op::I64Extend8S(a)
        | Op::I64Extend16S(a)
        | Op::I64Extend32S(a) => {
            expect_ty(ir, bi, a, Ty::I64, "i64 unop")?;
            expect_res(ir, bi, res, Ty::I64, "i64 unop")?;
        }
        Op::I64ExtendI32S(a) | Op::I64ExtendI32U(a) => {
            expect_ty(ir, bi, a, Ty::I32, "i32->i64")?;
            expect_res(ir, bi, res, Ty::I64, "i32->i64")?;
        }
        Op::I32Bin(_, a, b) | Op::I32Cmp(_, a, b) => {
            expect_ty(ir, bi, a, Ty::I32, "i32 bin")?;
            expect_ty(ir, bi, b, Ty::I32, "i32 bin")?;
            expect_res(ir, bi, res, Ty::I32, "i32 bin")?;
        }
        Op::I32BinImm(_, a, _) | Op::I32CmpImm(_, a, _) => {
            expect_ty(ir, bi, a, Ty::I32, "i32 bin imm")?;
            expect_res(ir, bi, res, Ty::I32, "i32 bin imm")?;
        }
        Op::I64Bin(_, a, b) => {
            expect_ty(ir, bi, a, Ty::I64, "i64 bin")?;
            expect_ty(ir, bi, b, Ty::I64, "i64 bin")?;
            expect_res(ir, bi, res, Ty::I64, "i64 bin")?;
        }
        Op::I64BinImm(_, a, _) => {
            expect_ty(ir, bi, a, Ty::I64, "i64 bin imm")?;
            expect_res(ir, bi, res, Ty::I64, "i64 bin imm")?;
        }
        Op::I64Cmp(_, a, b) => {
            expect_ty(ir, bi, a, Ty::I64, "i64 cmp")?;
            expect_ty(ir, bi, b, Ty::I64, "i64 cmp")?;
            expect_res(ir, bi, res, Ty::I32, "i64 cmp")?;
        }
        Op::I64CmpImm(_, a, _) => {
            expect_ty(ir, bi, a, Ty::I64, "i64 cmp imm")?;
            expect_res(ir, bi, res, Ty::I32, "i64 cmp imm")?;
        }
        Op::F32Cmp(_, a, b) => {
            expect_ty(ir, bi, a, Ty::F32, "f32 cmp")?;
            expect_ty(ir, bi, b, Ty::F32, "f32 cmp")?;
            expect_res(ir, bi, res, Ty::I32, "f32 cmp")?;
        }
        Op::F64Cmp(_, a, b) => {
            expect_ty(ir, bi, a, Ty::F64, "f64 cmp")?;
            expect_ty(ir, bi, b, Ty::F64, "f64 cmp")?;
            expect_res(ir, bi, res, Ty::I32, "f64 cmp")?;
        }
        Op::F32Un(_, a) => {
            expect_ty(ir, bi, a, Ty::F32, "f32 un")?;
            expect_res(ir, bi, res, Ty::F32, "f32 un")?;
        }
        Op::F64Un(_, a) => {
            expect_ty(ir, bi, a, Ty::F64, "f64 un")?;
            expect_res(ir, bi, res, Ty::F64, "f64 un")?;
        }
        Op::F32Bin(_, a, b) => {
            expect_ty(ir, bi, a, Ty::F32, "f32 bin")?;
            expect_ty(ir, bi, b, Ty::F32, "f32 bin")?;
            expect_res(ir, bi, res, Ty::F32, "f32 bin")?;
        }
        Op::F64Bin(_, a, b) => {
            expect_ty(ir, bi, a, Ty::F64, "f64 bin")?;
            expect_ty(ir, bi, b, Ty::F64, "f64 bin")?;
            expect_res(ir, bi, res, Ty::F64, "f64 bin")?;
        }
        Op::F32DemoteF64(a) => {
            expect_ty(ir, bi, a, Ty::F64, "F32DemoteF64")?;
            expect_res(ir, bi, res, Ty::F32, "F32DemoteF64")?;
        }
        Op::F64PromoteF32(a) => {
            expect_ty(ir, bi, a, Ty::F32, "F64PromoteF32")?;
            expect_res(ir, bi, res, Ty::F64, "F64PromoteF32")?;
        }
        Op::F32ConvertI32S(a) | Op::F32ConvertI32U(a) | Op::F32ReinterpretI32(a) => {
            expect_ty(ir, bi, a, Ty::I32, "i32->f32")?;
            expect_res(ir, bi, res, Ty::F32, "i32->f32")?;
        }
        Op::F32ConvertI64S(a) | Op::F32ConvertI64U(a) => {
            expect_ty(ir, bi, a, Ty::I64, "i64->f32")?;
            expect_res(ir, bi, res, Ty::F32, "i64->f32")?;
        }
        Op::F64ConvertI32S(a) | Op::F64ConvertI32U(a) => {
            expect_ty(ir, bi, a, Ty::I32, "i32->f64")?;
            expect_res(ir, bi, res, Ty::F64, "i32->f64")?;
        }
        Op::F64ConvertI64S(a) | Op::F64ConvertI64U(a) | Op::F64ReinterpretI64(a) => {
            expect_ty(ir, bi, a, Ty::I64, "i64->f64")?;
            expect_res(ir, bi, res, Ty::F64, "i64->f64")?;
        }
        Op::I32ReinterpretF32(a) => {
            expect_ty(ir, bi, a, Ty::F32, "I32ReinterpretF32")?;
            expect_res(ir, bi, res, Ty::I32, "I32ReinterpretF32")?;
        }
        Op::I64ReinterpretF64(a) => {
            expect_ty(ir, bi, a, Ty::F64, "I64ReinterpretF64")?;
            expect_res(ir, bi, res, Ty::I64, "I64ReinterpretF64")?;
        }
        Op::Trunc(top, a) => {
            let src_ty = match top {
                super::ir::TruncOp::I32F32S
                | super::ir::TruncOp::I32F32U
                | super::ir::TruncOp::I64F32S
                | super::ir::TruncOp::I64F32U
                | super::ir::TruncOp::I32F32SatS
                | super::ir::TruncOp::I32F32SatU
                | super::ir::TruncOp::I64F32SatS
                | super::ir::TruncOp::I64F32SatU => Ty::F32,
                _ => Ty::F64,
            };
            expect_ty(ir, bi, a, src_ty, "Trunc")?;
            expect_res(ir, bi, res, top.result_ty(), "Trunc")?;
        }
        Op::Select { cond, a, b } => {
            expect_ty(ir, bi, cond, Ty::I32, "Select.cond")?;
            let at = val_ty(ir, a, "Select.a")?;
            expect_ty(ir, bi, b, at, "Select.b")?;
            expect_res(ir, bi, res, at, "Select")?;
        }
        Op::CmpSelect { kind, a, b, tv, fv } => {
            match kind {
                CmpBranchKind::I32(_) => {
                    expect_ty(ir, bi, a, Ty::I32, "CmpSelect.a")?;
                    expect_ty(ir, bi, b, Ty::I32, "CmpSelect.b")?;
                }
                CmpBranchKind::I32Imm(_, _) => expect_ty(ir, bi, a, Ty::I32, "CmpSelect.a")?,
                CmpBranchKind::I64(_) => {
                    expect_ty(ir, bi, a, Ty::I64, "CmpSelect.a")?;
                    expect_ty(ir, bi, b, Ty::I64, "CmpSelect.b")?;
                }
                CmpBranchKind::I64Imm(_, _) => expect_ty(ir, bi, a, Ty::I64, "CmpSelect.a")?,
                CmpBranchKind::F32(_) => {
                    expect_ty(ir, bi, a, Ty::F32, "CmpSelect.a")?;
                    expect_ty(ir, bi, b, Ty::F32, "CmpSelect.b")?;
                }
                CmpBranchKind::F64(_) => {
                    expect_ty(ir, bi, a, Ty::F64, "CmpSelect.a")?;
                    expect_ty(ir, bi, b, Ty::F64, "CmpSelect.b")?;
                }
            }
            let at = val_ty(ir, tv, "CmpSelect.tv")?;
            expect_ty(ir, bi, fv, at, "CmpSelect.fv")?;
            expect_res(ir, bi, res, at, "CmpSelect")?;
        }
        Op::GlobalGet(g) => {
            let (vt, _) = *module
                .globals
                .get(g as usize)
                .ok_or_else(|| format!("verify_ir fn {} bb{bi}: GlobalGet {g} out of range", ir.func_index))?;
            expect_res(ir, bi, res, Ty::from_val(vt), "GlobalGet")?;
        }
        Op::GlobalSet(g, v) => {
            let (vt, mutable) = *module
                .globals
                .get(g as usize)
                .ok_or_else(|| format!("verify_ir fn {} bb{bi}: GlobalSet {g} out of range", ir.func_index))?;
            if !mutable {
                return Err(format!("verify_ir fn {} bb{bi}: GlobalSet on immutable global {g}", ir.func_index));
            }
            expect_ty(ir, bi, v, Ty::from_val(vt), "GlobalSet")?;
            expect_no_res(ir, bi, res, "GlobalSet")?;
        }
        Op::TableGet { table, idx } => {
            if !module.mut_extern_tables.contains_key(&table) {
                return Err(format!("verify_ir fn {} bb{bi}: TableGet unknown extern table {table}", ir.func_index));
            }
            expect_ty(ir, bi, idx, Ty::I32, "TableGet.idx")?;
            expect_res(ir, bi, res, Ty::I64, "TableGet")?;
        }
        Op::TableSet { table, idx, val } => {
            if !module.mut_extern_tables.contains_key(&table) {
                return Err(format!("verify_ir fn {} bb{bi}: TableSet unknown extern table {table}", ir.func_index));
            }
            expect_ty(ir, bi, idx, Ty::I32, "TableSet.idx")?;
            expect_ty(ir, bi, val, Ty::I64, "TableSet.val")?;
            expect_no_res(ir, bi, res, "TableSet")?;
        }
        Op::BoundsCheck { addr, .. } => {
            expect_ty(ir, bi, addr, Ty::I32, "BoundsCheck.addr")?;
            expect_no_res(ir, bi, res, "BoundsCheck")?;
        }
        Op::Load { kind, addr, .. } => {
            expect_ty(ir, bi, addr, Ty::I32, "Load.addr")?;
            expect_res(ir, bi, res, kind.ty(), "Load")?;
        }
        Op::Store { kind, addr, val, .. } => {
            expect_ty(ir, bi, addr, Ty::I32, "Store.addr")?;
            let vty = match kind {
                super::ir::StoreKind::I8 | super::ir::StoreKind::I16 => {
                    // May store either I32 or I64 wrapped in IR.
                    let actual = val_ty(ir, val, "Store.val")?;
                    if !matches!(actual, Ty::I32 | Ty::I64) {
                        return Err(format!("verify_ir fn {} bb{bi}: narrow Store val must be GP", ir.func_index));
                    }
                    actual
                }
                super::ir::StoreKind::I32 => {
                    let actual = val_ty(ir, val, "Store.val")?;
                    if !matches!(actual, Ty::I32 | Ty::I64) {
                        return Err(format!("verify_ir fn {} bb{bi}: i32 Store val must be GP", ir.func_index));
                    }
                    actual
                }
                super::ir::StoreKind::I64 => Ty::I64,
                super::ir::StoreKind::F32 => Ty::F32,
                super::ir::StoreKind::F64 => Ty::F64,
                super::ir::StoreKind::V128 => Ty::V128,
            };
            expect_ty(ir, bi, val, vty, "Store.val")?;
            expect_no_res(ir, bi, res, "Store")?;
        }
        Op::MemGrow(delta) => {
            expect_ty(ir, bi, delta, Ty::I32, "MemGrow")?;
            expect_res(ir, bi, res, Ty::I32, "MemGrow")?;
        }
        Op::MemCopy { dst, src, len } => {
            expect_ty(ir, bi, dst, Ty::I32, "MemCopy.dst")?;
            expect_ty(ir, bi, src, Ty::I32, "MemCopy.src")?;
            expect_ty(ir, bi, len, Ty::I32, "MemCopy.len")?;
            expect_no_res(ir, bi, res, "MemCopy")?;
        }
        Op::MemFill { dst, val, len } => {
            expect_ty(ir, bi, dst, Ty::I32, "MemFill.dst")?;
            expect_ty(ir, bi, val, Ty::I32, "MemFill.val")?;
            expect_ty(ir, bi, len, Ty::I32, "MemFill.len")?;
            expect_no_res(ir, bi, res, "MemFill")?;
        }
        Op::MemCopyConst { dst, src, len } => {
            if len > 64 {
                return Err(format!("verify_ir fn {} bb{bi}: MemCopyConst len {len} > 64", ir.func_index));
            }
            expect_ty(ir, bi, dst, Ty::I32, "MemCopyConst.dst")?;
            expect_ty(ir, bi, src, Ty::I32, "MemCopyConst.src")?;
            expect_no_res(ir, bi, res, "MemCopyConst")?;
        }
        Op::MemFillConst { dst, val, len } => {
            if len > 64 {
                return Err(format!("verify_ir fn {} bb{bi}: MemFillConst len {len} > 64", ir.func_index));
            }
            expect_ty(ir, bi, dst, Ty::I32, "MemFillConst.dst")?;
            expect_ty(ir, bi, val, Ty::I32, "MemFillConst.val")?;
            expect_no_res(ir, bi, res, "MemFillConst")?;
        }
        Op::V128Splat { lane_bytes, val } => {
            let expected = match lane_bytes {
                1 | 2 | 4 => Ty::I32,
                8 => Ty::I64,
                other => return Err(format!("verify_ir fn {} bb{bi}: bad V128Splat lane_bytes {other}", ir.func_index)),
            };
            expect_ty(ir, bi, val, expected, "V128Splat")?;
            expect_res(ir, bi, res, Ty::V128, "V128Splat")?;
        }
        Op::I64x2ExtractLane { v, lane } => {
            if lane > 1 {
                return Err(format!("verify_ir fn {} bb{bi}: bad I64x2ExtractLane lane {lane}", ir.func_index));
            }
            expect_ty(ir, bi, v, Ty::V128, "I64x2ExtractLane")?;
            expect_res(ir, bi, res, Ty::I64, "I64x2ExtractLane")?;
        }
        Op::I64x2ReplaceLane { v, x, lane } => {
            if lane > 1 {
                return Err(format!("verify_ir fn {} bb{bi}: bad I64x2ReplaceLane lane {lane}", ir.func_index));
            }
            expect_ty(ir, bi, v, Ty::V128, "I64x2ReplaceLane.v")?;
            expect_ty(ir, bi, x, Ty::I64, "I64x2ReplaceLane.x")?;
            expect_res(ir, bi, res, Ty::V128, "I64x2ReplaceLane")?;
        }
        Op::Call { func, args, rets } => {
            expect_no_res(ir, bi, res, "Call")?;
            check_val_range(ir, args, "Call.args")?;
            check_val_range(ir, rets, "Call.rets")?;
            let fty = module
                .func_type(func as usize)
                .ok_or_else(|| format!("verify_ir fn {} bb{bi}: Call func {func} out of range", ir.func_index))?;
            verify_call_sig(ir, bi, &fty.params, &fty.results, args, rets, "Call")?;
        }
        Op::CallIndirect { ci_id, idx, args, rets } => {
            expect_no_res(ir, bi, res, "CallIndirect")?;
            expect_ty(ir, bi, idx, Ty::I32, "CallIndirect.idx")?;
            check_val_range(ir, args, "CallIndirect.args")?;
            check_val_range(ir, rets, "CallIndirect.rets")?;
            let ci = module.ci_sigs.get(ci_id as usize).ok_or_else(|| {
                format!("verify_ir fn {} bb{bi}: CallIndirect ci_id {ci_id} out of range", ir.func_index)
            })?;
            let fty = module.module_types.get(ci.type_idx as usize).ok_or_else(|| {
                format!("verify_ir fn {} bb{bi}: CallIndirect type_idx {} out of range", ir.func_index, ci.type_idx)
            })?;
            verify_call_sig(ir, bi, &fty.params, &fty.results, args, rets, "CallIndirect")?;
        }
        Op::Trap(_) => expect_no_res(ir, bi, res, "Trap")?,
    }
    Ok(())
}

fn verify_call_sig(
    ir: &IrFunc,
    bi: usize,
    params: &[ValType],
    results: &[ValType],
    args: ValRange,
    rets: ValRange,
    ctx: &str,
) -> Result<(), String> {
    let av = ir.pool_slice(args);
    if av.len() != params.len() {
        return Err(format!(
            "verify_ir fn {} bb{bi}: {ctx} arg count {} != {}",
            ir.func_index,
            av.len(),
            params.len()
        ));
    }
    for (&v, &pt) in av.iter().zip(params) {
        expect_ty(ir, bi, v, Ty::from_val(pt), ctx)?;
    }
    let rv = ir.pool_slice(rets);
    if rv.len() != results.len() {
        return Err(format!(
            "verify_ir fn {} bb{bi}: {ctx} ret count {} != {}",
            ir.func_index,
            rv.len(),
            results.len()
        ));
    }
    for (&v, &rt) in rv.iter().zip(results) {
        expect_ty(ir, bi, v, Ty::from_val(rt), ctx)?;
    }
    Ok(())
}

fn spill_width(ty: Ty) -> i32 {
    if ty == Ty::V128 { 16 } else { 8 }
}

/// Verifies register/spill class validity, callee-save tracking, frame alignment & sizing,
/// physical register & spill-slot disjointness across all live sets, clobber safety across
/// calls/helpers/div/shifts, and distinct edge-copy destinations.
pub fn verify_alloc(ir: &IrFunc, alloc: &AllocResult) -> Result<(), String> {
    let n_vals = ir.val_tys.len();
    let n_blocks = ir.blocks.len();
    if alloc.locs.len() != n_vals {
        return Err(format!(
            "verify_alloc fn {}: locs.len() ({}) != val_tys.len() ({n_vals})",
            ir.func_index,
            alloc.locs.len()
        ));
    }
    if alloc.block_order.is_empty() || alloc.block_order[0] != 0 {
        return Err(format!("verify_alloc fn {}: block_order must start with bb0", ir.func_index));
    }
    if alloc.frame_size < 0 || alloc.frame_size > MAX_FRAME_BYTES {
        return Err(format!(
            "verify_alloc fn {}: invalid frame_size {}",
            ir.func_index, alloc.frame_size
        ));
    }
    let mut seen_blocks = HashSet::with_capacity(alloc.block_order.len());
    for &b in &alloc.block_order {
        if (b as usize) >= n_blocks || !seen_blocks.insert(b) {
            return Err(format!("verify_alloc fn {}: invalid/duplicate block bb{b} in block_order", ir.func_index));
        }
    }
    let mut used_callee_set = HashSet::new();
    for &g in &alloc.used_callee_gp {
        if !GP_CALLEE_SAVED.contains(&g) || !used_callee_set.insert(g) {
            return Err(format!("verify_alloc fn {}: invalid used_callee_gp {g:?}", ir.func_index));
        }
    }
    let mut used_callee_xmm_set = HashSet::new();
    for &x in &alloc.used_callee_xmm {
        if !XMM_CALLEE_SAVED.contains(&x) || !used_callee_xmm_set.insert(x) {
            return Err(format!("verify_alloc fn {}: invalid used_callee_xmm {x:?}", ir.func_index));
        }
    }

    let spill_base = (alloc.ret_buf_off + alloc.ret_buf_bytes + 15) & !15;
    let saved_xmm_bytes = (alloc.used_callee_xmm.len() as i32) * 16;
    if !alloc.used_callee_xmm.is_empty()
        && (alloc.saved_xmm_base < spill_base
            || (alloc.saved_xmm_base & 15) != 0
            || alloc.saved_xmm_base + saved_xmm_bytes > alloc.saved_ret_ptr_off.unwrap_or(alloc.frame_size))
    {
        return Err(format!(
            "verify_alloc fn {}: invalid saved_xmm_base {} (len={}, spill_base={}, frame={})",
            ir.func_index,
            alloc.saved_xmm_base,
            alloc.used_callee_xmm.len(),
            spill_base,
            alloc.frame_size
        ));
    }
    let spill_limit = if !alloc.used_callee_xmm.is_empty() {
        alloc.saved_xmm_base
    } else {
        alloc.saved_ret_ptr_off.unwrap_or(alloc.frame_size)
    };
    let self_ret_layout = layout_rets(&ir.result_tys)?;
    if self_ret_layout.stack_bytes > 0 {
        let Some(off) = alloc.saved_ret_ptr_off else {
            return Err(format!("verify_alloc fn {}: missing saved_ret_ptr_off for overflow returns", ir.func_index));
        };
        let min_ret_ptr_off = if !alloc.used_callee_xmm.is_empty() {
            alloc.saved_xmm_base + saved_xmm_bytes
        } else {
            spill_base
        };
        if off < min_ret_ptr_off || off + 8 > alloc.frame_size || (off & 7) != 0 {
            return Err(format!(
                "verify_alloc fn {}: invalid saved_ret_ptr_off {off} (min={min_ret_ptr_off}, frame={})",
                ir.func_index, alloc.frame_size
            ));
        }
    }

    // Check every block reachable from bb0 is present in block_order and build succs.
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n_blocks];
    let mut block_to_ord: HashMap<u32, usize> = HashMap::with_capacity(alloc.block_order.len());
    for (oi, &b) in alloc.block_order.iter().enumerate() {
        block_to_ord.insert(b, oi);
    }
    for &b in &alloc.block_order {
        let bb = &ir.blocks[b as usize];
        bb.term.for_each_succ(&ir.br_tables, |s| {
            if !succs[b as usize].contains(&(s.0 as usize)) {
                succs[b as usize].push(s.0 as usize);
            }
        });
        for &s in &succs[b as usize] {
            if !block_to_ord.contains_key(&(s as u32)) {
                return Err(format!(
                    "verify_alloc fn {}: reachable block bb{s} missing from block_order",
                    ir.func_index
                ));
            }
        }
    }

    // Collect defined values across reachable blocks and check their location validity.
    let mut defined_in_reach = vec![false; n_vals];
    let mut has_any_clobber_or_call = false;
    for &b in &alloc.block_order {
        let bb = &ir.blocks[b as usize];
        for &p in &bb.params {
            defined_in_reach[p.0 as usize] = true;
        }
        for inst in &bb.insts {
            if let Some(r) = inst.res {
                defined_in_reach[r.0 as usize] = true;
            }
            match inst.op {
                Op::Call { args, rets, .. } | Op::CallIndirect { args, rets, .. } => {
                    has_any_clobber_or_call = true;
                    let arg_tys: Vec<Ty> = ir.pool_slice(args).iter().map(|&v| ir.val_tys[v.0 as usize]).collect();
                    let arg_layout = layout_args(&arg_tys)?;
                    if arg_layout.stack_bytes > alloc.outgoing_arg_bytes {
                        return Err(format!(
                            "verify_alloc fn {}: outgoing_arg_bytes ({}) < needed ({})",
                            ir.func_index, alloc.outgoing_arg_bytes, arg_layout.stack_bytes
                        ));
                    }
                    let ret_tys: Vec<Ty> = ir.pool_slice(rets).iter().map(|&v| ir.val_tys[v.0 as usize]).collect();
                    let ret_layout = layout_rets(&ret_tys)?;
                    if ret_layout.stack_bytes > alloc.ret_buf_bytes {
                        return Err(format!(
                            "verify_alloc fn {}: ret_buf_bytes ({}) < needed ({})",
                            ir.func_index, alloc.ret_buf_bytes, ret_layout.stack_bytes
                        ));
                    }
                    for &rv in ir.pool_slice(rets) {
                        defined_in_reach[rv.0 as usize] = true;
                    }
                }
                Op::MemCopy { .. } | Op::MemFill { .. } | Op::MemGrow(_) | Op::Trunc(_, _) => {
                    has_any_clobber_or_call = true;
                }
                _ => {}
            }
        }
    }
    if has_any_clobber_or_call {
        let pushes = (1 + alloc.used_callee_gp.len() as i32) * 8;
        if ((pushes + alloc.frame_size) & 15) != 0 {
            return Err(format!(
                "verify_alloc fn {}: misaligned stack frame (pushes={pushes}, frame_size={})",
                ir.func_index, alloc.frame_size
            ));
        }
    }

    for (vi, &is_def) in defined_in_reach.iter().enumerate() {
        if !is_def {
            continue;
        }
        let ty = ir.val_tys[vi];
        match alloc.locs[vi] {
            Loc::Gp(g) => {
                if ty.is_fp() {
                    return Err(format!("verify_alloc fn {}: FP value v{vi} ({ty:?}) in GP {g:?}", ir.func_index));
                }
                if !GP_ALLOC.contains(&g) {
                    return Err(format!("verify_alloc fn {}: value v{vi} in non-allocatable GP {g:?}", ir.func_index));
                }
                if matches!(g, Gp::Rbx | Gp::Rbp) && !used_callee_set.contains(&g) {
                    return Err(format!("verify_alloc fn {}: callee-saved {g:?} used without saving", ir.func_index));
                }
            }
            Loc::Xmm(x) => {
                if !ty.is_fp() {
                    return Err(format!("verify_alloc fn {}: GP value v{vi} ({ty:?}) in XMM {x:?}", ir.func_index));
                }
                if !XMM_ALLOC.contains(&x) {
                    return Err(format!("verify_alloc fn {}: value v{vi} in non-allocatable XMM {x:?}", ir.func_index));
                }
                if XMM_CALLEE_SAVED.contains(&x) && !used_callee_xmm_set.contains(&x) {
                    return Err(format!(
                        "verify_alloc fn {}: callee-saved {x:?} used without saving",
                        ir.func_index
                    ));
                }
            }
            Loc::Spill(off) => {
                let w = spill_width(ty);
                let align_mask = w - 1;
                if off < spill_base || off + w > spill_limit || (off & align_mask) != 0 {
                    return Err(format!(
                        "verify_alloc fn {}: invalid spill Loc::Spill({off}) for v{vi} ({ty:?}, width {w}, range {spill_base}..{spill_limit})",
                        ir.func_index
                    ));
                }
            }
        }
    }

    // Independently compute exact live-in/live-out sets (over ALL values) on reachable blocks.
    let n_ord = alloc.block_order.len();
    let mut live_in: Vec<HashSet<u32>> = vec![HashSet::new(); n_ord];
    let mut live_out: Vec<HashSet<u32>> = vec![HashSet::new(); n_ord];
    let mut changed = true;
    while changed {
        changed = false;
        for oi in (0..n_ord).rev() {
            let bi = alloc.block_order[oi] as usize;
            let bb = &ir.blocks[bi];
            let mut out_set: HashSet<u32> = HashSet::new();
            for &s in &succs[bi] {
                let soi = block_to_ord[&(s as u32)];
                for &v in &live_in[soi] {
                    out_set.insert(v);
                }
            }
            let mut cur = out_set.clone();
            bb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
                cur.insert(v.0);
            });
            for inst in bb.insts.iter().rev() {
                if let Some(r) = inst.res {
                    cur.remove(&r.0);
                }
                match inst.op {
                    Op::Call { rets, .. } | Op::CallIndirect { rets, .. } => {
                        for &rv in ir.pool_slice(rets) {
                            cur.remove(&rv.0);
                        }
                    }
                    _ => {}
                }
                inst.op.for_each_use(&ir.val_pool, |v| {
                    cur.insert(v.0);
                });
            }
            for &p in &bb.params {
                cur.remove(&p.0);
            }
            if out_set != live_out[oi] || cur != live_in[oi] {
                live_out[oi] = out_set;
                live_in[oi] = cur;
                changed = true;
            }
        }
    }

    let check_disjoint = |set: &HashSet<u32>, ctx: &str| -> Result<(), String> {
        let mut gp_owner: [Option<u32>; 16] = [None; 16];
        let mut xmm_owner: [Option<u32>; 16] = [None; 16];
        let mut spills: Vec<(i32, i32, u32)> = Vec::new();
        for &vi in set {
            match alloc.locs[vi as usize] {
                Loc::Gp(g) => {
                    if let Some(other) = gp_owner[g.id() as usize].replace(vi) {
                        return Err(format!(
                            "verify_alloc fn {} ({ctx}): live values v{other} and v{vi} share GP {g:?}",
                            ir.func_index
                        ));
                    }
                }
                Loc::Xmm(x) => {
                    if let Some(other) = xmm_owner[x.id() as usize].replace(vi) {
                        return Err(format!(
                            "verify_alloc fn {} ({ctx}): live values v{other} and v{vi} share XMM {x:?}",
                            ir.func_index
                        ));
                    }
                }
                Loc::Spill(off) => {
                    let w = spill_width(ir.val_tys[vi as usize]);
                    spills.push((off, off + w, vi));
                }
            }
        }
        spills.sort_by_key(|&(s, e, _)| (s, e));
        for w in spills.windows(2) {
            if w[0].1 > w[1].0 {
                return Err(format!(
                    "verify_alloc fn {} ({ctx}): live values v{} [{}..{}) and v{} [{}..{}) have overlapping spill slots",
                    ir.func_index, w[0].2, w[0].0, w[0].1, w[1].2, w[1].0, w[1].1
                ));
            }
        }
        Ok(())
    };

    // Walk backward through each block verifying disjointness and clobber safety at every point.
    for (oi, &b) in alloc.block_order.iter().enumerate() {
        let bb = &ir.blocks[b as usize];
        // Verify all params of this block occupy distinct physical locations (for parallel edge moves).
        if bb.params.len() > 1 {
            let param_set: HashSet<u32> = bb.params.iter().map(|v| v.0).collect();
            check_disjoint(&param_set, &format!("bb{b} params"))?;
        }
        let mut live = live_out[oi].clone();
        check_disjoint(&live, &format!("bb{b} live_out"))?;
        bb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
            live.insert(v.0);
        });
        check_disjoint(&live, &format!("bb{b} term"))?;

        for (ii, inst) in bb.insts.iter().enumerate().rev() {
            // Collect values defined by this instruction.
            let mut defs: Vec<u32> = Vec::new();
            if let Some(r) = inst.res {
                defs.push(r.0);
            }
            match inst.op {
                Op::Call { rets, .. } | Op::CallIndirect { rets, .. } => {
                    for &rv in ir.pool_slice(rets) {
                        defs.push(rv.0);
                    }
                }
                _ => {}
            }
            // All values defined by this instruction + all values live after it must be mutually disjoint.
            for &d in &defs {
                live.insert(d);
            }
            check_disjoint(&live, &format!("bb{b} inst {ii} post"))?;
            for &d in &defs {
                live.remove(&d);
            }
            // Now `live` is the EXACT set of values live strictly ACROSS `inst`: check clobbers!
            let (gp_clob, xmm_clob) = match inst.op {
                Op::Call { .. } | Op::CallIndirect { .. } => (MASK_GP_CALLER, MASK_XMM_CALLER),
                Op::MemCopy { .. } => helper_clobbers(HelperKind::MemCopy),
                Op::MemFill { .. } => helper_clobbers(HelperKind::MemFill),
                Op::MemGrow(_) => helper_clobbers(HelperKind::MemGrow),
                Op::Trunc(_, _) => helper_clobbers(HelperKind::Trunc),
                Op::I32Bin(op, _, _) | Op::I64Bin(op, _, _) => match op {
                    IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU => (MASK_RAX_RDX, 0),
                    IntBinOp::Shl | IntBinOp::ShrS | IntBinOp::ShrU | IntBinOp::Rotl | IntBinOp::Rotr => (MASK_RCX, 0),
                    _ => (0, 0),
                },
                Op::I64BinImm(IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU, _, _) => {
                    (MASK_RAX_RDX, 0)
                }
                _ => (0, 0),
            };
            if gp_clob != 0 || xmm_clob != 0 {
                for &vi in &live {
                    match alloc.locs[vi as usize] {
                        Loc::Gp(g) if (gp_clob & (1 << g.id())) != 0 => {
                            return Err(format!(
                                "verify_alloc fn {} bb{b} inst {ii}: live value v{vi} in {g:?} clobbered by {:?}",
                                ir.func_index, inst.op
                            ));
                        }
                        Loc::Xmm(x) if (xmm_clob & (1 << x.id())) != 0 => {
                            return Err(format!(
                                "verify_alloc fn {} bb{b} inst {ii}: live value v{vi} in {x:?} clobbered by {:?}",
                                ir.func_index, inst.op
                            ));
                        }
                        _ => {}
                    }
                }
            }
            inst.op.for_each_use(&ir.val_pool, |v| {
                live.insert(v.0);
            });
            check_disjoint(&live, &format!("bb{b} inst {ii} pre"))?;
        }
        // At block entry, all params + live_in must be mutually disjoint.
        for &p in &bb.params {
            live.insert(p.0);
        }
        check_disjoint(&live, &format!("bb{b} entry"))?;
    }

    Ok(())
}
