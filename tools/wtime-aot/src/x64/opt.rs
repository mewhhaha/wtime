//! Stage 2 SSA IR optimization pipeline & interprocedural analyses (`#![forbid(unsafe_code)]`).
//!
//! Dependency-ordered passes:
//! 1. `inline_small_helpers`: budgeted bottom-up inlining of non-recursive small helpers (`<= 45` insns).
//! 2. `rebuild_cfg_and_prune`: unreachable-block pruning, single-pred jump-block merging, exact `bb.preds`,
//!    Cooper–Harvey–Kennedy dominator tree, and natural-loop discovery.
//! 3. `compute_module_effects`: transitive interprocedural memory/grow/global/table/trap/termination summaries.
//! 4. `run_licm`: conservative loop-invariant code motion of strictly non-trapping pure ops, safe `GlobalGet`,
//!    and safe `MemSize` into dedicated loop preheaders.
//! 5. `run_dom_gvn_and_bce`: dominator-tree scoped GVN/CSE, `GlobalGet`/`MemSize`/`TableGet`/alias-safe `Load`
//!    CSE, cross-block dominator `checked_max` bounds-check elimination, and wrap-safe addressing-mode folding.
//! 6. `run_stack_and_trap_elision`: strictly coupled 2-phase native-frame stack-check elision (Step 8a)
//!    followed by direct-call trap-check elision (Step 8b).

use std::collections::{HashMap, HashSet};

use super::ir::{
    Block, BlockId, BrTableData, CmpBranchKind, CmpOp, FloatBinOp, FloatUnOp, Inst, IntBinOp,
    IrFunc, LoadKind, Op, StoreKind, Term, TruncOp, Ty, Val, ValRange, optimize_ir_with_budget,
};
use super::module::{
    CompileBudget, MAX_BLOCKS_PER_FN, MAX_IR_VALS_PER_FN, MAX_VAL_POOL_PER_FN, X64Module,
};
use super::regalloc::AllocResult;

type Res<T> = Result<T, String>;

const INLINE_MAX_CALLEE_INSTS: usize = 32;
const INLINE_MAX_CALLEE_BLOCKS: usize = 10;
const INLINE_MAX_CALLEE_VALS: usize = 96;
const INLINE_MAX_LOOP_CALLEE_INSTS: usize = 56;
const INLINE_MAX_LOOP_CALLEE_BLOCKS: usize = 14;
const INLINE_MAX_LOOP_CALLEE_VALS: usize = 160;
const INLINE_MAX_SINGLE_CALLEE_INSTS: usize = 64;
const INLINE_MAX_SINGLE_CALLEE_BLOCKS: usize = 16;
const INLINE_MAX_SINGLE_CALLEE_VALS: usize = 192;
const INLINE_MAX_CALLER_INSTS: usize = 1536;
const INLINE_MAX_CALLER_BLOCKS: usize = 256;
const INLINE_MAX_CALLER_VALS: usize = 3072;
const INLINE_MAX_ADDED_PER_CALLER: usize = 256;
const INLINE_MAX_MODULE_ADDED_INSTS: usize = 4_000_000;
const LICM_MAX_HOIST_PER_LOOP: usize = 12;
const ELIDE_MAX_SUBTREE_STACK_BYTES: i32 = 8192;
const PURE_HELPER_STACK_BYTES: i32 = 64;

// ============================================================================
// 1. CFG Rebuild, Single-Pred Block Merging, Dominators (CHK), & Natural Loops
// ============================================================================

pub struct CfgDom {
    pub rpo: Vec<u32>,
    pub rpo_pos: Vec<u32>,
    pub preds: Vec<Vec<u32>>,
    pub succs: Vec<Vec<u32>>,
    pub idom: Vec<u32>,
    pub dom_children: Vec<Vec<u32>>,
}

impl CfgDom {
    pub fn compute_with_budget(ir: &IrFunc, budget: Option<&CompileBudget>) -> Res<Self> {
        if let Some(bg) = budget {
            bg.check_deadline()?;
        }
        let n = ir.blocks.len();
        let mut succs: Vec<Vec<u32>> = vec![Vec::new(); n];
        let mut succ_seen = vec![u32::MAX; n];
        for (bi, bb) in ir.blocks.iter().enumerate() {
            if let Some(bg) = budget
                && (bi & 4095) == 4095
            {
                bg.check_deadline()?;
            }
            let stamp = bi as u32;
            bb.term.for_each_succ(&ir.br_tables, |s| {
                let sid = s.0 as usize;
                if sid < n && succ_seen[sid] != stamp {
                    succ_seen[sid] = stamp;
                    succs[bi].push(s.0);
                }
            });
        }
        // Iterative DFS postorder -> reverse = RPO.
        let mut visited = vec![false; n];
        let mut postorder = Vec::with_capacity(n);
        if n > 0 {
            visited[0] = true;
            let mut dfs: Vec<(u32, usize)> = vec![(0, 0)];
            let mut dfs_steps = 0usize;
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
        }
        postorder.reverse();
        let rpo = postorder;
        let mut rpo_pos = vec![u32::MAX; n];
        for (oi, &b) in rpo.iter().enumerate() {
            rpo_pos[b as usize] = oi as u32;
        }
        let mut preds: Vec<Vec<u32>> = vec![Vec::new(); n];
        for &b in &rpo {
            for &s in &succs[b as usize] {
                if rpo_pos[s as usize] != u32::MAX {
                    preds[s as usize].push(b);
                }
            }
        }
        // Cooper–Harvey–Kennedy exact iterative dominator algorithm.
        let mut idom = vec![u32::MAX; n];
        if n > 0 {
            idom[0] = 0;
        }
        let intersect = |idom: &[u32], rpo_pos: &[u32], mut b1: u32, mut b2: u32| -> u32 {
            while b1 != b2 {
                while rpo_pos[b1 as usize] > rpo_pos[b2 as usize] {
                    b1 = idom[b1 as usize];
                }
                while rpo_pos[b2 as usize] > rpo_pos[b1 as usize] {
                    b2 = idom[b2 as usize];
                }
            }
            b1
        };
        let mut changed = true;
        let mut dom_steps = 0usize;
        while changed {
            changed = false;
            for &b in rpo.iter().skip(1) {
                dom_steps += 1;
                if let Some(bg) = budget
                    && (dom_steps & 4095) == 0
                {
                    bg.check_deadline()?;
                }
                let mut new_idom = u32::MAX;
                for &p in &preds[b as usize] {
                    if idom[p as usize] != u32::MAX {
                        new_idom = if new_idom == u32::MAX {
                            p
                        } else {
                            intersect(&idom, &rpo_pos, p, new_idom)
                        };
                    }
                }
                if new_idom != u32::MAX && idom[b as usize] != new_idom {
                    idom[b as usize] = new_idom;
                    changed = true;
                }
            }
        }
        let mut dom_children: Vec<Vec<u32>> = vec![Vec::new(); n];
        for &b in rpo.iter().skip(1) {
            let p = idom[b as usize];
            if p != u32::MAX && p != b {
                dom_children[p as usize].push(b);
            }
        }
        Ok(CfgDom {
            rpo,
            rpo_pos,
            preds,
            succs,
            idom,
            dom_children,
        })
    }

    #[inline]
    pub fn dominates(&self, a: u32, mut b: u32) -> bool {
        if (a as usize) >= self.rpo_pos.len() || (b as usize) >= self.rpo_pos.len() {
            return false;
        }
        if self.rpo_pos[a as usize] == u32::MAX || self.rpo_pos[b as usize] == u32::MAX {
            return false;
        }
        if a == b {
            return true;
        }
        while b != 0 && self.rpo_pos[b as usize] > self.rpo_pos[a as usize] {
            b = self.idom[b as usize];
        }
        b == a
    }
}

fn resolve_subst(subst: &[Val], mut v: Val) -> Val {
    let limit = subst.len();
    let mut hops = 0usize;
    while (v.0 as usize) < limit && subst[v.0 as usize] != v && hops < limit {
        v = subst[v.0 as usize];
        hops += 1;
    }
    v
}

/// Compresses every substitution chain to its root in O(N) amortized so arbitrary-length
/// reverse-order merge chains resolve in 1 hop without any fixed depth cap.
fn flatten_subst(subst: &mut [Val]) {
    let n = subst.len();
    for i in 0..n {
        let mut cur = Val(i as u32);
        let mut steps = 0usize;
        while (cur.0 as usize) < n && subst[cur.0 as usize] != cur && steps < n {
            cur = subst[cur.0 as usize];
            steps += 1;
        }
        let root = cur;
        let mut walk = Val(i as u32);
        let mut csteps = 0usize;
        while (walk.0 as usize) < n && subst[walk.0 as usize] != root && csteps < n {
            let nxt = subst[walk.0 as usize];
            subst[walk.0 as usize] = root;
            walk = nxt;
            csteps += 1;
        }
    }
}

fn rewrite_op_uses(op: &mut Op, pool: &mut [Val], subst: &[Val]) {
    let r = |v: &mut Val| {
        *v = resolve_subst(subst, *v);
    };
    match op {
        Op::ConstI32(_)
        | Op::ConstI64(_)
        | Op::ConstF32(_)
        | Op::ConstF64(_)
        | Op::ConstV128(_)
        | Op::GlobalGet(_)
        | Op::MemSize
        | Op::Trap(_) => {}
        Op::I32Eqz(a)
        | Op::I64Eqz(a)
        | Op::I32Clz(a)
        | Op::I32Ctz(a)
        | Op::I32Popcnt(a)
        | Op::I64Clz(a)
        | Op::I64Ctz(a)
        | Op::I64Popcnt(a)
        | Op::I32WrapI64(a)
        | Op::I64ExtendI32S(a)
        | Op::I64ExtendI32U(a)
        | Op::I32Extend8S(a)
        | Op::I32Extend16S(a)
        | Op::I64Extend8S(a)
        | Op::I64Extend16S(a)
        | Op::I64Extend32S(a)
        | Op::I32BinImm(_, a, _)
        | Op::I64BinImm(_, a, _)
        | Op::I32CmpImm(_, a, _)
        | Op::I64CmpImm(_, a, _)
        | Op::F32Un(_, a)
        | Op::F64Un(_, a)
        | Op::F32DemoteF64(a)
        | Op::F64PromoteF32(a)
        | Op::F32ConvertI32S(a)
        | Op::F32ConvertI32U(a)
        | Op::F32ConvertI64S(a)
        | Op::F32ConvertI64U(a)
        | Op::F64ConvertI32S(a)
        | Op::F64ConvertI32U(a)
        | Op::F64ConvertI64S(a)
        | Op::F64ConvertI64U(a)
        | Op::I32ReinterpretF32(a)
        | Op::I64ReinterpretF64(a)
        | Op::F32ReinterpretI32(a)
        | Op::F64ReinterpretI64(a)
        | Op::Trunc(_, a)
        | Op::GlobalSet(_, a)
        | Op::TableGet { idx: a, .. }
        | Op::BoundsCheck { addr: a, .. }
        | Op::Load { addr: a, .. }
        | Op::MemGrow(a)
        | Op::V128Splat { val: a, .. }
        | Op::I64x2ExtractLane { v: a, .. } => r(a),
        Op::I32Bin(_, a, b)
        | Op::I64Bin(_, a, b)
        | Op::I32Cmp(_, a, b)
        | Op::I64Cmp(_, a, b)
        | Op::F32Cmp(_, a, b)
        | Op::F64Cmp(_, a, b)
        | Op::F32Bin(_, a, b)
        | Op::F64Bin(_, a, b)
        | Op::TableSet { idx: a, val: b, .. }
        | Op::Store {
            addr: a, val: b, ..
        }
        | Op::I64x2ReplaceLane { v: a, x: b, .. }
        | Op::MemCopyConst { dst: a, src: b, .. }
        | Op::MemFillConst { dst: a, val: b, .. } => {
            r(a);
            r(b);
        }
        Op::Select { cond, a, b } => {
            r(cond);
            r(a);
            r(b);
        }
        Op::CmpSelect { kind, a, b, tv, fv } => {
            r(a);
            if !matches!(kind, CmpBranchKind::I32Imm(..) | CmpBranchKind::I64Imm(..)) {
                r(b);
            } else {
                *b = *a;
            }
            r(tv);
            r(fv);
        }
        Op::MemCopy { dst, src, len } => {
            r(dst);
            r(src);
            r(len);
        }
        Op::MemFill { dst, val, len } => {
            r(dst);
            r(val);
            r(len);
        }
        Op::Call { args, .. } => {
            for v in &mut pool[args.start as usize..args.start as usize + args.len as usize] {
                r(v);
            }
        }
        Op::CallIndirect { idx, args, .. } => {
            for v in &mut pool[args.start as usize..args.start as usize + args.len as usize] {
                r(v);
            }
            r(idx);
        }
    }
}

fn rewrite_term_uses(term: &mut Term, pool: &mut [Val], tables: &[BrTableData], subst: &[Val]) {
    let rw_range = |pool: &mut [Val], r: ValRange| {
        for v in &mut pool[r.start as usize..r.start as usize + r.len as usize] {
            *v = resolve_subst(subst, *v);
        }
    };
    match *term {
        Term::Jump { args, .. } => rw_range(pool, args),
        Term::BranchIf {
            ref mut cond,
            then_args,
            else_args,
            ..
        } => {
            *cond = resolve_subst(subst, *cond);
            rw_range(pool, then_args);
            rw_range(pool, else_args);
        }
        Term::CmpBranch {
            kind,
            ref mut a,
            ref mut b,
            then_args,
            else_args,
            ..
        } => {
            *a = resolve_subst(subst, *a);
            if !matches!(kind, CmpBranchKind::I32Imm(..) | CmpBranchKind::I64Imm(..)) {
                *b = resolve_subst(subst, *b);
            } else {
                *b = *a;
            }
            rw_range(pool, then_args);
            rw_range(pool, else_args);
        }
        Term::BrTable {
            ref mut idx,
            table_idx,
        } => {
            *idx = resolve_subst(subst, *idx);
            for &(_, a) in &tables[table_idx as usize].unique_edges {
                rw_range(pool, a);
            }
        }
        Term::Return { vals } => rw_range(pool, vals),
        Term::Unreachable => {}
    }
}

fn apply_subst_to_func(ir: &mut IrFunc, subst: &[Val]) {
    let mut pool = std::mem::take(&mut ir.val_pool);
    for bb in &mut ir.blocks {
        for inst in &mut bb.insts {
            rewrite_op_uses(&mut inst.op, &mut pool, subst);
        }
        rewrite_term_uses(&mut bb.term, &mut pool, &ir.br_tables, subst);
    }
    ir.val_pool = pool;
}

/// Removes unreachable blocks, merges trivial single-predecessor `Jump` chains (`A -> B` with
/// `preds[B] == 1` and `B != 0`), compacts block IDs and `br_tables`, recomputes exact `bb.preds`
/// and `is_loop_header`, and returns the exact `CfgDom` for the compacted CFG.
pub fn rebuild_cfg_and_prune(ir: &mut IrFunc, budget: Option<&CompileBudget>) -> Res<CfgDom> {
    if let Some(bg) = budget {
        bg.check_deadline()?;
    }
    // Step 1: Merge linear `A -> Jump(B, args)` chains where `B` has exactly 1 incoming edge (`B != 0`, `B != A`).
    let n = ir.blocks.len();
    if n > 1 {
        // Count exact edge occurrences (note: a BranchIf with then == else counts as 2 edges, preventing unsafe merge!).
        let mut edge_in = vec![0u32; n];
        let mut reachable = vec![false; n];
        let mut stack = vec![0u32];
        reachable[0] = true;
        let mut walk_steps = 0usize;
        while let Some(b) = stack.pop() {
            walk_steps += 1;
            if let Some(bg) = budget
                && (walk_steps & 4095) == 0
            {
                bg.check_deadline()?;
            }
            let bb = &ir.blocks[b as usize];
            match bb.term {
                Term::Jump { target, .. } => {
                    edge_in[target.0 as usize] = edge_in[target.0 as usize].saturating_add(1);
                    if !reachable[target.0 as usize] {
                        reachable[target.0 as usize] = true;
                        stack.push(target.0);
                    }
                }
                Term::BranchIf {
                    then_bb, else_bb, ..
                }
                | Term::CmpBranch {
                    then_bb, else_bb, ..
                } => {
                    for t in [then_bb, else_bb] {
                        edge_in[t.0 as usize] = edge_in[t.0 as usize].saturating_add(1);
                        if !reachable[t.0 as usize] {
                            reachable[t.0 as usize] = true;
                            stack.push(t.0);
                        }
                    }
                }
                Term::BrTable { table_idx, .. } => {
                    let td = &ir.br_tables[table_idx as usize];
                    for &(t, _) in &td.unique_edges {
                        edge_in[t.0 as usize] = edge_in[t.0 as usize].saturating_add(1);
                        if !reachable[t.0 as usize] {
                            reachable[t.0 as usize] = true;
                            stack.push(t.0);
                        }
                    }
                }
                Term::Return { .. } | Term::Unreachable => {}
            }
        }

        let mut subst: Vec<Val> = Vec::new();
        let mut any_subst = false;
        let mut merged_away = vec![false; n];
        for bi in 0..n {
            if let Some(bg) = budget
                && (bi & 4095) == 4095
            {
                bg.check_deadline()?;
            }
            if !reachable[bi] || merged_away[bi] {
                continue;
            }
            while let Term::Jump { target, args } = ir.blocks[bi].term {
                let ti = target.0 as usize;
                if ti == 0 || ti == bi || merged_away[ti] || edge_in[ti] != 1 {
                    break;
                }
                // Merge `ti` into `bi`!
                let t_params = std::mem::take(&mut ir.blocks[ti].params);
                let t_insts = std::mem::take(&mut ir.blocks[ti].insts);
                let t_term = std::mem::replace(&mut ir.blocks[ti].term, Term::Unreachable);
                if !t_params.is_empty() {
                    if subst.is_empty() {
                        subst = (0..ir.val_tys.len() as u32).map(Val).collect();
                    }
                    for (&p, &a) in t_params.iter().zip(ir.pool_slice(args)) {
                        let resolved = resolve_subst(&subst, a);
                        subst[p.0 as usize] = resolved;
                        any_subst = true;
                    }
                }
                ir.blocks[bi].insts.extend(t_insts);
                ir.blocks[bi].term = t_term;
                merged_away[ti] = true;
                edge_in[ti] = 0;
            }
        }
        if any_subst {
            flatten_subst(&mut subst);
            apply_subst_to_func(ir, &subst);
        }
    }

    // Step 2: Compute reachability/dominators once; only recompute if dead blocks were compacted out.
    let mut dom = CfgDom::compute_with_budget(ir, budget)?;
    let n = ir.blocks.len();
    if dom.rpo.len() < n {
        let mut remap = vec![u32::MAX; n];
        let mut new_blocks = Vec::with_capacity(dom.rpo.len());
        let old_blocks = std::mem::take(&mut ir.blocks);
        let mut old_opt: Vec<Option<Block>> = old_blocks.into_iter().map(Some).collect();
        for (new_id, &old_id) in dom.rpo.iter().enumerate() {
            remap[old_id as usize] = new_id as u32;
            new_blocks.push(old_opt[old_id as usize].take().unwrap());
        }
        let map_b = |b: BlockId| BlockId(remap[b.0 as usize]);
        let old_br_tables = std::mem::take(&mut ir.br_tables);
        let mut old_bt_opt: Vec<Option<BrTableData>> =
            old_br_tables.into_iter().map(Some).collect();
        let mut new_br_tables: Vec<BrTableData> = Vec::new();
        let mut bt_remap = vec![u32::MAX; old_bt_opt.len()];
        for bb in &mut new_blocks {
            match &mut bb.term {
                Term::Jump { target, .. } => *target = map_b(*target),
                Term::BranchIf {
                    then_bb, else_bb, ..
                }
                | Term::CmpBranch {
                    then_bb, else_bb, ..
                } => {
                    *then_bb = map_b(*then_bb);
                    *else_bb = map_b(*else_bb);
                }
                Term::BrTable { table_idx, .. } => {
                    let old_ti = *table_idx as usize;
                    let new_ti = if bt_remap[old_ti] != u32::MAX {
                        bt_remap[old_ti]
                    } else {
                        let mut td = old_bt_opt[old_ti]
                            .take()
                            .expect("br_table entry referenced once");
                        td.default.0 = map_b(td.default.0);
                        for (t, _) in &mut td.targets {
                            *t = map_b(*t);
                        }
                        for (t, _) in &mut td.unique_edges {
                            *t = map_b(*t);
                        }
                        let idx = new_br_tables.len() as u32;
                        new_br_tables.push(td);
                        bt_remap[old_ti] = idx;
                        idx
                    };
                    *table_idx = new_ti;
                }
                Term::Return { .. } | Term::Unreachable => {}
            }
        }
        ir.br_tables = new_br_tables;
        ir.blocks = new_blocks;
        dom = CfgDom::compute_with_budget(ir, budget)?;
    }

    // Recompute accurate `bb.preds` and `bb.is_loop_header`.
    let mut pred_counts = vec![0u32; ir.blocks.len()];
    let mut loop_hdrs = vec![false; ir.blocks.len()];
    for (bi, bb) in ir.blocks.iter().enumerate() {
        let b = bi as u32;
        bb.term.for_each_succ(&ir.br_tables, |s| {
            pred_counts[s.0 as usize] = pred_counts[s.0 as usize].saturating_add(1);
            if dom.dominates(s.0, b) {
                loop_hdrs[s.0 as usize] = true;
            }
        });
    }
    for (bi, bb) in ir.blocks.iter_mut().enumerate() {
        bb.preds = pred_counts[bi];
        bb.is_loop_header = loop_hdrs[bi];
    }
    Ok(dom)
}

/// Eliminates redundant block parameters (`phi(x, x) -> x`, `phi(init, self) -> init`) and runs
/// exact global mark-sweep dead-code + dead-block-parameter elimination (breaking dead loop-carried
/// phi cycles in O(V + E) so unmodified or loop-local Wasm locals never generate edge-copy storms
/// or starve registers).
pub fn prune_block_params(
    ir: &mut IrFunc,
    dom: &CfgDom,
    budget: Option<&CompileBudget>,
) -> Res<bool> {
    if ir.blocks.len() <= 1 {
        return Ok(false);
    }
    let mut changed_any = false;

    for _round in 0..3 {
        if let Some(bg) = budget {
            bg.check_deadline()?;
        }
        let n_blocks = ir.blocks.len();
        let n_vals = ir.val_tys.len();

        // Collect defining block for every SSA value and incoming edge arg ranges per block.
        let mut def_block = vec![u32::MAX; n_vals];
        let mut in_edges: Vec<Vec<ValRange>> = vec![Vec::new(); n_blocks];
        for (bi, bb) in ir.blocks.iter().enumerate() {
            let b = bi as u32;
            for &p in &bb.params {
                def_block[p.0 as usize] = b;
            }
            for inst in &bb.insts {
                if let Some(r) = inst.res {
                    def_block[r.0 as usize] = b;
                }
                match inst.op {
                    Op::Call { rets, .. } | Op::CallIndirect { rets, .. } => {
                        for &rv in ir.pool_slice(rets) {
                            def_block[rv.0 as usize] = b;
                        }
                    }
                    _ => {}
                }
            }
            match bb.term {
                Term::Jump { target, args } => in_edges[target.0 as usize].push(args),
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
                    in_edges[then_bb.0 as usize].push(then_args);
                    in_edges[else_bb.0 as usize].push(else_args);
                }
                Term::BrTable { table_idx, .. } => {
                    let td = &ir.br_tables[table_idx as usize];
                    for &(t, a) in &td.unique_edges {
                        in_edges[t.0 as usize].push(a);
                    }
                }
                Term::Return { .. } | Term::Unreachable => {}
            }
        }

        // Part A: Redundant block-param elimination (`bi > 0` only; never touch entry ABI `bb0.params`).
        let mut subst: Vec<Val> = (0..n_vals as u32).map(Val).collect();
        let mut any_subst = false;
        let mut steps = 0usize;
        for _sweep in 0..4 {
            let mut sweep_changed = false;
            for (bi, edges) in in_edges.iter().enumerate().skip(1) {
                let params = &ir.blocks[bi].params;
                if params.is_empty() || edges.is_empty() {
                    continue;
                }
                for (k, &p) in params.iter().enumerate() {
                    steps += 1;
                    if let Some(bg) = budget
                        && (steps & 4095) == 0
                    {
                        bg.check_deadline()?;
                    }
                    if subst[p.0 as usize] != p {
                        continue;
                    }
                    let mut unique: Option<Val> = None;
                    let mut conflict = false;
                    for &r in edges {
                        let raw = ir.val_pool[r.start as usize + k];
                        let a = resolve_subst(&subst, raw);
                        if a == p {
                            continue;
                        }
                        match unique {
                            None => unique = Some(a),
                            Some(u) if u == a => {}
                            Some(_) => {
                                conflict = true;
                                break;
                            }
                        }
                    }
                    if !conflict
                        && let Some(u) = unique
                        && u != p
                        && def_block[u.0 as usize] != u32::MAX
                        && dom.dominates(def_block[u.0 as usize], bi as u32)
                    {
                        subst[p.0 as usize] = u;
                        any_subst = true;
                        sweep_changed = true;
                    }
                }
            }
            if !sweep_changed {
                break;
            }
        }
        if any_subst {
            flatten_subst(&mut subst);
            apply_subst_to_func(ir, &subst);
            changed_any = true;
        }

        // Part B: Exact O(V + E) global mark-sweep liveness over instructions + block params
        // (breaks arbitrary dead loop-carried phi cycles!).
        #[derive(Clone, Copy)]
        enum DefSite {
            None,
            Inst(u32, u32),
            BlockParam(u32, u32),
        }
        let mut def_site = vec![DefSite::None; n_vals];
        for (bi, bb) in ir.blocks.iter().enumerate() {
            if bi > 0 {
                for (k, &p) in bb.params.iter().enumerate() {
                    def_site[p.0 as usize] = DefSite::BlockParam(bi as u32, k as u32);
                }
            }
            for (ii, inst) in bb.insts.iter().enumerate() {
                if let Some(r) = inst.res {
                    def_site[r.0 as usize] = DefSite::Inst(bi as u32, ii as u32);
                }
            }
        }

        let mut live_val = vec![false; n_vals];
        let mut live_inst: Vec<Vec<bool>> = ir
            .blocks
            .iter()
            .map(|bb| vec![false; bb.insts.len()])
            .collect();
        let mut worklist: Vec<Val> = Vec::new();
        let mark = |v: Val, live_val: &mut [bool], worklist: &mut Vec<Val>| {
            let vi = v.0 as usize;
            if !live_val[vi] {
                live_val[vi] = true;
                worklist.push(v);
            }
        };

        // Entry ABI params stay live.
        for &p in &ir.blocks[0].params {
            live_val[p.0 as usize] = true;
        }
        // Seed side-effecting instructions and terminator control/return operands (NOT edge args).
        for (bi, bb) in ir.blocks.iter().enumerate() {
            for (ii, inst) in bb.insts.iter().enumerate() {
                if inst.op.has_side_effect() {
                    live_inst[bi][ii] = true;
                    inst.op
                        .for_each_use(&ir.val_pool, |v| mark(v, &mut live_val, &mut worklist));
                }
            }
            match bb.term {
                Term::Jump { .. } | Term::Unreachable => {}
                Term::BranchIf { cond, .. } => mark(cond, &mut live_val, &mut worklist),
                Term::CmpBranch { kind, a, b, .. } => {
                    mark(a, &mut live_val, &mut worklist);
                    if !matches!(
                        kind,
                        CmpBranchKind::I32Imm(_, _) | CmpBranchKind::I64Imm(_, _)
                    ) {
                        mark(b, &mut live_val, &mut worklist);
                    }
                }
                Term::BrTable { idx, .. } => mark(idx, &mut live_val, &mut worklist),
                Term::Return { vals } => {
                    for &rv in ir.pool_slice(vals) {
                        mark(rv, &mut live_val, &mut worklist);
                    }
                }
            }
        }

        let mut wl_steps = 0usize;
        while let Some(v) = worklist.pop() {
            wl_steps += 1;
            if let Some(bg) = budget
                && (wl_steps & 4095) == 0
            {
                bg.check_deadline()?;
            }
            match def_site[v.0 as usize] {
                DefSite::None => {}
                DefSite::Inst(bi, ii) => {
                    let (bu, iu) = (bi as usize, ii as usize);
                    if !live_inst[bu][iu] {
                        live_inst[bu][iu] = true;
                        ir.blocks[bu].insts[iu]
                            .op
                            .for_each_use(&ir.val_pool, |u| mark(u, &mut live_val, &mut worklist));
                    }
                }
                DefSite::BlockParam(bi, k) => {
                    let bu = bi as usize;
                    let ku = k as usize;
                    for &r in &in_edges[bu] {
                        let arg_v = ir.val_pool[r.start as usize + ku];
                        mark(arg_v, &mut live_val, &mut worklist);
                    }
                }
            }
        }

        let mut drop_any_inst = false;
        let mut drop_any_param = false;
        for (bi, bb) in ir.blocks.iter().enumerate() {
            if live_inst[bi].iter().any(|&k| !k) {
                drop_any_inst = true;
            }
            if bi > 0 && bb.params.iter().any(|p| !live_val[p.0 as usize]) {
                drop_any_param = true;
            }
        }

        if drop_any_inst {
            for (bi, bb) in ir.blocks.iter_mut().enumerate() {
                if live_inst[bi].iter().any(|&k| !k) {
                    let mut idx = 0usize;
                    bb.insts.retain(|_| {
                        let keep = live_inst[bi][idx];
                        idx += 1;
                        keep
                    });
                }
            }
            changed_any = true;
        }

        if drop_any_param {
            let mut keep_param: Vec<Vec<bool>> = Vec::with_capacity(n_blocks);
            for (bi, bb) in ir.blocks.iter_mut().enumerate() {
                if bi == 0 {
                    keep_param.push(vec![true; bb.params.len()]);
                } else {
                    let kp: Vec<bool> = bb.params.iter().map(|p| live_val[p.0 as usize]).collect();
                    let mut idx = 0usize;
                    bb.params.retain(|_| {
                        let k = kp[idx];
                        idx += 1;
                        k
                    });
                    keep_param.push(kp);
                }
            }

            // Compact `val_pool`, filtering edge argument ranges to kept destination params.
            let old_pool = std::mem::take(&mut ir.val_pool);
            let mut new_pool: Vec<Val> = Vec::with_capacity(old_pool.len());
            let push_verbatim = |new_pool: &mut Vec<Val>, r: ValRange| -> ValRange {
                if r.len == 0 {
                    return ValRange::EMPTY;
                }
                let start = new_pool.len() as u32;
                new_pool.extend_from_slice(
                    &old_pool[r.start as usize..r.start as usize + r.len as usize],
                );
                ValRange { start, len: r.len }
            };
            let push_filtered =
                |new_pool: &mut Vec<Val>, target: BlockId, r: ValRange| -> ValRange {
                    if r.len == 0 {
                        return ValRange::EMPTY;
                    }
                    let kp = &keep_param[target.0 as usize];
                    let start = new_pool.len() as u32;
                    let slice = &old_pool[r.start as usize..r.start as usize + r.len as usize];
                    for (&v, &keep) in slice.iter().zip(kp.iter()) {
                        if keep {
                            new_pool.push(v);
                        }
                    }
                    let new_len = (new_pool.len() as u32) - start;
                    if new_len == 0 {
                        ValRange::EMPTY
                    } else {
                        ValRange {
                            start,
                            len: new_len,
                        }
                    }
                };

            for bb in &mut ir.blocks {
                for inst in &mut bb.insts {
                    match &mut inst.op {
                        Op::Call { args, rets, .. } | Op::CallIndirect { args, rets, .. } => {
                            *args = push_verbatim(&mut new_pool, *args);
                            *rets = push_verbatim(&mut new_pool, *rets);
                        }
                        _ => {}
                    }
                }
                match &mut bb.term {
                    Term::Jump { target, args } => {
                        *args = push_filtered(&mut new_pool, *target, *args);
                    }
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
                        *then_args = push_filtered(&mut new_pool, *then_bb, *then_args);
                        *else_args = push_filtered(&mut new_pool, *else_bb, *else_args);
                    }
                    Term::Return { vals } => {
                        *vals = push_verbatim(&mut new_pool, *vals);
                    }
                    Term::BrTable { .. } | Term::Unreachable => {}
                }
            }
            for td in &mut ir.br_tables {
                let mut edge_map: HashMap<(BlockId, u32, u32), ValRange> = HashMap::new();
                let mut seen_new: HashSet<(BlockId, u32, u32)> = HashSet::new();
                let mut new_unique: Vec<(BlockId, ValRange)> =
                    Vec::with_capacity(td.unique_edges.len());
                let mut map_edge = |e: (BlockId, ValRange)| -> (BlockId, ValRange) {
                    let key = (e.0, e.1.start, e.1.len);
                    if let Some(&nr) = edge_map.get(&key) {
                        return (e.0, nr);
                    }
                    let nr = push_filtered(&mut new_pool, e.0, e.1);
                    edge_map.insert(key, nr);
                    if seen_new.insert((e.0, nr.start, nr.len)) {
                        new_unique.push((e.0, nr));
                    }
                    (e.0, nr)
                };
                for e in &mut td.targets {
                    *e = map_edge(*e);
                }
                td.default = map_edge(td.default);
                td.unique_edges = new_unique;
            }
            ir.val_pool = new_pool;
            changed_any = true;
        }

        if !any_subst && !drop_any_param {
            break;
        }
    }
    Ok(changed_any)
}

/// Early per-function CFG compaction + dead/redundant block-param pruning before inlining so
/// small helpers drop dead loop/if local params and inline cleanly without bloating `val_pool`.
pub fn prune_early_block_params(ir: &mut IrFunc, budget: Option<&CompileBudget>) -> Res<()> {
    if ir.blocks.len() <= 1 {
        return Ok(());
    }
    let dom = rebuild_cfg_and_prune(ir, budget)?;
    let _ = prune_block_params(ir, &dom, budget)?;
    Ok(())
}

// ============================================================================
// 2. Budgeted Small-Leaf Inlining
// ============================================================================

/// Fallibly computes Tarjan SCCs from an adjacency list of defined-function edges,
/// returning `None` on allocation failure so optional cache preparation can degrade cleanly.
pub fn try_call_graph_sccs_from_edges(edges: &[Vec<usize>]) -> Option<(Vec<bool>, Vec<usize>)> {
    let n = edges.len();
    let mut index = Vec::new();
    index.try_reserve_exact(n).ok()?;
    index.resize(n, u32::MAX);
    let mut lowlink = Vec::new();
    lowlink.try_reserve_exact(n).ok()?;
    lowlink.resize(n, 0u32);
    let mut on_stack = Vec::new();
    on_stack.try_reserve_exact(n).ok()?;
    on_stack.resize(n, false);
    let mut stack: Vec<usize> = Vec::new();
    stack.try_reserve_exact(n).ok()?;
    let mut cur_idx = 0u32;
    let mut in_cycle = Vec::new();
    in_cycle.try_reserve_exact(n).ok()?;
    in_cycle.resize(n, false);
    let mut topo_bottom_up = Vec::new();
    topo_bottom_up.try_reserve_exact(n).ok()?;
    let mut frames: Vec<(usize, usize)> = Vec::new();
    frames.try_reserve_exact(n).ok()?;
    let mut scc: Vec<usize> = Vec::new();
    scc.try_reserve_exact(n).ok()?;

    // Iterative Tarjan SCC so deep call chains never overflow the host stack.
    for start in 0..n {
        if index[start] != u32::MAX {
            continue;
        }
        index[start] = cur_idx;
        lowlink[start] = cur_idx;
        cur_idx += 1;
        stack.push(start);
        on_stack[start] = true;
        frames.push((start, 0));

        while let Some((v, ei)) = frames.last_mut() {
            let v = *v;
            if *ei < edges[v].len() {
                let w = edges[v][*ei];
                *ei += 1;
                if index[w] == u32::MAX {
                    index[w] = cur_idx;
                    lowlink[w] = cur_idx;
                    cur_idx += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    frames.push((w, 0));
                } else if on_stack[w] {
                    lowlink[v] = lowlink[v].min(index[w]);
                }
            } else {
                frames.pop();
                if let Some(&(parent, _)) = frames.last() {
                    lowlink[parent] = lowlink[parent].min(lowlink[v]);
                }
                if lowlink[v] == index[v] {
                    scc.clear();
                    while let Some(w) = stack.pop() {
                        on_stack[w] = false;
                        scc.push(w);
                        if w == v {
                            break;
                        }
                    }
                    let cyclic =
                        scc.len() > 1 || (scc.len() == 1 && edges[scc[0]].contains(&scc[0]));
                    for &node in &scc {
                        if cyclic {
                            in_cycle[node] = true;
                        }
                        topo_bottom_up.push(node);
                    }
                }
            }
        }
    }
    Some((in_cycle, topo_bottom_up))
}

/// Computes Tarjan SCCs from an adjacency list of defined-function edges and returns `(in_cycle, topo_bottom_up)`.
pub fn call_graph_sccs_from_edges(edges: &[Vec<usize>]) -> (Vec<bool>, Vec<usize>) {
    let n = edges.len();
    let mut index = vec![u32::MAX; n];
    let mut lowlink = vec![0u32; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut cur_idx = 0u32;
    let mut in_cycle = vec![false; n];
    let mut topo_bottom_up = Vec::with_capacity(n);

    // Iterative Tarjan SCC so deep call chains never overflow the host stack.
    let mut frames: Vec<(usize, usize)> = Vec::new();
    for start in 0..n {
        if index[start] != u32::MAX {
            continue;
        }
        index[start] = cur_idx;
        lowlink[start] = cur_idx;
        cur_idx += 1;
        stack.push(start);
        on_stack[start] = true;
        frames.push((start, 0));

        while let Some((v, ei)) = frames.last_mut() {
            let v = *v;
            if *ei < edges[v].len() {
                let w = edges[v][*ei];
                *ei += 1;
                if index[w] == u32::MAX {
                    index[w] = cur_idx;
                    lowlink[w] = cur_idx;
                    cur_idx += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    frames.push((w, 0));
                } else if on_stack[w] {
                    lowlink[v] = lowlink[v].min(index[w]);
                }
            } else {
                frames.pop();
                if let Some(&(parent, _)) = frames.last() {
                    lowlink[parent] = lowlink[parent].min(lowlink[v]);
                }
                if lowlink[v] == index[v] {
                    let mut scc = Vec::new();
                    while let Some(w) = stack.pop() {
                        on_stack[w] = false;
                        scc.push(w);
                        if w == v {
                            break;
                        }
                    }
                    let cyclic =
                        scc.len() > 1 || (scc.len() == 1 && edges[scc[0]].contains(&scc[0]));
                    for &node in &scc {
                        if cyclic {
                            in_cycle[node] = true;
                        }
                        topo_bottom_up.push(node);
                    }
                }
            }
        }
    }
    (in_cycle, topo_bottom_up)
}

/// Collects sorted `(callee_bi, call_count)` pairs for direct defined callees of `ir`.
pub fn collect_pre_inline_callees(ir: &IrFunc, n_imports: usize, n_defs: usize) -> Vec<(u32, u32)> {
    let mut raw: Vec<u32> = Vec::new();
    for bb in &ir.blocks {
        for inst in &bb.insts {
            if let Op::Call { func, .. } = inst.op {
                let f = func as usize;
                if f >= n_imports {
                    let c = f - n_imports;
                    if c < n_defs {
                        raw.push(c as u32);
                    }
                }
            }
        }
    }
    raw.sort_unstable();
    let mut out: Vec<(u32, u32)> = Vec::new();
    for c in raw {
        if let Some(last) = out.last_mut()
            && last.0 == c
        {
            last.1 = last.1.saturating_add(1);
        } else {
            out.push((c, 1));
        }
    }
    out
}

/// Fallible variant of `collect_pre_inline_callees` for optional incremental cache preparation.
pub fn try_collect_pre_inline_callees(
    ir: &IrFunc,
    n_imports: usize,
    n_defs: usize,
) -> Option<Vec<(u32, u32)>> {
    let mut raw: Vec<u32> = Vec::new();
    for bb in &ir.blocks {
        for inst in &bb.insts {
            if let Op::Call { func, .. } = inst.op {
                let f = func as usize;
                if f >= n_imports {
                    let c = f - n_imports;
                    if c < n_defs {
                        if raw.len() == raw.capacity() {
                            raw.try_reserve(1).ok()?;
                        }
                        raw.push(c as u32);
                    }
                }
            }
        }
    }
    raw.sort_unstable();
    let mut out: Vec<(u32, u32)> = Vec::new();
    for c in raw {
        if let Some(last) = out.last_mut()
            && last.0 == c
        {
            last.1 = last.1.saturating_add(1);
        } else {
            if out.len() == out.capacity() {
                out.try_reserve(1).ok()?;
            }
            out.push((c, 1));
        }
    }
    Some(out)
}

/// Conservative check for whether `ir` could qualify as an inline callee under any threshold
/// (general, inside-loop, or single-call-site).
pub fn is_inlinable_ir(ir: &IrFunc) -> bool {
    ir.br_tables.is_empty()
        && count_insts(ir) <= INLINE_MAX_SINGLE_CALLEE_INSTS
        && ir.blocks.len() <= INLINE_MAX_SINGLE_CALLEE_BLOCKS
        && ir.val_tys.len() <= INLINE_MAX_SINGLE_CALLEE_VALS
}

/// Fallible fast O(B + E) detection of blocks belonging to natural loops (backedges in RPO) in `ir`.
fn try_compute_in_loop_blocks(ir: &IrFunc) -> Option<Vec<bool>> {
    let n = ir.blocks.len();
    let mut in_loop = Vec::new();
    in_loop.try_reserve_exact(n).ok()?;
    in_loop.resize(n, false);
    if n <= 1 {
        return Some(in_loop);
    }
    let mut succs: Vec<Vec<usize>> = Vec::new();
    succs.try_reserve_exact(n).ok()?;
    for _ in 0..n {
        succs.push(Vec::new());
    }
    let mut seen = Vec::new();
    seen.try_reserve_exact(n).ok()?;
    seen.resize(n, u32::MAX);
    for (bi, bb) in ir.blocks.iter().enumerate() {
        let stamp = bi as u32;
        let mut oom = false;
        bb.term.for_each_succ(&ir.br_tables, |s| {
            if oom {
                return;
            }
            let sid = s.0 as usize;
            if sid < n && seen[sid] != stamp {
                seen[sid] = stamp;
                let row = &mut succs[bi];
                if row.len() == row.capacity() && row.try_reserve(1).is_err() {
                    oom = true;
                    return;
                }
                row.push(sid);
            }
        });
        if oom {
            return None;
        }
    }
    let mut visited = Vec::new();
    visited.try_reserve_exact(n).ok()?;
    visited.resize(n, false);
    let mut postorder = Vec::new();
    postorder.try_reserve_exact(n).ok()?;
    visited[0] = true;
    let mut dfs = Vec::new();
    dfs.try_reserve_exact(n).ok()?;
    dfs.push((0usize, 0usize));
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
    let mut rpo_pos = Vec::new();
    rpo_pos.try_reserve_exact(n).ok()?;
    rpo_pos.resize(n, u32::MAX);
    for (oi, &b) in postorder.iter().enumerate() {
        rpo_pos[b] = oi as u32;
    }
    let mut preds: Vec<Vec<usize>> = Vec::new();
    preds.try_reserve_exact(n).ok()?;
    for _ in 0..n {
        preds.push(Vec::new());
    }
    let mut backedges: Vec<(usize, usize)> = Vec::new();
    for &b in &postorder {
        let bp = rpo_pos[b];
        for &s in &succs[b] {
            let sp = rpo_pos[s];
            if sp != u32::MAX {
                let prow = &mut preds[s];
                if prow.len() == prow.capacity() {
                    prow.try_reserve(1).ok()?;
                }
                prow.push(b);
                if sp <= bp {
                    if backedges.len() == backedges.capacity() {
                        backedges.try_reserve(1).ok()?;
                    }
                    backedges.push((b, s));
                }
            }
        }
    }
    if backedges.is_empty() {
        return Some(in_loop);
    }
    let mut mark = Vec::new();
    mark.try_reserve_exact(n).ok()?;
    mark.resize(n, u32::MAX);
    let mut wl = Vec::new();
    wl.try_reserve_exact(n).ok()?;
    for (li, &(latch, hdr)) in backedges.iter().take(512).enumerate() {
        let stamp = li as u32;
        let hp = rpo_pos[hdr];
        in_loop[hdr] = true;
        mark[hdr] = stamp;
        if latch != hdr {
            in_loop[latch] = true;
            mark[latch] = stamp;
            wl.clear();
            wl.push(latch);
            while let Some(cur) = wl.pop() {
                for &p in &preds[cur] {
                    if rpo_pos[p] > hp && mark[p] != stamp {
                        mark[p] = stamp;
                        in_loop[p] = true;
                        wl.push(p);
                    }
                }
            }
        }
    }
    Some(in_loop)
}

/// Fallibly computes which defined functions have external references (exported, `start`, or in `call_indirect`).
fn try_compute_externally_referenced(module: &X64Module, n_defs: usize) -> Option<Vec<bool>> {
    let n_imports = module.imports.len();
    let mut ext = Vec::new();
    ext.try_reserve_exact(n_defs).ok()?;
    ext.resize(n_defs, false);
    for &(_, f) in &module.exported_funcs {
        if f >= n_imports && f - n_imports < n_defs {
            ext[f - n_imports] = true;
        }
    }
    if let Some(st) = module.start {
        let f = st as usize;
        if f >= n_imports && f - n_imports < n_defs {
            ext[f - n_imports] = true;
        }
    }
    for ci in &module.ci_sigs {
        for &(_, fi) in &ci.targets {
            let f = fi as usize;
            if f >= n_imports && f - n_imports < n_defs {
                ext[f - n_imports] = true;
            }
        }
    }
    Some(ext)
}

fn count_insts(ir: &IrFunc) -> usize {
    ir.blocks.iter().map(|b| b.insts.len()).sum()
}

fn clone_callee_into_caller(
    caller: &mut IrFunc,
    callee: &IrFunc,
    call_bb: usize,
    call_inst_idx: usize,
    call_args: ValRange,
    call_rets: ValRange,
) -> Res<()> {
    let base_val = caller.val_tys.len() as u32;
    let base_v128 = caller.v128_consts.len() as u32;
    let base_bt = caller.br_tables.len() as u32;

    // Pre-check ALL caps (`val_tys`, `blocks`, `val_pool`) BEFORE mutating `caller` so any
    // rejected inline leaves `caller` 100% untouched.
    if caller.val_tys.len().saturating_add(callee.val_tys.len()) > MAX_IR_VALS_PER_FN {
        return Err("x64: inline val limit".into());
    }
    if caller
        .blocks
        .len()
        .saturating_add(callee.blocks.len())
        .saturating_add(1)
        > MAX_BLOCKS_PER_FN
    {
        return Err("x64: inline block limit".into());
    }
    let mut needed_pool = 0usize;
    for td in &callee.br_tables {
        for &(_, r) in &td.unique_edges {
            needed_pool = needed_pool.saturating_add(r.len as usize);
        }
    }
    for cb in &callee.blocks {
        for inst in &cb.insts {
            match inst.op {
                Op::Call { args, rets, .. } | Op::CallIndirect { args, rets, .. } => {
                    needed_pool = needed_pool
                        .saturating_add(args.len as usize)
                        .saturating_add(rets.len as usize);
                }
                _ => {}
            }
        }
        match cb.term {
            Term::Jump { args, .. } => needed_pool = needed_pool.saturating_add(args.len as usize),
            Term::BranchIf {
                then_args,
                else_args,
                ..
            }
            | Term::CmpBranch {
                then_args,
                else_args,
                ..
            } => {
                needed_pool = needed_pool
                    .saturating_add(then_args.len as usize)
                    .saturating_add(else_args.len as usize);
            }
            Term::Return { vals } => needed_pool = needed_pool.saturating_add(vals.len as usize),
            Term::BrTable { .. } | Term::Unreachable => {}
        }
    }
    if caller.val_pool.len().saturating_add(needed_pool) > MAX_VAL_POOL_PER_FN {
        return Err("x64: inline val_pool limit".into());
    }

    caller.val_tys.extend_from_slice(&callee.val_tys);
    caller.v128_consts.extend_from_slice(&callee.v128_consts);

    // Split caller block at `call_inst_idx`:
    let ret_vals: Vec<Val> = caller.pool_slice(call_rets).to_vec();
    let tail_insts = caller.blocks[call_bb].insts.split_off(call_inst_idx + 1);
    caller.blocks[call_bb].insts.pop(); // remove Op::Call
    let orig_term = std::mem::replace(&mut caller.blocks[call_bb].term, Term::Unreachable);

    let cont_bb = BlockId(caller.blocks.len() as u32);
    caller.blocks.push(Block {
        params: ret_vals,
        insts: tail_insts,
        term: orig_term,
        is_loop_header: false,
        preds: 0,
    });

    let base_bb = caller.blocks.len() as u32;
    let map_v = |v: Val| Val(base_val + v.0);
    let map_b = |b: BlockId| BlockId(base_bb + b.0);
    let map_range = |pool: &mut Vec<Val>, src_pool: &[Val], r: ValRange| -> Res<ValRange> {
        if r.len == 0 {
            return Ok(ValRange::EMPTY);
        }
        let start = pool.len();
        if start + r.len as usize > MAX_VAL_POOL_PER_FN {
            return Err("x64: inline val_pool limit".into());
        }
        for &v in &src_pool[r.start as usize..r.start as usize + r.len as usize] {
            pool.push(map_v(v));
        }
        Ok(ValRange {
            start: start as u32,
            len: r.len,
        })
    };

    // Clone br_tables with remapped targets and arg ranges.
    for td in &callee.br_tables {
        let mut targets = Vec::with_capacity(td.targets.len());
        let mut unique_edges = Vec::with_capacity(td.unique_edges.len());
        let mut edge_map: HashMap<(BlockId, u32, u32), (BlockId, ValRange)> = HashMap::new();
        let mut map_edge =
            |e: (BlockId, ValRange), pool: &mut Vec<Val>| -> Res<(BlockId, ValRange)> {
                let key = (e.0, e.1.start, e.1.len);
                if let Some(&mapped) = edge_map.get(&key) {
                    return Ok(mapped);
                }
                let nr = map_range(pool, &callee.val_pool, e.1)?;
                let res = (map_b(e.0), nr);
                edge_map.insert(key, res);
                unique_edges.push(res);
                Ok(res)
            };
        for &e in &td.targets {
            targets.push(map_edge(e, &mut caller.val_pool)?);
        }
        let default = map_edge(td.default, &mut caller.val_pool)?;
        caller.br_tables.push(BrTableData {
            targets,
            default,
            unique_edges,
        });
    }

    // Clone callee blocks, remapping Val, BlockId, v128_consts, br_tables, and replacing Return -> Jump(cont_bb).
    for cb in &callee.blocks {
        let params: Vec<Val> = cb.params.iter().map(|&v| map_v(v)).collect();
        let mut insts = Vec::with_capacity(cb.insts.len());
        for inst in &cb.insts {
            let res = inst.res.map(map_v);
            let op = match inst.op {
                Op::ConstI32(c) => Op::ConstI32(c),
                Op::ConstI64(c) => Op::ConstI64(c),
                Op::ConstF32(c) => Op::ConstF32(c),
                Op::ConstF64(c) => Op::ConstF64(c),
                Op::ConstV128(idx) => Op::ConstV128(base_v128 + idx),
                Op::I32Eqz(a) => Op::I32Eqz(map_v(a)),
                Op::I64Eqz(a) => Op::I64Eqz(map_v(a)),
                Op::I32Clz(a) => Op::I32Clz(map_v(a)),
                Op::I32Ctz(a) => Op::I32Ctz(map_v(a)),
                Op::I32Popcnt(a) => Op::I32Popcnt(map_v(a)),
                Op::I64Clz(a) => Op::I64Clz(map_v(a)),
                Op::I64Ctz(a) => Op::I64Ctz(map_v(a)),
                Op::I64Popcnt(a) => Op::I64Popcnt(map_v(a)),
                Op::I32WrapI64(a) => Op::I32WrapI64(map_v(a)),
                Op::I64ExtendI32S(a) => Op::I64ExtendI32S(map_v(a)),
                Op::I64ExtendI32U(a) => Op::I64ExtendI32U(map_v(a)),
                Op::I32Extend8S(a) => Op::I32Extend8S(map_v(a)),
                Op::I32Extend16S(a) => Op::I32Extend16S(map_v(a)),
                Op::I64Extend8S(a) => Op::I64Extend8S(map_v(a)),
                Op::I64Extend16S(a) => Op::I64Extend16S(map_v(a)),
                Op::I64Extend32S(a) => Op::I64Extend32S(map_v(a)),
                Op::I32Bin(o, a, b) => Op::I32Bin(o, map_v(a), map_v(b)),
                Op::I64Bin(o, a, b) => Op::I64Bin(o, map_v(a), map_v(b)),
                Op::I32BinImm(o, a, imm) => Op::I32BinImm(o, map_v(a), imm),
                Op::I64BinImm(o, a, imm) => Op::I64BinImm(o, map_v(a), imm),
                Op::I32Cmp(o, a, b) => Op::I32Cmp(o, map_v(a), map_v(b)),
                Op::I32CmpImm(o, a, imm) => Op::I32CmpImm(o, map_v(a), imm),
                Op::I64Cmp(o, a, b) => Op::I64Cmp(o, map_v(a), map_v(b)),
                Op::I64CmpImm(o, a, imm) => Op::I64CmpImm(o, map_v(a), imm),
                Op::F32Cmp(o, a, b) => Op::F32Cmp(o, map_v(a), map_v(b)),
                Op::F64Cmp(o, a, b) => Op::F64Cmp(o, map_v(a), map_v(b)),
                Op::F32Un(o, a) => Op::F32Un(o, map_v(a)),
                Op::F64Un(o, a) => Op::F64Un(o, map_v(a)),
                Op::F32Bin(o, a, b) => Op::F32Bin(o, map_v(a), map_v(b)),
                Op::F64Bin(o, a, b) => Op::F64Bin(o, map_v(a), map_v(b)),
                Op::F32DemoteF64(a) => Op::F32DemoteF64(map_v(a)),
                Op::F64PromoteF32(a) => Op::F64PromoteF32(map_v(a)),
                Op::F32ConvertI32S(a) => Op::F32ConvertI32S(map_v(a)),
                Op::F32ConvertI32U(a) => Op::F32ConvertI32U(map_v(a)),
                Op::F32ConvertI64S(a) => Op::F32ConvertI64S(map_v(a)),
                Op::F32ConvertI64U(a) => Op::F32ConvertI64U(map_v(a)),
                Op::F64ConvertI32S(a) => Op::F64ConvertI32S(map_v(a)),
                Op::F64ConvertI32U(a) => Op::F64ConvertI32U(map_v(a)),
                Op::F64ConvertI64S(a) => Op::F64ConvertI64S(map_v(a)),
                Op::F64ConvertI64U(a) => Op::F64ConvertI64U(map_v(a)),
                Op::I32ReinterpretF32(a) => Op::I32ReinterpretF32(map_v(a)),
                Op::I64ReinterpretF64(a) => Op::I64ReinterpretF64(map_v(a)),
                Op::F32ReinterpretI32(a) => Op::F32ReinterpretI32(map_v(a)),
                Op::F64ReinterpretI64(a) => Op::F64ReinterpretI64(map_v(a)),
                Op::Trunc(t, a) => Op::Trunc(t, map_v(a)),
                Op::Select { cond, a, b } => Op::Select {
                    cond: map_v(cond),
                    a: map_v(a),
                    b: map_v(b),
                },
                Op::CmpSelect { kind, a, b, tv, fv } => Op::CmpSelect {
                    kind,
                    a: map_v(a),
                    b: map_v(b),
                    tv: map_v(tv),
                    fv: map_v(fv),
                },
                Op::GlobalGet(g) => Op::GlobalGet(g),
                Op::GlobalSet(g, a) => Op::GlobalSet(g, map_v(a)),
                Op::TableGet { table, idx } => Op::TableGet {
                    table,
                    idx: map_v(idx),
                },
                Op::TableSet { table, idx, val } => Op::TableSet {
                    table,
                    idx: map_v(idx),
                    val: map_v(val),
                },
                Op::BoundsCheck { addr, end_off } => Op::BoundsCheck {
                    addr: map_v(addr),
                    end_off,
                },
                Op::Load {
                    kind,
                    addr,
                    offset,
                    checked,
                } => Op::Load {
                    kind,
                    addr: map_v(addr),
                    offset,
                    checked,
                },
                Op::Store {
                    kind,
                    addr,
                    val,
                    offset,
                    checked,
                } => Op::Store {
                    kind,
                    addr: map_v(addr),
                    val: map_v(val),
                    offset,
                    checked,
                },
                Op::MemSize => Op::MemSize,
                Op::MemGrow(a) => Op::MemGrow(map_v(a)),
                Op::MemCopy { dst, src, len } => Op::MemCopy {
                    dst: map_v(dst),
                    src: map_v(src),
                    len: map_v(len),
                },
                Op::MemFill { dst, val, len } => Op::MemFill {
                    dst: map_v(dst),
                    val: map_v(val),
                    len: map_v(len),
                },
                Op::MemCopyConst {
                    dst,
                    src,
                    len,
                    dst_checked,
                    src_checked,
                } => Op::MemCopyConst {
                    dst: map_v(dst),
                    src: map_v(src),
                    len,
                    dst_checked,
                    src_checked,
                },
                Op::MemFillConst {
                    dst,
                    val,
                    len,
                    checked,
                } => Op::MemFillConst {
                    dst: map_v(dst),
                    val: map_v(val),
                    len,
                    checked,
                },
                Op::V128Splat { lane_bytes, val } => Op::V128Splat {
                    lane_bytes,
                    val: map_v(val),
                },
                Op::I64x2ExtractLane { v, lane } => Op::I64x2ExtractLane { v: map_v(v), lane },
                Op::I64x2ReplaceLane { v, x, lane } => Op::I64x2ReplaceLane {
                    v: map_v(v),
                    x: map_v(x),
                    lane,
                },
                Op::Call { func, args, rets } => Op::Call {
                    func,
                    args: map_range(&mut caller.val_pool, &callee.val_pool, args)?,
                    rets: map_range(&mut caller.val_pool, &callee.val_pool, rets)?,
                },
                Op::CallIndirect {
                    ci_id,
                    idx,
                    args,
                    rets,
                } => Op::CallIndirect {
                    ci_id,
                    idx: map_v(idx),
                    args: map_range(&mut caller.val_pool, &callee.val_pool, args)?,
                    rets: map_range(&mut caller.val_pool, &callee.val_pool, rets)?,
                },
                Op::Trap(code) => Op::Trap(code),
            };
            insts.push(Inst { res, op });
        }
        let term = match cb.term {
            Term::Jump { target, args } => Term::Jump {
                target: map_b(target),
                args: map_range(&mut caller.val_pool, &callee.val_pool, args)?,
            },
            Term::BranchIf {
                cond,
                then_bb,
                then_args,
                else_bb,
                else_args,
            } => Term::BranchIf {
                cond: map_v(cond),
                then_bb: map_b(then_bb),
                then_args: map_range(&mut caller.val_pool, &callee.val_pool, then_args)?,
                else_bb: map_b(else_bb),
                else_args: map_range(&mut caller.val_pool, &callee.val_pool, else_args)?,
            },
            Term::CmpBranch {
                kind,
                a,
                b,
                then_bb,
                then_args,
                else_bb,
                else_args,
            } => Term::CmpBranch {
                kind,
                a: map_v(a),
                b: map_v(b),
                then_bb: map_b(then_bb),
                then_args: map_range(&mut caller.val_pool, &callee.val_pool, then_args)?,
                else_bb: map_b(else_bb),
                else_args: map_range(&mut caller.val_pool, &callee.val_pool, else_args)?,
            },
            Term::BrTable { idx, table_idx } => Term::BrTable {
                idx: map_v(idx),
                table_idx: base_bt + table_idx,
            },
            Term::Return { vals } => Term::Jump {
                target: cont_bb,
                args: map_range(&mut caller.val_pool, &callee.val_pool, vals)?,
            },
            Term::Unreachable => Term::Unreachable,
        };
        caller.blocks.push(Block {
            params,
            insts,
            term,
            is_loop_header: false,
            preds: 0,
        });
    }

    caller.blocks[call_bb].term = Term::Jump {
        target: BlockId(base_bb),
        args: call_args,
    };
    Ok(())
}

/// Execution mode for `inline_small_helpers_subset`, distinguishing cold non-cache builds,
/// cold builds capturing optional incremental cache metadata (where core inlining must run
/// exactly once and never retry on partially mutated IR), and speculative warm builds on
/// disposable IR clones (where any OOM cleanly aborts speculation via `Ok(None)`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InlineMode {
    ColdNoCache,
    ColdCaptureCache,
    WarmSpeculative,
}

/// Per-function tracking produced by `inline_small_helpers_subset` for incremental caching.
pub struct InlineTrack {
    pub inlined_callees: Vec<Vec<u32>>,
    pub added_insts: Vec<u32>,
    pub was_inlinable: Vec<bool>,
    pub hit_module_cap: bool,
    /// `false` if optional cache-tracking metadata could not be allocated under memory pressure.
    pub tracking_complete: bool,
}

/// Budgeted bottom-up helper inlining across the module (or a dirty/cone subset when `irs[bi]`
/// is `Some` only for active functions), supporting general, loop-call-site, and single-call-site
/// thresholds and returning exact per-function inline tracking.
/// - `InlineMode::WarmSpeculative`: any OOM returns `Ok(None)` so the caller discards its
///   disposable `work_irs` and retries cold.
/// - `InlineMode::ColdCaptureCache`: tracking-only OOM disables tracking (`tracking_complete = false`)
///   and completes the exact single deterministic inlining pass without mutating and retrying;
///   core inlining OOM returns `Err(...)`.
/// - `InlineMode::ColdNoCache`: no tracking allocations; core inlining OOM returns `Err(...)`.
#[allow(clippy::too_many_arguments)]
pub fn inline_small_helpers_subset(
    irs: &mut [Option<IrFunc>],
    in_cycle: &[bool],
    topo_bottom_up: &[usize],
    total_call_counts: &[u32],
    module: &X64Module,
    initial_added_insts: usize,
    mode: InlineMode,
    budget: Option<&CompileBudget>,
) -> Res<Option<InlineTrack>> {
    let n_imports = module.imports.len();
    let n_defs = irs.len();
    let warm = mode == InlineMode::WarmSpeculative;
    let mut track_ok = mode != InlineMode::ColdNoCache;
    let mut inlined_callees: Vec<Vec<u32>> = Vec::new();
    let mut added_insts: Vec<u32> = Vec::new();
    let mut was_inlinable: Vec<bool> = Vec::new();
    if track_ok {
        let init_ok = (|| -> Option<()> {
            inlined_callees.try_reserve_exact(n_defs).ok()?;
            for _ in 0..n_defs {
                inlined_callees.push(Vec::new());
            }
            added_insts.try_reserve_exact(n_defs).ok()?;
            added_insts.resize(n_defs, 0);
            was_inlinable.try_reserve_exact(n_defs).ok()?;
            was_inlinable.resize(n_defs, false);
            Some(())
        })()
        .is_some();
        if !init_ok {
            if warm {
                return Ok(None);
            }
            // Cold capture: drop optional tracking metadata and continue the single exact inlining pass.
            track_ok = false;
            inlined_callees = Vec::new();
            added_insts = Vec::new();
            was_inlinable = Vec::new();
        }
    }
    if n_defs <= 1 {
        return Ok(Some(InlineTrack {
            inlined_callees,
            added_insts,
            was_inlinable,
            hit_module_cap: false,
            tracking_complete: track_ok,
        }));
    }
    let Some(ext_ref) = try_compute_externally_referenced(module, n_defs) else {
        if warm {
            return Ok(None);
        }
        return Err("x64: out of memory allocating externally-referenced table".into());
    };
    let mut inst_counts: Vec<usize> = {
        let mut counts = Vec::new();
        if counts.try_reserve_exact(n_defs).is_err() {
            if warm {
                return Ok(None);
            }
            return Err("x64: out of memory allocating inline instruction counts".into());
        }
        for o in irs.iter() {
            counts.push(o.as_ref().map_or(0, count_insts));
        }
        counts
    };
    if track_ok {
        for (bi, o) in irs.iter().enumerate() {
            if let Some(ir) = o {
                // Purely structural inlinability (independent of `in_cycle`) so incremental
                // builds always invalidate callers when an edit breaks an SCC cycle.
                was_inlinable[bi] = is_inlinable_ir(ir);
            }
        }
    }
    let mut total_added_insts = initial_added_insts;
    let mut hit_module_cap = total_added_insts >= INLINE_MAX_MODULE_ADDED_INSTS;

    // Bottom-up topological order naturally handles chained small helpers (`a -> b -> c`) in one pass!
    let mut inline_steps = 0usize;
    for (step, &caller_bi) in topo_bottom_up.iter().enumerate() {
        if let Some(bg) = budget
            && (step & 63) == 0
        {
            bg.check_deadline()?;
        }
        if irs[caller_bi].is_none() {
            continue;
        }
        if total_added_insts >= INLINE_MAX_MODULE_ADDED_INSTS {
            hit_module_cap = true;
            break;
        }
        let mut caller_insts = inst_counts[caller_bi];
        if caller_insts >= INLINE_MAX_CALLER_INSTS {
            continue;
        }
        let Some(mut caller_in_loop) = try_compute_in_loop_blocks(irs[caller_bi].as_ref().unwrap())
        else {
            if warm {
                return Ok(None);
            }
            return Err("x64: out of memory computing loop blocks during inlining".into());
        };
        let mut caller_added = 0usize;
        let mut did_inline = false;

        // Scan blocks dynamically (splitting pushes new blocks at the end; cont_bb inherits loop status).
        let mut bi = 0usize;
        while bi < irs[caller_bi].as_ref().unwrap().blocks.len() {
            let block_in_loop = caller_in_loop.get(bi).copied().unwrap_or(false);
            let mut ii = 0usize;
            while ii < irs[caller_bi].as_ref().unwrap().blocks[bi].insts.len() {
                inline_steps += 1;
                if let Some(bg) = budget
                    && (inline_steps & 4095) == 0
                {
                    bg.check_deadline()?;
                }
                if total_added_insts >= INLINE_MAX_MODULE_ADDED_INSTS {
                    hit_module_cap = true;
                    break;
                }
                if caller_added >= INLINE_MAX_ADDED_PER_CALLER
                    || caller_insts >= INLINE_MAX_CALLER_INSTS
                {
                    break;
                }
                let Op::Call { func, args, rets } =
                    irs[caller_bi].as_ref().unwrap().blocks[bi].insts[ii].op
                else {
                    ii += 1;
                    continue;
                };
                let f = func as usize;
                if f < n_imports {
                    ii += 1;
                    continue;
                }
                let callee_bi = f - n_imports;
                if callee_bi == caller_bi || callee_bi >= n_defs || in_cycle[callee_bi] {
                    ii += 1;
                    continue;
                }
                let Some(callee_ir) = irs[callee_bi].as_ref() else {
                    ii += 1;
                    continue;
                };
                let c_insts = inst_counts[callee_bi];
                let c_blocks = callee_ir.blocks.len();
                let c_vals = callee_ir.val_tys.len();
                let is_single_site = !ext_ref[callee_bi]
                    && total_call_counts.get(callee_bi).copied().unwrap_or(0) == 1;
                let (max_insts, max_blocks, max_vals) = if is_single_site {
                    (
                        INLINE_MAX_SINGLE_CALLEE_INSTS,
                        INLINE_MAX_SINGLE_CALLEE_BLOCKS,
                        INLINE_MAX_SINGLE_CALLEE_VALS,
                    )
                } else if block_in_loop {
                    (
                        INLINE_MAX_LOOP_CALLEE_INSTS,
                        INLINE_MAX_LOOP_CALLEE_BLOCKS,
                        INLINE_MAX_LOOP_CALLEE_VALS,
                    )
                } else {
                    (
                        INLINE_MAX_CALLEE_INSTS,
                        INLINE_MAX_CALLEE_BLOCKS,
                        INLINE_MAX_CALLEE_VALS,
                    )
                };
                let caller_ir = irs[caller_bi].as_ref().unwrap();
                if !callee_ir.br_tables.is_empty()
                    || c_insts > max_insts
                    || c_blocks > max_blocks
                    || c_vals > max_vals
                    || caller_ir.blocks.len() + c_blocks + 1 > INLINE_MAX_CALLER_BLOCKS
                    || caller_ir.val_tys.len() + c_vals > INLINE_MAX_CALLER_VALS
                {
                    ii += 1;
                    continue;
                }
                // Reserve `caller_in_loop` capacity BEFORE mutating the caller IR so an OOM
                // cannot leave a caller partially inlined.
                let needed_blocks = caller_ir.blocks.len() + c_blocks + 1;
                let extra_loop_slots = needed_blocks.saturating_sub(caller_in_loop.len());
                if caller_in_loop.try_reserve(extra_loop_slots).is_err() {
                    if warm {
                        return Ok(None);
                    }
                    return Err("x64: out of memory extending loop blocks during inlining".into());
                }
                // Borrow disjoint `caller` (&mut) and `callee` (&) directly via `split_at_mut` (zero cloning!).
                let ok = if caller_bi < callee_bi {
                    let (lo, hi) = irs.split_at_mut(callee_bi);
                    clone_callee_into_caller(
                        lo[caller_bi].as_mut().unwrap(),
                        hi[0].as_ref().unwrap(),
                        bi,
                        ii,
                        args,
                        rets,
                    )
                    .is_ok()
                } else {
                    let (lo, hi) = irs.split_at_mut(caller_bi);
                    clone_callee_into_caller(
                        hi[0].as_mut().unwrap(),
                        lo[callee_bi].as_ref().unwrap(),
                        bi,
                        ii,
                        args,
                        rets,
                    )
                    .is_ok()
                };
                if ok {
                    let new_blocks_len = irs[caller_bi].as_ref().unwrap().blocks.len();
                    caller_in_loop.resize(new_blocks_len, block_in_loop);
                    let cost = c_insts.max(1);
                    caller_insts += cost;
                    caller_added += cost;
                    total_added_insts += cost;
                    did_inline = true;
                    if track_ok {
                        let ok_track = if caller_bi < callee_bi {
                            let (lo, hi) = inlined_callees.split_at_mut(callee_bi);
                            let dst = &mut lo[caller_bi];
                            let src = &hi[0];
                            if dst.try_reserve(1 + src.len()).is_ok() {
                                dst.push(callee_bi as u32);
                                dst.extend_from_slice(src);
                                true
                            } else {
                                false
                            }
                        } else {
                            let (lo, hi) = inlined_callees.split_at_mut(caller_bi);
                            let dst = &mut hi[0];
                            let src = &lo[callee_bi];
                            if dst.try_reserve(1 + src.len()).is_ok() {
                                dst.push(callee_bi as u32);
                                dst.extend_from_slice(src);
                                true
                            } else {
                                false
                            }
                        };
                        if !ok_track {
                            track_ok = false;
                            inlined_callees.clear();
                        }
                    }
                    // `bi` now ends with `Jump(callee_entry)`; tail moved to `cont_bb` (visited later in `while bi`).
                    break;
                }
                ii += 1;
            }
            bi += 1;
        }
        if did_inline {
            inst_counts[caller_bi] = caller_insts;
            if track_ok {
                added_insts[caller_bi] = caller_added as u32;
                inlined_callees[caller_bi].sort_unstable();
                inlined_callees[caller_bi].dedup();
            }
            // Only compact CFG sequentially if this caller is itself small enough to be inlined upstream;
            // all larger callers compact in parallel inside `optimize_func_stage2`!
            if caller_insts <= INLINE_MAX_SINGLE_CALLEE_INSTS
                && irs[caller_bi].as_ref().unwrap().blocks.len() > INLINE_MAX_SINGLE_CALLEE_BLOCKS
            {
                rebuild_cfg_and_prune(irs[caller_bi].as_mut().unwrap(), budget)?;
            }
            if track_ok && is_inlinable_ir(irs[caller_bi].as_ref().unwrap()) {
                was_inlinable[caller_bi] = true;
            }
        }
    }
    Ok(Some(InlineTrack {
        inlined_callees,
        added_insts,
        was_inlinable,
        hit_module_cap,
        tracking_complete: track_ok,
    }))
}

/// Budgeted bottom-up small-helper inlining across the module (keeping originals intact for
/// indirect/exported callers). Runs first so all downstream passes optimize inlined bodies.
pub fn inline_small_helpers(
    irs: &mut [IrFunc],
    module: &X64Module,
    budget: Option<&CompileBudget>,
) -> Res<()> {
    let n_imports = module.imports.len();
    let n_defs = irs.len();
    if n_defs <= 1 {
        return Ok(());
    }
    let pre: Vec<Vec<(u32, u32)>> = irs
        .iter()
        .map(|ir| collect_pre_inline_callees(ir, n_imports, n_defs))
        .collect();
    let mut total_call_counts = vec![0u32; n_defs];
    let mut edges: Vec<Vec<usize>> = Vec::with_capacity(n_defs);
    for cs in &pre {
        edges.push(cs.iter().map(|&(c, _)| c as usize).collect());
        for &(c, cnt) in cs {
            total_call_counts[c as usize] = total_call_counts[c as usize].saturating_add(cnt);
        }
    }
    let (in_cycle, topo_bottom_up) = call_graph_sccs_from_edges(&edges);
    let mut opt_irs: Vec<Option<IrFunc>> = irs
        .iter_mut()
        .map(|ir| {
            Some(std::mem::replace(
                ir,
                IrFunc {
                    func_index: 0,
                    params: Vec::new(),
                    result_tys: Vec::new(),
                    val_tys: Vec::new(),
                    blocks: Vec::new(),
                    val_pool: Vec::new(),
                    v128_consts: Vec::new(),
                    br_tables: Vec::new(),
                    needs_stack_check: true,
                    elided_callee_stack_bytes: 0,
                },
            ))
        })
        .collect();
    let res = inline_small_helpers_subset(
        &mut opt_irs,
        &in_cycle,
        &topo_bottom_up,
        &total_call_counts,
        module,
        0,
        InlineMode::ColdNoCache,
        budget,
    );
    for (dst, src) in irs.iter_mut().zip(opt_irs) {
        *dst = src.unwrap();
    }
    res.map(|_| ())
}

// ============================================================================
// 3. Interprocedural Effect Analysis
// ============================================================================

/// Non-transitive own side-effect summary of one defined function (post-inlining, pre-Stage-2 opt).
#[derive(Clone)]
pub struct FnOwnEffects {
    pub writes_mem: bool,
    pub grows_mem: bool,
    pub writes_all_globals: bool,
    pub written_globals: Vec<u64>,
    pub writes_all_tables: bool,
    pub written_tables: u64,
    /// Unique sorted direct callee Wasm function indices (imports + defined).
    pub direct_callees: Vec<u32>,
}

pub fn try_compute_fn_own_effects(
    ir: &IrFunc,
    module: &X64Module,
    n_total: usize,
) -> Option<FnOwnEffects> {
    let global_words = module.globals.len().div_ceil(64);
    let fixed_mem = module.fixed32.is_some();
    let mut writes_mem = false;
    let mut grows_mem = false;
    let mut writes_all_globals = false;
    let mut written_globals = Vec::new();
    written_globals.try_reserve_exact(global_words).ok()?;
    written_globals.resize(global_words, 0u64);
    let mut writes_all_tables = false;
    let mut written_tables = 0u64;
    let mut direct_callees: Vec<u32> = Vec::new();
    for bb in &ir.blocks {
        for inst in &bb.insts {
            match inst.op {
                Op::Store { .. }
                | Op::MemCopy { .. }
                | Op::MemFill { .. }
                | Op::MemCopyConst { .. }
                | Op::MemFillConst { .. } => writes_mem = true,
                Op::MemGrow(_) => {
                    writes_mem = true;
                    grows_mem = !fixed_mem;
                }
                Op::GlobalSet(g, _) => {
                    let gi = g as usize;
                    if (gi >> 6) < global_words {
                        written_globals[gi >> 6] |= 1u64 << (gi & 63);
                    }
                }
                Op::TableSet { table, .. } => {
                    if table < 64 {
                        written_tables |= 1u64 << table;
                    } else {
                        writes_all_tables = true;
                    }
                }
                Op::CallIndirect { .. } => {
                    writes_mem = true;
                    grows_mem = !fixed_mem;
                    writes_all_globals = true;
                    writes_all_tables = true;
                }
                Op::Call { func, .. } if (func as usize) < n_total => {
                    if direct_callees.len() == direct_callees.capacity() {
                        direct_callees.try_reserve(1).ok()?;
                    }
                    direct_callees.push(func);
                }
                _ => {}
            }
        }
    }
    direct_callees.sort_unstable();
    direct_callees.dedup();
    Some(FnOwnEffects {
        writes_mem,
        grows_mem,
        writes_all_globals,
        written_globals,
        writes_all_tables,
        written_tables,
        direct_callees,
    })
}

pub struct ModuleEffects {
    pub writes_mem: Vec<bool>,
    pub grows_mem: Vec<bool>,
    pub writes_all_globals: Vec<bool>,
    pub written_globals: Vec<Vec<u64>>,
    pub global_words: usize,
    pub writes_all_tables: Vec<bool>,
    pub written_tables: Vec<u64>,
}

/// Fallibly computes transitive `ModuleEffects` directly from per-function `FnOwnEffects`
/// summaries, returning `Ok(None)` on allocation failure so optional cache work can fall back cleanly.
pub fn try_compute_module_effects_from_summaries(
    summaries: &[FnOwnEffects],
    module: &X64Module,
    budget: Option<&CompileBudget>,
) -> Res<Option<ModuleEffects>> {
    if let Some(bg) = budget {
        bg.check_deadline()?;
    }
    let n_imports = module.imports.len();
    let n_total = n_imports + summaries.len();
    let global_words = module.globals.len().div_ceil(64);
    let fixed_mem = module.fixed32.is_some();

    let alloc_res = (|| -> Option<_> {
        let mut writes_mem = Vec::new();
        writes_mem.try_reserve_exact(n_total).ok()?;
        writes_mem.resize(n_total, false);
        let mut grows_mem = Vec::new();
        grows_mem.try_reserve_exact(n_total).ok()?;
        grows_mem.resize(n_total, false);
        let mut writes_all_globals = Vec::new();
        writes_all_globals.try_reserve_exact(n_total).ok()?;
        writes_all_globals.resize(n_total, false);
        let mut written_globals = Vec::new();
        written_globals.try_reserve_exact(n_total).ok()?;
        for _ in 0..n_total {
            let mut row = Vec::new();
            row.try_reserve_exact(global_words).ok()?;
            row.resize(global_words, 0u64);
            written_globals.push(row);
        }
        let mut writes_all_tables = Vec::new();
        writes_all_tables.try_reserve_exact(n_total).ok()?;
        writes_all_tables.resize(n_total, false);
        let mut written_tables = Vec::new();
        written_tables.try_reserve_exact(n_total).ok()?;
        written_tables.resize(n_total, 0u64);
        Some((
            writes_mem,
            grows_mem,
            writes_all_globals,
            written_globals,
            writes_all_tables,
            written_tables,
        ))
    })();
    let Some((
        mut writes_mem,
        mut grows_mem,
        mut writes_all_globals,
        mut written_globals,
        mut writes_all_tables,
        mut written_tables,
    )) = alloc_res
    else {
        return Ok(None);
    };

    for fi in 0..n_imports {
        writes_mem[fi] = true;
        grows_mem[fi] = !fixed_mem;
        writes_all_globals[fi] = true;
        writes_all_tables[fi] = true;
    }
    for (bi, s) in summaries.iter().enumerate() {
        let fi = n_imports + bi;
        writes_mem[fi] = s.writes_mem;
        grows_mem[fi] = s.grows_mem;
        writes_all_globals[fi] = s.writes_all_globals;
        for (w, dst) in written_globals[fi]
            .iter_mut()
            .enumerate()
            .take(global_words)
        {
            *dst = s.written_globals.get(w).copied().unwrap_or(0);
        }
        writes_all_tables[fi] = s.writes_all_tables;
        written_tables[fi] = s.written_tables;
    }

    let mut changed = true;
    let mut fix_steps = 0usize;
    while changed {
        changed = false;
        for (bi, s) in summaries.iter().enumerate() {
            let fi = n_imports + bi;
            for &c_u32 in &s.direct_callees {
                let c = c_u32 as usize;
                fix_steps = fix_steps.saturating_add(global_words.max(1));
                if let Some(bg) = budget
                    && fix_steps >= 4096
                {
                    fix_steps = 0;
                    bg.check_deadline()?;
                }
                if writes_mem[c] && !writes_mem[fi] {
                    writes_mem[fi] = true;
                    changed = true;
                }
                if grows_mem[c] && !grows_mem[fi] {
                    grows_mem[fi] = true;
                    changed = true;
                }
                if writes_all_globals[c] && !writes_all_globals[fi] {
                    writes_all_globals[fi] = true;
                    changed = true;
                } else if !writes_all_globals[fi] && c != fi && global_words > 0 {
                    let (dst_slice, src_slice) = if fi < c {
                        let (lo, hi) = written_globals.split_at_mut(c);
                        (&mut lo[fi], &hi[0])
                    } else {
                        let (lo, hi) = written_globals.split_at_mut(fi);
                        (&mut hi[0], &lo[c])
                    };
                    for (dst, &src) in dst_slice.iter_mut().zip(src_slice.iter()) {
                        let nv = *dst | src;
                        if nv != *dst {
                            *dst = nv;
                            changed = true;
                        }
                    }
                }
                if writes_all_tables[c] && !writes_all_tables[fi] {
                    writes_all_tables[fi] = true;
                    changed = true;
                } else if !writes_all_tables[fi] {
                    let old = written_tables[fi];
                    let nv = old | written_tables[c];
                    if nv != old {
                        written_tables[fi] = nv;
                        changed = true;
                    }
                }
            }
        }
    }

    Ok(Some(ModuleEffects {
        writes_mem,
        grows_mem,
        writes_all_globals,
        written_globals,
        global_words,
        writes_all_tables,
        written_tables,
    }))
}

/// Computes transitive `ModuleEffects` directly from per-function `FnOwnEffects` summaries.
pub fn compute_module_effects_from_summaries(
    summaries: &[FnOwnEffects],
    module: &X64Module,
    budget: Option<&CompileBudget>,
) -> Res<ModuleEffects> {
    try_compute_module_effects_from_summaries(summaries, module, budget)?
        .ok_or_else(|| "x64: out of memory allocating module effects".to_string())
}

pub fn compute_module_effects(
    irs: &[IrFunc],
    module: &X64Module,
    budget: Option<&CompileBudget>,
) -> Res<ModuleEffects> {
    let n_total = module.imports.len() + irs.len();
    let mut summaries = Vec::new();
    summaries
        .try_reserve_exact(irs.len())
        .map_err(|_| "x64: out of memory allocating function effect summaries".to_string())?;
    for ir in irs {
        let s = try_compute_fn_own_effects(ir, module, n_total)
            .ok_or_else(|| "x64: out of memory allocating function effect summary".to_string())?;
        summaries.push(s);
    }
    compute_module_effects_from_summaries(&summaries, module, budget)
}

// ============================================================================
// 4. Conservative LICM
// ============================================================================

fn is_pure_licm_op(op: &Op) -> bool {
    match *op {
        // Skip cheap zero floats (`xorps`, 3 bytes) to avoid lengthening live ranges across loops.
        Op::ConstF32(b) => b != 0,
        Op::ConstF64(b) => b != 0,
        Op::ConstV128(_)
        | Op::I32Eqz(_)
        | Op::I64Eqz(_)
        | Op::I32Clz(_)
        | Op::I32Ctz(_)
        | Op::I32Popcnt(_)
        | Op::I64Clz(_)
        | Op::I64Ctz(_)
        | Op::I64Popcnt(_)
        | Op::I32WrapI64(_)
        | Op::I64ExtendI32S(_)
        | Op::I64ExtendI32U(_)
        | Op::I32Extend8S(_)
        | Op::I32Extend16S(_)
        | Op::I64Extend8S(_)
        | Op::I64Extend16S(_)
        | Op::I64Extend32S(_)
        | Op::I32Cmp(_, _, _)
        | Op::I32CmpImm(_, _, _)
        | Op::I64Cmp(_, _, _)
        | Op::I64CmpImm(_, _, _)
        | Op::F32Cmp(_, _, _)
        | Op::F64Cmp(_, _, _)
        | Op::F32Un(_, _)
        | Op::F64Un(_, _)
        | Op::F32Bin(_, _, _)
        | Op::F64Bin(_, _, _)
        | Op::F32DemoteF64(_)
        | Op::F64PromoteF32(_)
        | Op::F32ConvertI32S(_)
        | Op::F32ConvertI32U(_)
        | Op::F32ConvertI64S(_)
        | Op::F32ConvertI64U(_)
        | Op::F64ConvertI32S(_)
        | Op::F64ConvertI32U(_)
        | Op::F64ConvertI64S(_)
        | Op::F64ConvertI64U(_)
        | Op::I32ReinterpretF32(_)
        | Op::I64ReinterpretF64(_)
        | Op::F32ReinterpretI32(_)
        | Op::F64ReinterpretI64(_)
        | Op::Select { .. }
        | Op::CmpSelect { .. }
        | Op::V128Splat { .. }
        | Op::I64x2ExtractLane { .. }
        | Op::I64x2ReplaceLane { .. } => true,
        Op::I32Bin(bop, _, _) | Op::I64Bin(bop, _, _) => !matches!(
            bop,
            IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU
        ),
        Op::I32BinImm(bop, _, imm) | Op::I64BinImm(bop, _, imm) => {
            !matches!(
                bop,
                IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU
            ) || (imm != 0 && !(bop == IntBinOp::DivS && imm == -1))
        }
        Op::Trunc(t, _) => !t.can_trap(),
        // NEVER hoist Load, Store, BoundsCheck, TableGet, TableSet, Call, CallIndirect, MemGrow, MemCopy*, MemFill*, Trap!
        _ => false,
    }
}

/// Conservative LICM: hoists strictly non-trapping loop-invariant pure ops, uninvalidated
/// `GlobalGet(g)`, uninvalidated `MemSize`, and (via guarded preheaders when a header check
/// proves `divisor != 0`) loop-invariant unsigned `DivU`/`RemU` out of natural loops.
/// Returns `true` if a new guarded preheader block was created (requiring CFG/dom rebuild).
pub fn run_licm(
    ir: &mut IrFunc,
    dom: &CfgDom,
    module: &X64Module,
    effects: &ModuleEffects,
    budget: Option<&CompileBudget>,
) -> Res<bool> {
    let n = ir.blocks.len();
    if n <= 1 {
        return Ok(false);
    }

    // Group natural loops by header (`header -> union of body blocks for all backedges latch -> header`).
    let mut loops_by_hdr: HashMap<u32, HashSet<u32>> = HashMap::new();
    for &b in &dom.rpo {
        for &s in &dom.succs[b as usize] {
            if dom.dominates(s, b) {
                let body = loops_by_hdr.entry(s).or_default();
                body.insert(s);
                if body.insert(b) {
                    let mut wl = vec![b];
                    while let Some(cur) = wl.pop() {
                        for &p in &dom.preds[cur as usize] {
                            if body.insert(p) {
                                wl.push(p);
                            }
                        }
                    }
                }
            }
        }
    }
    if loops_by_hdr.is_empty() {
        return Ok(false);
    }

    // Sort loops innermost-first (smallest body first), skip header == 0 (entry block has no preheader).
    let mut loops: Vec<(u32, Vec<u32>)> = loops_by_hdr
        .into_iter()
        .filter(|&(h, _)| h != 0)
        .map(|(h, set)| {
            let mut v: Vec<u32> = set.into_iter().collect();
            v.sort_by_key(|&b| dom.rpo_pos[b as usize]);
            (h, v)
        })
        .collect();
    // Sort loops innermost-first with a deterministic header-id tie-breaker so `HashMap`
    // iteration order can never alter guarded-LICM selection or block/value numbering.
    loops.sort_by_key(|(h, body)| (body.len(), *h));

    // Map every Val -> defining block.
    let mut def_block = vec![u32::MAX; ir.val_tys.len()];
    for (bi, bb) in ir.blocks.iter().enumerate() {
        for &p in &bb.params {
            def_block[p.0 as usize] = bi as u32;
        }
        for inst in &bb.insts {
            if let Some(r) = inst.res {
                def_block[r.0 as usize] = bi as u32;
            }
            match inst.op {
                Op::Call { rets, .. } | Op::CallIndirect { rets, .. } => {
                    for &rv in ir.pool_slice(rets) {
                        def_block[rv.0 as usize] = bi as u32;
                    }
                }
                _ => {}
            }
        }
    }

    let fixed_mem = module.fixed32.is_some();
    let mut guarded_candidates: Vec<(u32, u32, Vec<u32>)> = Vec::new();
    for (li, (hdr, body)) in loops.into_iter().enumerate() {
        if let Some(bg) = budget
            && (li & 15) == 0
        {
            bg.check_deadline()?;
        }
        let body_set: HashSet<u32> = body.iter().copied().collect();
        // Find all non-loop predecessors of `hdr`. Only hoist cleanly into a single Jump preheader
        // (avoids allocating extra blocks on complex multi-entry loops while covering 99%+ of Wasm loops,
        // since Wasm `loop` entry always has a single dominating fallthrough Jump predecessor!).
        let outside_preds: Vec<u32> = dom.preds[hdr as usize]
            .iter()
            .copied()
            .filter(|p| !body_set.contains(p))
            .collect();
        if outside_preds.len() != 1 {
            continue;
        }
        let pre = outside_preds[0];
        if !matches!(ir.blocks[pre as usize].term, Term::Jump { target, .. } if target.0 == hdr) {
            continue;
        }
        // Check that `pre` dominates `hdr` (guaranteed when `outside_preds.len() == 1`!).
        if !dom.dominates(pre, hdr) {
            continue;
        }

        // Compute loop-body effect summary.
        let mut loop_grows_mem = false;
        let mut loop_writes_all_globals = false;
        let mut loop_written_globals: HashSet<u32> = HashSet::new();
        for &b in &body {
            for inst in &ir.blocks[b as usize].insts {
                match inst.op {
                    Op::MemGrow(_) => {
                        if !fixed_mem {
                            loop_grows_mem = true;
                        }
                    }
                    Op::GlobalSet(g, _) => {
                        loop_written_globals.insert(g);
                    }
                    Op::CallIndirect { .. } => {
                        if !fixed_mem {
                            loop_grows_mem = true;
                        }
                        loop_writes_all_globals = true;
                    }
                    Op::Call { func, .. } => {
                        let f = func as usize;
                        if effects.grows_mem[f] {
                            loop_grows_mem = true;
                        }
                        if effects.writes_all_globals[f] {
                            loop_writes_all_globals = true;
                        } else {
                            for w in 0..effects.global_words {
                                let mut bits = effects.written_globals[f][w];
                                while bits != 0 {
                                    let tz = bits.trailing_zeros();
                                    bits &= bits - 1;
                                    loop_written_globals.insert((w as u32) * 64 + tz);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Iteratively hoist loop-invariant non-trapping instructions to `pre` (capped to protect reg pressure).
        let mut hoisted_count = 0usize;
        let mut progress = true;
        while progress && hoisted_count < LICM_MAX_HOIST_PER_LOOP {
            progress = false;
            for &b in &body {
                let mut i = 0;
                while i < ir.blocks[b as usize].insts.len()
                    && hoisted_count < LICM_MAX_HOIST_PER_LOOP
                {
                    let inst = ir.blocks[b as usize].insts[i];
                    let can_hoist = match inst.op {
                        Op::GlobalGet(g) => {
                            module.pinned_global != Some(g)
                                && !loop_writes_all_globals
                                && !loop_written_globals.contains(&g)
                        }
                        Op::MemSize => !loop_grows_mem,
                        ref op => is_pure_licm_op(op),
                    };
                    if can_hoist && let Some(res) = inst.res {
                        let mut all_outside = true;
                        inst.op.for_each_use(&ir.val_pool, |u| {
                            let db = def_block[u.0 as usize];
                            if db == u32::MAX || body_set.contains(&db) || !dom.dominates(db, pre) {
                                all_outside = false;
                            }
                        });
                        if all_outside {
                            let removed = ir.blocks[b as usize].insts.remove(i);
                            ir.blocks[pre as usize].insts.push(removed);
                            def_block[res.0 as usize] = pre;
                            hoisted_count += 1;
                            progress = true;
                            continue;
                        }
                    }
                    i += 1;
                }
            }
        }
        guarded_candidates.push((hdr, pre, body));
    }

    // Second pass: guarded preheader hoisting for loop-invariant unsigned `DivU`/`RemU` when
    // an empty loop header proves `divisor != 0` on loop entry (e.g. `i <u n` implies `n >= 1`).
    // Tracks touched blocks so nested/adjacent loops never rewrite an already-rewritten block.
    let mut created_guard = false;
    let mut touched_blocks: HashSet<u32> = HashSet::new();
    let mut guard_steps: usize = 0;
    for (gi, (hdr, pre, body)) in guarded_candidates.into_iter().enumerate() {
        if let Some(bg) = budget
            && (gi & 7) == 0
        {
            bg.check_deadline()?;
        }
        if touched_blocks.contains(&hdr) || touched_blocks.contains(&pre) {
            continue;
        }
        if ir.blocks.len() + 1 > MAX_BLOCKS_PER_FN {
            break;
        }
        if !ir.blocks[hdr as usize].insts.is_empty() {
            continue;
        }
        let Term::Jump {
            target: pre_tgt,
            args: entry_args,
        } = ir.blocks[pre as usize].term
        else {
            continue;
        };
        if pre_tgt.0 != hdr {
            continue;
        }
        let Term::CmpBranch {
            kind: CmpBranchKind::I32(cop),
            a,
            b,
            then_bb,
            then_args,
            else_bb,
            else_args,
        } = ir.blocks[hdr as usize].term
        else {
            continue;
        };
        let body_set: HashSet<u32> = body.iter().copied().collect();
        let then_in = body_set.contains(&then_bb.0);
        let else_in = body_set.contains(&else_bb.0);
        if then_in == else_in {
            continue;
        }
        // Determine which operand is proven strictly > 0 (`!= 0`) when entering the loop body.
        let body_on_then = then_in;
        let (exit_bb, exit_args) = if body_on_then {
            (else_bb, else_args)
        } else {
            (then_bb, then_args)
        };
        if touched_blocks.contains(&exit_bb.0) {
            continue;
        }
        let _ = (then_args, else_args);
        let eff_cop = if body_on_then { cop } else { cop.invert_int() };
        let nz_val = match eff_cop {
            CmpOp::LtU => b, // `a <u b` => `b >= 1`
            CmpOp::GtU => a, // `a >u b` => `a >= 1`
            _ => continue,
        };
        let nz_db = def_block[nz_val.0 as usize];
        if nz_db == u32::MAX || body_set.contains(&nz_db) || !dom.dominates(nz_db, pre) {
            continue;
        }
        // Check that `a`, `b`, and every arg in `exit_args` can be mapped cleanly at `pre`
        // (either a header param -> `entry_args[k]`, or already dominating `pre`).
        let hdr_params = &ir.blocks[hdr as usize].params;
        let entry_slice = ir.pool_slice(entry_args);
        if hdr_params.len() != entry_slice.len() {
            continue;
        }
        let map_at_pre = |v: Val| -> Option<Val> {
            if let Some(pos) = hdr_params.iter().position(|&p| p == v) {
                return Some(entry_slice[pos]);
            }
            let db = def_block[v.0 as usize];
            if db != u32::MAX && !body_set.contains(&db) && dom.dominates(db, pre) {
                Some(v)
            } else {
                None
            }
        };
        let (Some(a_pre), Some(b_pre)) = (map_at_pre(a), map_at_pre(b)) else {
            continue;
        };
        let mut exit_args_pre = Vec::with_capacity(exit_args.len as usize);
        let mut exit_ok = true;
        for &ev in ir.pool_slice(exit_args) {
            if let Some(mv) = map_at_pre(ev) {
                exit_args_pre.push(mv);
            } else {
                exit_ok = false;
                break;
            }
        }
        if !exit_ok {
            continue;
        }

        // Scan body for loop-invariant unsigned `DivU`/`RemU` with divisor == `nz_val` (100% trap-free
        // inside `guarded_pre` because `nz_val >= 1` is proven by the entry guard!).
        let mut to_hoist: Vec<(u32, usize)> = Vec::new();
        for &bb_id in &body {
            guard_steps += 1;
            if let Some(bg) = budget
                && guard_steps >= 4096
            {
                guard_steps = 0;
                bg.check_deadline()?;
            }
            for (ii, inst) in ir.blocks[bb_id as usize].insts.iter().enumerate() {
                if to_hoist.len() >= 8 {
                    break;
                }
                guard_steps += 1;
                if let Some(bg) = budget
                    && guard_steps >= 4096
                {
                    guard_steps = 0;
                    bg.check_deadline()?;
                }
                if let Op::I32Bin(IntBinOp::DivU | IntBinOp::RemU, num, div) = inst.op
                    && div == nz_val
                    && inst.res.is_some()
                {
                    let ndb = def_block[num.0 as usize];
                    if ndb != u32::MAX && !body_set.contains(&ndb) && dom.dominates(ndb, pre) {
                        to_hoist.push((bb_id, ii));
                    }
                }
            }
        }
        if to_hoist.is_empty() {
            continue;
        }

        // Adding the bypass edge `pre -> exit_bb` removes `hdr` as a dominator of `exit_bb`
        // (and blocks dominated by `exit_bb`). Verify whether any block outside `body_set`
        // directly uses a value defined inside `body_set`:
        // - If none do, adding `pre -> exit_bb` preserves SSA dominance unconditionally.
        // - If only `hdr.params` escape, `exit_bb`'s sole predecessor is `hdr`, and every
        //   outside use lies in `exit_bb`'s dominator subtree, promote those escaping `hdr.params`
        //   into fresh `exit_bb` block params and rewrite the outside uses.
        // - Otherwise, skip guarded hoisting for this loop.
        let exit_sole_hdr_pred = exit_bb.0 != 0
            && (exit_bb.0 as usize) < dom.preds.len()
            && dom.preds[exit_bb.0 as usize].len() == 1
            && dom.preds[exit_bb.0 as usize][0] == hdr;
        let mut promoted_hdr_params: Vec<Val> = Vec::new();
        let mut rewrite_blocks: Vec<usize> = Vec::new();
        let mut outside_ok = true;
        for b_idx in 0..ir.blocks.len() {
            guard_steps += 1;
            if let Some(bg) = budget
                && guard_steps >= 4096
            {
                guard_steps = 0;
                bg.check_deadline()?;
            }
            if body_set.contains(&(b_idx as u32)) {
                continue;
            }
            let mut block_uses_body = false;
            let check_one = |v: Val,
                             outside_ok: &mut bool,
                             block_uses_body: &mut bool,
                             promoted: &mut Vec<Val>| {
                if !*outside_ok {
                    return;
                }
                let db = def_block[v.0 as usize];
                if db != u32::MAX && body_set.contains(&db) {
                    if !exit_sole_hdr_pred
                        || !dom.dominates(exit_bb.0, b_idx as u32)
                        || !hdr_params.contains(&v)
                        || map_at_pre(v).is_none()
                    {
                        *outside_ok = false;
                        return;
                    }
                    *block_uses_body = true;
                    if !promoted.contains(&v) {
                        promoted.push(v);
                    }
                }
            };
            for inst in &ir.blocks[b_idx].insts {
                guard_steps += 1;
                if let Some(bg) = budget
                    && guard_steps >= 4096
                {
                    guard_steps = 0;
                    bg.check_deadline()?;
                }
                inst.op.for_each_use(&ir.val_pool, |v| {
                    check_one(
                        v,
                        &mut outside_ok,
                        &mut block_uses_body,
                        &mut promoted_hdr_params,
                    );
                });
                if !outside_ok {
                    break;
                }
            }
            if outside_ok {
                ir.blocks[b_idx]
                    .term
                    .for_each_use(&ir.val_pool, &ir.br_tables, |v| {
                        check_one(
                            v,
                            &mut outside_ok,
                            &mut block_uses_body,
                            &mut promoted_hdr_params,
                        );
                    });
            }
            if !outside_ok {
                break;
            }
            if block_uses_body {
                rewrite_blocks.push(b_idx);
            }
        }
        if !outside_ok {
            continue;
        }

        // Pre-check all IR caps and push new edge argument slices BEFORE mutating any block.
        let hdr_exit_args_new: Option<ValRange> = if promoted_hdr_params.is_empty() {
            None
        } else {
            let new_len = (exit_args.len as usize) + promoted_hdr_params.len();
            if ir.val_tys.len() + promoted_hdr_params.len() > MAX_IR_VALS_PER_FN
                || ir.val_pool.len() + 2 * new_len > MAX_VAL_POOL_PER_FN
            {
                continue;
            }
            let mut hdr_vec = Vec::with_capacity(new_len);
            hdr_vec.extend_from_slice(ir.pool_slice(exit_args));
            for &hp in &promoted_hdr_params {
                hdr_vec.push(hp);
                exit_args_pre.push(map_at_pre(hp).unwrap());
            }
            let Ok(hr) = ir.push_pool(&hdr_vec) else {
                continue;
            };
            Some(hr)
        };
        let Ok(exit_args_r) = ir.push_pool(&exit_args_pre) else {
            continue;
        };

        if let Some(hr) = hdr_exit_args_new {
            let mut subst: Vec<Val> = (0..ir.val_tys.len() as u32).map(Val).collect();
            for &hp in &promoted_hdr_params {
                let ty = ir.val_tys[hp.0 as usize];
                let np = ir.push_val(ty);
                def_block.push(exit_bb.0);
                ir.blocks[exit_bb.0 as usize].params.push(np);
                subst.push(np);
                subst[hp.0 as usize] = np;
            }
            if let Term::CmpBranch {
                then_args,
                else_args,
                ..
            } = &mut ir.blocks[hdr as usize].term
            {
                if body_on_then {
                    *else_args = hr;
                } else {
                    *then_args = hr;
                }
            }
            let mut pool = std::mem::take(&mut ir.val_pool);
            for b_idx in rewrite_blocks {
                guard_steps += 1;
                if let Some(bg) = budget
                    && guard_steps >= 4096
                {
                    guard_steps = 0;
                    if let Err(e) = bg.check_deadline() {
                        ir.val_pool = pool;
                        return Err(e);
                    }
                }
                for inst in &mut ir.blocks[b_idx].insts {
                    guard_steps += 1;
                    if let Some(bg) = budget
                        && guard_steps >= 4096
                    {
                        guard_steps = 0;
                        if let Err(e) = bg.check_deadline() {
                            ir.val_pool = pool;
                            return Err(e);
                        }
                    }
                    rewrite_op_uses(&mut inst.op, &mut pool, &subst);
                }
                rewrite_term_uses(&mut ir.blocks[b_idx].term, &mut pool, &ir.br_tables, &subst);
            }
            ir.val_pool = pool;
        }

        // Remove in reverse index order per block to keep indices valid, preserving original order.
        let mut hoisted_insts: Vec<Inst> = Vec::with_capacity(to_hoist.len());
        let mut by_block: HashMap<u32, Vec<usize>> = HashMap::new();
        for &(bb_id, ii) in &to_hoist {
            by_block.entry(bb_id).or_default().push(ii);
        }
        let mut extracted: HashMap<(u32, usize), Inst> = HashMap::with_capacity(to_hoist.len());
        for (bb_id, mut idxs) in by_block {
            idxs.sort_unstable();
            for ii in idxs.into_iter().rev() {
                let removed = ir.blocks[bb_id as usize].insts.remove(ii);
                extracted.insert((bb_id, ii), removed);
            }
        }
        for key in &to_hoist {
            if let Some(inst) = extracted.remove(key) {
                hoisted_insts.push(inst);
            }
        }

        let guard_bb = BlockId(ir.blocks.len() as u32);
        for inst in &hoisted_insts {
            if let Some(res) = inst.res {
                def_block[res.0 as usize] = guard_bb.0;
            }
        }
        ir.blocks.push(Block {
            params: Vec::new(),
            insts: hoisted_insts,
            term: Term::Jump {
                target: BlockId(hdr),
                args: entry_args,
            },
            is_loop_header: false,
            preds: 1,
        });
        ir.blocks[pre as usize].term = if body_on_then {
            Term::CmpBranch {
                kind: CmpBranchKind::I32(cop),
                a: a_pre,
                b: b_pre,
                then_bb: guard_bb,
                then_args: ValRange::EMPTY,
                else_bb: exit_bb,
                else_args: exit_args_r,
            }
        } else {
            Term::CmpBranch {
                kind: CmpBranchKind::I32(cop),
                a: a_pre,
                b: b_pre,
                then_bb: exit_bb,
                then_args: exit_args_r,
                else_bb: guard_bb,
                else_args: ValRange::EMPTY,
            }
        };
        ir.blocks[exit_bb.0 as usize].preds = ir.blocks[exit_bb.0 as usize].preds.saturating_add(1);
        touched_blocks.insert(hdr);
        touched_blocks.insert(pre);
        touched_blocks.insert(exit_bb.0);
        created_guard = true;
    }

    Ok(created_guard)
}

// ============================================================================
// 5. Dominator GVN/CSE + Global/Table/Load CSE + Cross-Block BCE + Wrap-Safe Addr Folding
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum GvnKey {
    ConstF32(u32),
    ConstF64(u64),
    I32Eqz(Val),
    I64Eqz(Val),
    I32Clz(Val),
    I32Ctz(Val),
    I32Popcnt(Val),
    I64Clz(Val),
    I64Ctz(Val),
    I64Popcnt(Val),
    I32WrapI64(Val),
    I64ExtendI32S(Val),
    I64ExtendI32U(Val),
    I32Extend8S(Val),
    I32Extend16S(Val),
    I64Extend8S(Val),
    I64Extend16S(Val),
    I64Extend32S(Val),
    I32Bin(IntBinOp, Val, Val),
    I64Bin(IntBinOp, Val, Val),
    I32BinImm(IntBinOp, Val, i32),
    I64BinImm(IntBinOp, Val, i32),
    I32Cmp(CmpOp, Val, Val),
    I32CmpImm(CmpOp, Val, i32),
    I64Cmp(CmpOp, Val, Val),
    I64CmpImm(CmpOp, Val, i32),
    F32Cmp(CmpOp, Val, Val),
    F64Cmp(CmpOp, Val, Val),
    F32Un(FloatUnOp, Val),
    F64Un(FloatUnOp, Val),
    F32Bin(FloatBinOp, Val, Val),
    F64Bin(FloatBinOp, Val, Val),
    F32DemoteF64(Val),
    F64PromoteF32(Val),
    F32ConvertI32S(Val),
    F32ConvertI32U(Val),
    F32ConvertI64S(Val),
    F32ConvertI64U(Val),
    F64ConvertI32S(Val),
    F64ConvertI32U(Val),
    F64ConvertI64S(Val),
    F64ConvertI64U(Val),
    I32ReinterpretF32(Val),
    I64ReinterpretF64(Val),
    F32ReinterpretI32(Val),
    F64ReinterpretI64(Val),
    TruncSat(TruncOp, Val),
    Select(Val, Val, Val),
}

fn canonical_gvn_key(op: Op) -> Option<GvnKey> {
    let comm_int = |bop: IntBinOp, a: Val, b: Val| -> (Val, Val) {
        if matches!(
            bop,
            IntBinOp::Add | IntBinOp::Mul | IntBinOp::And | IntBinOp::Or | IntBinOp::Xor
        ) && a.0 > b.0
        {
            (b, a)
        } else {
            (a, b)
        }
    };
    let comm_cmp = |cop: CmpOp, a: Val, b: Val| -> (Val, Val) {
        if matches!(cop, CmpOp::Eq | CmpOp::Ne) && a.0 > b.0 {
            (b, a)
        } else {
            (a, b)
        }
    };
    Some(match op {
        // Deliberately DO NOT GVN integer constants or +0.0 floats across blocks: rematerializing
        // `xor r, r` / `mov r, imm` / `xorps` is vastly cheaper than holding a register or spill across loops!
        Op::ConstF32(c) if c != 0 => GvnKey::ConstF32(c),
        Op::ConstF64(c) if c != 0 => GvnKey::ConstF64(c),
        Op::I32Eqz(a) => GvnKey::I32Eqz(a),
        Op::I64Eqz(a) => GvnKey::I64Eqz(a),
        Op::I32Clz(a) => GvnKey::I32Clz(a),
        Op::I32Ctz(a) => GvnKey::I32Ctz(a),
        Op::I32Popcnt(a) => GvnKey::I32Popcnt(a),
        Op::I64Clz(a) => GvnKey::I64Clz(a),
        Op::I64Ctz(a) => GvnKey::I64Ctz(a),
        Op::I64Popcnt(a) => GvnKey::I64Popcnt(a),
        Op::I32WrapI64(a) => GvnKey::I32WrapI64(a),
        Op::I64ExtendI32S(a) => GvnKey::I64ExtendI32S(a),
        Op::I64ExtendI32U(a) => GvnKey::I64ExtendI32U(a),
        Op::I32Extend8S(a) => GvnKey::I32Extend8S(a),
        Op::I32Extend16S(a) => GvnKey::I32Extend16S(a),
        Op::I64Extend8S(a) => GvnKey::I64Extend8S(a),
        Op::I64Extend16S(a) => GvnKey::I64Extend16S(a),
        Op::I64Extend32S(a) => GvnKey::I64Extend32S(a),
        Op::I32Bin(bop, a, b)
            if !matches!(
                bop,
                IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU
            ) =>
        {
            let (x, y) = comm_int(bop, a, b);
            GvnKey::I32Bin(bop, x, y)
        }
        Op::I64Bin(bop, a, b)
            if !matches!(
                bop,
                IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU
            ) =>
        {
            let (x, y) = comm_int(bop, a, b);
            GvnKey::I64Bin(bop, x, y)
        }
        Op::I32BinImm(bop, a, imm) if !op.has_side_effect() => GvnKey::I32BinImm(bop, a, imm),
        Op::I64BinImm(bop, a, imm) if !op.has_side_effect() => GvnKey::I64BinImm(bop, a, imm),
        Op::I32Cmp(cop, a, b) => {
            let (x, y) = comm_cmp(cop, a, b);
            GvnKey::I32Cmp(cop, x, y)
        }
        Op::I32CmpImm(cop, a, imm) => GvnKey::I32CmpImm(cop, a, imm),
        Op::I64Cmp(cop, a, b) => {
            let (x, y) = comm_cmp(cop, a, b);
            GvnKey::I64Cmp(cop, x, y)
        }
        Op::I64CmpImm(cop, a, imm) => GvnKey::I64CmpImm(cop, a, imm),
        Op::F32Cmp(cop, a, b) => {
            let (x, y) = comm_cmp(cop, a, b);
            GvnKey::F32Cmp(cop, x, y)
        }
        Op::F64Cmp(cop, a, b) => {
            let (x, y) = comm_cmp(cop, a, b);
            GvnKey::F64Cmp(cop, x, y)
        }
        Op::F32Un(u, a) => GvnKey::F32Un(u, a),
        Op::F64Un(u, a) => GvnKey::F64Un(u, a),
        Op::F32Bin(bop, a, b) => GvnKey::F32Bin(bop, a, b),
        Op::F64Bin(bop, a, b) => GvnKey::F64Bin(bop, a, b),
        Op::F32DemoteF64(a) => GvnKey::F32DemoteF64(a),
        Op::F64PromoteF32(a) => GvnKey::F64PromoteF32(a),
        Op::F32ConvertI32S(a) => GvnKey::F32ConvertI32S(a),
        Op::F32ConvertI32U(a) => GvnKey::F32ConvertI32U(a),
        Op::F32ConvertI64S(a) => GvnKey::F32ConvertI64S(a),
        Op::F32ConvertI64U(a) => GvnKey::F32ConvertI64U(a),
        Op::F64ConvertI32S(a) => GvnKey::F64ConvertI32S(a),
        Op::F64ConvertI32U(a) => GvnKey::F64ConvertI32U(a),
        Op::F64ConvertI64S(a) => GvnKey::F64ConvertI64S(a),
        Op::F64ConvertI64U(a) => GvnKey::F64ConvertI64U(a),
        Op::I32ReinterpretF32(a) => GvnKey::I32ReinterpretF32(a),
        Op::I64ReinterpretF64(a) => GvnKey::I64ReinterpretF64(a),
        Op::F32ReinterpretI32(a) => GvnKey::F32ReinterpretI32(a),
        Op::F64ReinterpretI64(a) => GvnKey::F64ReinterpretI64(a),
        Op::Trunc(t, a) if !t.can_trap() => GvnKey::TruncSat(t, a),
        Op::Select { cond, a, b } => GvnKey::Select(cond, a, b),
        _ => return None,
    })
}

#[derive(Clone, Copy)]
enum MemFactKind {
    Load(LoadKind),
    Store(StoreKind),
}

#[derive(Clone, Copy)]
struct MemFact {
    addr: Val,
    const_addr: Option<u64>,
    offset: u32,
    size: u64,
    kind: MemFactKind,
    val: Val,
    val_ty: Ty,
}

#[inline]
fn ranges_disjoint(off1: u64, sz1: u64, off2: u64, sz2: u64) -> bool {
    off1.saturating_add(sz1) <= off2 || off2.saturating_add(sz2) <= off1
}

enum UndoEntry {
    Expr(GvnKey, Option<Val>),
    Const32(Val, Option<i32>),
    AddConst(Val, Option<(Val, u32)>),
    Bound(Val, Option<u64>),
    CheckedMax(Val, Option<u64>),
    Global(u32, Option<(u32, Val)>),
    MemSize(Option<(u32, Val)>),
    Table((u32, Val), Option<(u32, Val)>),
    MemPush,
    MemRemove(usize, MemFact),
    MemClear(Vec<MemFact>),
}

struct GvnState<'a> {
    module: &'a X64Module,
    effects: &'a ModuleEffects,
    fn_may_grow: bool,
    fn_writes_mem: bool,
    fn_writes_all_globals: bool,
    fn_written_globals: HashSet<u32>,
    fn_writes_all_tables: bool,
    fn_written_tables: u64,
    mem_min_bytes: u64,
    mem_max_bytes: u64,

    subst: Vec<Val>,
    any_subst: bool,
    exprs: HashMap<GvnKey, Val>,
    const32: Vec<Option<i32>>,
    add_const: Vec<Option<(Val, u32)>>,
    non_wrap_bound: Vec<Option<u64>>,
    checked_max: Vec<Option<u64>>,
    globals: Vec<Option<(u32, Val)>>,
    mem_size: Option<(u32, Val)>,
    tables: HashMap<(u32, Val), (u32, Val)>,
    mem_facts: Vec<MemFact>,
    undo: Vec<UndoEntry>,

    grow_epoch: u32,
    glob_epoch: u32,
    tbl_epoch: u32,
}

impl<'a> GvnState<'a> {
    fn fn_writes_global(&self, g: u32) -> bool {
        self.fn_writes_all_globals || self.fn_written_globals.contains(&g)
    }

    fn fn_writes_table(&self, t: u32) -> bool {
        self.fn_writes_all_tables || t >= 64 || (self.fn_written_tables & (1u64 << t)) != 0
    }

    fn insert_expr(&mut self, k: GvnKey, v: Val) {
        let prev = self.exprs.insert(k, v);
        self.undo.push(UndoEntry::Expr(k, prev));
    }

    fn insert_const32(&mut self, v: Val, c: i32) {
        let prev = self.const32[v.0 as usize].replace(c);
        self.undo.push(UndoEntry::Const32(v, prev));
    }

    fn insert_add_const(&mut self, v: Val, base: Val, k: u32) {
        let prev = self.add_const[v.0 as usize].replace((base, k));
        self.undo.push(UndoEntry::AddConst(v, prev));
    }

    fn tighten_bound(&mut self, v: Val, b: u64) {
        let slot = &mut self.non_wrap_bound[v.0 as usize];
        if slot.is_none_or(|old| b < old) {
            let prev = slot.replace(b);
            self.undo.push(UndoEntry::Bound(v, prev));
        }
    }

    fn record_checked(&mut self, addr: Val, end_off: u64) {
        // Wasm linear memory can only grow, never shrink, so `checked_max` stays valid
        // across the entire dominator subtree regardless of `MemGrow` or calls.
        let slot = &mut self.checked_max[addr.0 as usize];
        if slot.is_none_or(|cm| end_off > cm) {
            let prev = slot.replace(end_off);
            self.undo.push(UndoEntry::CheckedMax(addr, prev));
        }
        // If total linear memory capacity is strictly < 4 GiB, any successful check proves
        // `u64::from(addr) + end_off <= mem_max_bytes`, so `addr <= mem_max_bytes - end_off`!
        if self.mem_max_bytes <= u32::MAX as u64 && end_off <= self.mem_max_bytes {
            self.tighten_bound(addr, self.mem_max_bytes - end_off);
        }
    }

    fn is_checked(&self, addr: Val, end_off: u64) -> bool {
        if self.checked_max[addr.0 as usize].is_some_and(|cm| cm >= end_off) {
            return true;
        }
        if let Some(b) = self.non_wrap_bound[addr.0 as usize]
            && b.saturating_add(end_off) <= self.mem_min_bytes
        {
            return true;
        }
        false
    }

    fn clear_mem_facts(&mut self) {
        if !self.mem_facts.is_empty() {
            let old = std::mem::take(&mut self.mem_facts);
            self.undo.push(UndoEntry::MemClear(old));
        }
    }

    fn push_mem_fact(&mut self, fact: MemFact) {
        const MAX_MEM_FACTS: usize = 64;
        if self.mem_facts.len() < MAX_MEM_FACTS {
            self.mem_facts.push(fact);
            self.undo.push(UndoEntry::MemPush);
        }
    }

    /// Looks up a dominating `Load` or `Store` fact that can bit-exactly forward to `load_kind`.
    fn find_forward_val(
        &self,
        addr: Val,
        const_addr: Option<u64>,
        offset: u32,
        load_kind: LoadKind,
    ) -> Option<Val> {
        for fact in self.mem_facts.iter().rev() {
            let same_loc = (fact.addr == addr && fact.offset == offset)
                || (fact.const_addr.is_some() && fact.const_addr == const_addr);
            if !same_loc {
                continue;
            }
            match (fact.kind, load_kind) {
                (MemFactKind::Load(lk), rk) if lk == rk => return Some(fact.val),
                (MemFactKind::Store(StoreKind::I32), LoadKind::I32)
                | (MemFactKind::Store(StoreKind::I64), LoadKind::I64)
                | (MemFactKind::Store(StoreKind::F32), LoadKind::F32)
                | (MemFactKind::Store(StoreKind::F64), LoadKind::F64)
                | (MemFactKind::Store(StoreKind::V128), LoadKind::V128)
                    if fact.val_ty == load_kind.ty() =>
                {
                    return Some(fact.val);
                }
                (MemFactKind::Store(StoreKind::I8), LoadKind::I32Load8U)
                    if fact.val_ty == Ty::I32
                        && self.non_wrap_bound[fact.val.0 as usize].is_some_and(|b| b <= 255) =>
                {
                    return Some(fact.val);
                }
                (MemFactKind::Store(StoreKind::I16), LoadKind::I32Load16U)
                    if fact.val_ty == Ty::I32
                        && self.non_wrap_bound[fact.val.0 as usize]
                            .is_some_and(|b| b <= 65_535) =>
                {
                    return Some(fact.val);
                }
                _ => {}
            }
        }
        None
    }

    /// Invalidates all `mem_facts` that may alias `[addr + offset .. + sz)` while preserving
    /// provably disjoint facts (same SSA base with non-overlapping offsets, or disjoint constant addresses).
    fn kill_aliasing_mem_facts(
        &mut self,
        addr: Val,
        const_addr: Option<u64>,
        offset: u32,
        sz: u64,
    ) {
        let mut i = self.mem_facts.len();
        while i > 0 {
            i -= 1;
            let f = self.mem_facts[i];
            let disjoint = (f.addr == addr
                && ranges_disjoint(u64::from(f.offset), f.size, u64::from(offset), sz))
                || matches!(
                    (f.const_addr, const_addr),
                    (Some(a1), Some(a2)) if ranges_disjoint(a1, f.size, a2, sz)
                );
            if !disjoint {
                let removed = self.mem_facts.remove(i);
                self.undo.push(UndoEntry::MemRemove(i, removed));
            }
        }
    }

    fn try_fold_addr(&self, addr: Val, offset: u32) -> (Val, u32) {
        if let Some((base, k)) = self.add_const[addr.0 as usize]
            && let Some(bound) = self.non_wrap_bound[base.0 as usize]
            && bound.saturating_add(u64::from(k)) <= u32::MAX as u64
            && u64::from(offset).saturating_add(u64::from(k)) <= i32::MAX as u64
        {
            return (base, offset + k);
        }
        (addr, offset)
    }

    fn rollback(&mut self, mark: usize) {
        while self.undo.len() > mark {
            match self.undo.pop().unwrap() {
                UndoEntry::Expr(k, Some(v)) => {
                    self.exprs.insert(k, v);
                }
                UndoEntry::Expr(k, None) => {
                    self.exprs.remove(&k);
                }
                UndoEntry::Const32(k, prev) => {
                    self.const32[k.0 as usize] = prev;
                }
                UndoEntry::AddConst(k, prev) => {
                    self.add_const[k.0 as usize] = prev;
                }
                UndoEntry::Bound(k, prev) => {
                    self.non_wrap_bound[k.0 as usize] = prev;
                }
                UndoEntry::CheckedMax(k, prev) => {
                    self.checked_max[k.0 as usize] = prev;
                }
                UndoEntry::Global(k, prev) => {
                    self.globals[k as usize] = prev;
                }
                UndoEntry::MemSize(prev) => {
                    self.mem_size = prev;
                }
                UndoEntry::Table(k, Some(v)) => {
                    self.tables.insert(k, v);
                }
                UndoEntry::Table(k, None) => {
                    self.tables.remove(&k);
                }
                UndoEntry::MemPush => {
                    self.mem_facts.pop();
                }
                UndoEntry::MemRemove(idx, fact) => {
                    self.mem_facts.insert(idx, fact);
                }
                UndoEntry::MemClear(old) => {
                    self.mem_facts = old;
                }
            }
        }
    }
}

/// Evaluates a pure non-trapping 32-bit binary op on known constants.
fn fold_i32_bin_const(op: IntBinOp, ca: i32, cb: i32) -> Option<i32> {
    match op {
        IntBinOp::Add => Some(ca.wrapping_add(cb)),
        IntBinOp::Sub => Some(ca.wrapping_sub(cb)),
        IntBinOp::Mul => Some(ca.wrapping_mul(cb)),
        IntBinOp::DivS if cb != 0 && !(ca == i32::MIN && cb == -1) => Some(ca.wrapping_div(cb)),
        IntBinOp::DivU if cb != 0 => Some(((ca as u32) / (cb as u32)) as i32),
        IntBinOp::RemS if cb != 0 => Some(if ca == i32::MIN && cb == -1 {
            0
        } else {
            ca.wrapping_rem(cb)
        }),
        IntBinOp::RemU if cb != 0 => Some(((ca as u32) % (cb as u32)) as i32),
        IntBinOp::And => Some(ca & cb),
        IntBinOp::Or => Some(ca | cb),
        IntBinOp::Xor => Some(ca ^ cb),
        IntBinOp::Shl => Some(ca.wrapping_shl(cb as u32)),
        IntBinOp::ShrS => Some(ca.wrapping_shr(cb as u32)),
        IntBinOp::ShrU => Some(((ca as u32).wrapping_shr(cb as u32)) as i32),
        IntBinOp::Rotl => Some((ca as u32).rotate_left((cb as u32) & 31) as i32),
        IntBinOp::Rotr => Some((ca as u32).rotate_right((cb as u32) & 31) as i32),
        _ => None,
    }
}

fn fold_i32_cmp_const(op: CmpOp, ca: i32, cb: i32) -> bool {
    match op {
        CmpOp::Eq => ca == cb,
        CmpOp::Ne => ca != cb,
        CmpOp::LtS => ca < cb,
        CmpOp::LtU => (ca as u32) < (cb as u32),
        CmpOp::GtS => ca > cb,
        CmpOp::GtU => (ca as u32) > (cb as u32),
        CmpOp::LeS => ca <= cb,
        CmpOp::LeU => (ca as u32) <= (cb as u32),
        CmpOp::GeS => ca >= cb,
        CmpOp::GeU => (ca as u32) >= (cb as u32),
    }
}

/// Runs dominator-tree scoped GVN/CSE, post-inlining `const32` folding, `GlobalGet`/`MemSize`/`TableGet`/
/// alias-safe `Load` CSE, cross-block dominator `checked_max` bounds-check elimination, and wrap-safe
/// addressing-mode folding. Returns `true` if any conditional branch was constant-folded to `Jump`.
pub fn run_dom_gvn_and_bce(
    ir: &mut IrFunc,
    dom: &CfgDom,
    module: &X64Module,
    effects: &ModuleEffects,
    budget: Option<&CompileBudget>,
) -> Res<bool> {
    if dom.rpo.is_empty() {
        return Ok(false);
    }
    let mut folded_any_branch = false;

    let fixed_mem = module.fixed32.is_some();
    let mem_min_bytes = module
        .memory
        .map_or(0u64, |(min_p, _)| min_p.saturating_mul(65_536));
    let mem_max_bytes = module
        .memory
        .map_or(0u64, |(_, max_p)| max_p.saturating_mul(65_536));

    // Precompute exact function-level write/grow flags (so read-only globals, fixed memories, and
    // non-mutating functions keep facts valid across the ENTIRE dominator tree including joins & loops!).
    let mut fn_may_grow = false;
    let mut fn_writes_mem = false;
    let mut fn_writes_all_globals = false;
    let mut fn_written_globals: HashSet<u32> = HashSet::new();
    let mut fn_writes_all_tables = false;
    let mut fn_written_tables = 0u64;
    for &b in &dom.rpo {
        for inst in &ir.blocks[b as usize].insts {
            match inst.op {
                Op::Store { .. }
                | Op::MemCopy { .. }
                | Op::MemFill { .. }
                | Op::MemCopyConst { .. }
                | Op::MemFillConst { .. } => fn_writes_mem = true,
                Op::MemGrow(_) => {
                    fn_writes_mem = true;
                    if !fixed_mem {
                        fn_may_grow = true;
                    }
                }
                Op::GlobalSet(g, _) => {
                    fn_written_globals.insert(g);
                }
                Op::TableSet { table, .. } => {
                    if table < 64 {
                        fn_written_tables |= 1u64 << table;
                    } else {
                        fn_writes_all_tables = true;
                    }
                }
                Op::CallIndirect { .. } => {
                    fn_writes_mem = true;
                    if !fixed_mem {
                        fn_may_grow = true;
                    }
                    fn_writes_all_globals = true;
                    fn_writes_all_tables = true;
                }
                Op::Call { func, .. } => {
                    let f = func as usize;
                    if effects.writes_mem[f] {
                        fn_writes_mem = true;
                    }
                    if effects.grows_mem[f] {
                        fn_may_grow = true;
                    }
                    if effects.writes_all_globals[f] {
                        fn_writes_all_globals = true;
                    } else {
                        for w in 0..effects.global_words {
                            let mut bits = effects.written_globals[f][w];
                            while bits != 0 {
                                let tz = bits.trailing_zeros();
                                bits &= bits - 1;
                                fn_written_globals.insert((w as u32) * 64 + tz);
                            }
                        }
                    }
                    if effects.writes_all_tables[f] {
                        fn_writes_all_tables = true;
                    } else {
                        fn_written_tables |= effects.written_tables[f];
                    }
                }
                _ => {}
            }
        }
    }

    let n_vals = ir.val_tys.len();
    let mut st = GvnState {
        module,
        effects,
        fn_may_grow,
        fn_writes_mem,
        fn_writes_all_globals,
        fn_written_globals,
        fn_writes_all_tables,
        fn_written_tables,
        mem_min_bytes,
        mem_max_bytes,
        subst: (0..n_vals as u32).map(Val).collect(),
        any_subst: false,
        exprs: HashMap::new(),
        const32: vec![None; n_vals],
        add_const: vec![None; n_vals],
        non_wrap_bound: vec![None; n_vals],
        checked_max: vec![None; n_vals],
        globals: vec![None; module.globals.len()],
        mem_size: None,
        tables: HashMap::new(),
        mem_facts: Vec::new(),
        undo: Vec::new(),
        grow_epoch: 1,
        glob_epoch: 1,
        tbl_epoch: 1,
    };

    #[derive(Clone, Copy)]
    enum EdgeFact {
        None,
        Bound(Val, u64),
        LtU(Val, Val),
    }

    // Iterative DFS over dominator tree: `(block_id, edge_fact)` / `Exit { mark, epochs }`.
    enum DfsAction {
        Enter(u32, EdgeFact),
        Exit {
            mark: usize,
            epochs: (u32, u32, u32),
        },
    }
    let mut stack: Vec<DfsAction> = vec![DfsAction::Enter(0, EdgeFact::None)];
    let mut steps = 0usize;
    let mut pool = std::mem::take(&mut ir.val_pool);

    while let Some(act) = stack.pop() {
        steps += 1;
        if let Some(bg) = budget
            && (steps & 255) == 0
        {
            ir.val_pool = pool;
            bg.check_deadline()?;
            pool = std::mem::take(&mut ir.val_pool);
        }
        match act {
            DfsAction::Exit { mark, epochs } => {
                st.rollback(mark);
                st.grow_epoch = epochs.0;
                st.glob_epoch = epochs.1;
                st.tbl_epoch = epochs.2;
            }
            DfsAction::Enter(bid, edge_fact) => {
                let mark = st.undo.len();
                let saved_epochs = (st.grow_epoch, st.glob_epoch, st.tbl_epoch);
                stack.push(DfsAction::Exit {
                    mark,
                    epochs: saved_epochs,
                });

                // Apply branch-refined range fact when this block is uniquely dominated by one branch edge.
                match edge_fact {
                    EdgeFact::None => {}
                    EdgeFact::Bound(v, b) => st.tighten_bound(v, b),
                    EdgeFact::LtU(x, y) => {
                        if let Some(by) = st.non_wrap_bound[y.0 as usize]
                            && by > 0
                        {
                            st.tighten_bound(x, by - 1);
                        }
                        if let Some(cy) = st.checked_max[y.0 as usize] {
                            st.record_checked(x, cy.saturating_add(1));
                        }
                    }
                }

                // At multi-predecessor joins/loops, conservatively invalidate mutable state (read-only facts persist!).
                if dom.preds[bid as usize].len() > 1 {
                    if st.fn_may_grow {
                        st.grow_epoch = st.grow_epoch.wrapping_add(1);
                    }
                    if st.fn_writes_mem {
                        st.clear_mem_facts();
                    }
                    if st.fn_writes_all_globals || !st.fn_written_globals.is_empty() {
                        st.glob_epoch = st.glob_epoch.wrapping_add(1);
                    }
                    if st.fn_writes_all_tables || st.fn_written_tables != 0 {
                        st.tbl_epoch = st.tbl_epoch.wrapping_add(1);
                    }
                }

                let bb = &mut ir.blocks[bid as usize];
                let mut out_insts = Vec::with_capacity(bb.insts.len());
                for mut inst in bb.insts.drain(..) {
                    if st.any_subst {
                        rewrite_op_uses(&mut inst.op, &mut pool, &st.subst);
                    }

                    // Promote newly-known constant `MemCopy`/`MemFill` lengths (<= 64) after inlining/GVN.
                    match inst.op {
                        Op::MemCopy { dst, src, len } => {
                            if let Some(c) = st.const32[len.0 as usize]
                                && (c as u32) <= 64
                            {
                                inst.op = Op::MemCopyConst {
                                    dst,
                                    src,
                                    len: c as u32,
                                    dst_checked: false,
                                    src_checked: false,
                                };
                            }
                        }
                        Op::MemFill { dst, val, len } => {
                            if let Some(c) = st.const32[len.0 as usize]
                                && (c as u32) <= 64
                            {
                                inst.op = Op::MemFillConst {
                                    dst,
                                    val,
                                    len: c as u32,
                                    checked: false,
                                };
                            }
                        }
                        _ => {}
                    }

                    // Wrap-safe addressing-mode folding + cross-block dominator BCE + alias-aware Load/Store forwarding.
                    match &mut inst.op {
                        Op::BoundsCheck { addr, end_off } => {
                            if st.is_checked(*addr, *end_off) {
                                continue;
                            }
                            st.record_checked(*addr, *end_off);
                            out_insts.push(inst);
                            continue;
                        }
                        Op::Load {
                            kind,
                            addr,
                            offset,
                            checked,
                        } => {
                            let (na, noff) = st.try_fold_addr(*addr, *offset);
                            *addr = na;
                            *offset = noff;
                            let sz = kind.bytes();
                            let end = u64::from(*offset) + sz;
                            let is_safe = *checked || st.is_checked(*addr, end);
                            if is_safe {
                                *checked = true;
                            }
                            let const_addr = st.const32[addr.0 as usize]
                                .map(|c| u64::from(c as u32).saturating_add(u64::from(*offset)));
                            if is_safe
                                && let Some(res) = inst.res
                                && let Some(prev_v) =
                                    st.find_forward_val(*addr, const_addr, *offset, *kind)
                            {
                                // Dominated by an identical load or exact-width store with no aliasing write -> forward!
                                st.record_checked(*addr, end);
                                st.subst[res.0 as usize] = prev_v;
                                st.any_subst = true;
                                continue;
                            }
                            st.record_checked(*addr, end);
                            if let Some(res) = inst.res {
                                st.push_mem_fact(MemFact {
                                    addr: *addr,
                                    const_addr,
                                    offset: *offset,
                                    size: sz,
                                    kind: MemFactKind::Load(*kind),
                                    val: res,
                                    val_ty: ir.val_tys[res.0 as usize],
                                });
                                match *kind {
                                    LoadKind::I32Load8U => st.tighten_bound(res, 255),
                                    LoadKind::I32Load16U => st.tighten_bound(res, 65_535),
                                    _ => {}
                                }
                            }
                            out_insts.push(inst);
                            continue;
                        }
                        Op::Store {
                            kind,
                            addr,
                            val,
                            offset,
                            checked,
                        } => {
                            let (na, noff) = st.try_fold_addr(*addr, *offset);
                            *addr = na;
                            *offset = noff;
                            let sz = kind.bytes();
                            let end = u64::from(*offset) + sz;
                            if *checked || st.is_checked(*addr, end) {
                                *checked = true;
                            }
                            st.record_checked(*addr, end);
                            let const_addr = st.const32[addr.0 as usize]
                                .map(|c| u64::from(c as u32).saturating_add(u64::from(*offset)));
                            st.kill_aliasing_mem_facts(*addr, const_addr, *offset, sz);
                            st.push_mem_fact(MemFact {
                                addr: *addr,
                                const_addr,
                                offset: *offset,
                                size: sz,
                                kind: MemFactKind::Store(*kind),
                                val: *val,
                                val_ty: ir.val_tys[val.0 as usize],
                            });
                            out_insts.push(inst);
                            continue;
                        }
                        Op::MemCopyConst {
                            dst,
                            src,
                            len,
                            dst_checked,
                            src_checked,
                        } => {
                            let end = u64::from(*len);
                            if *dst_checked || st.is_checked(*dst, end) {
                                *dst_checked = true;
                            }
                            if *src_checked || st.is_checked(*src, end) {
                                *src_checked = true;
                            }
                            st.record_checked(*dst, end);
                            st.record_checked(*src, end);
                            st.clear_mem_facts();
                            out_insts.push(inst);
                            continue;
                        }
                        Op::MemFillConst {
                            dst, len, checked, ..
                        } => {
                            let end = u64::from(*len);
                            if *checked || st.is_checked(*dst, end) {
                                *checked = true;
                            }
                            st.record_checked(*dst, end);
                            st.clear_mem_facts();
                            out_insts.push(inst);
                            continue;
                        }
                        Op::MemCopy { .. } | Op::MemFill { .. } => {
                            st.clear_mem_facts();
                            out_insts.push(inst);
                            continue;
                        }
                        Op::MemGrow(_) => {
                            st.clear_mem_facts();
                            if st.module.fixed32.is_none() {
                                st.grow_epoch = st.grow_epoch.wrapping_add(1);
                            }
                            out_insts.push(inst);
                            continue;
                        }
                        Op::MemSize => {
                            if let Some(res) = inst.res {
                                let ep = if st.fn_may_grow { st.grow_epoch } else { 0 };
                                if let Some((sep, sv)) = st.mem_size
                                    && sep == ep
                                {
                                    st.subst[res.0 as usize] = sv;
                                    st.any_subst = true;
                                    continue;
                                }
                                let prev = st.mem_size;
                                st.mem_size = Some((ep, res));
                                st.undo.push(UndoEntry::MemSize(prev));
                            }
                            out_insts.push(inst);
                            continue;
                        }
                        Op::GlobalGet(g) => {
                            if let Some(res) = inst.res {
                                let gi = *g as usize;
                                if gi < st.globals.len() {
                                    let ep = if st.fn_writes_global(*g) {
                                        st.glob_epoch
                                    } else {
                                        0
                                    };
                                    if let Some((gep, gv)) = st.globals[gi]
                                        && gep == ep
                                    {
                                        st.subst[res.0 as usize] = gv;
                                        st.any_subst = true;
                                        continue;
                                    }
                                    let prev = st.globals[gi].replace((ep, res));
                                    st.undo.push(UndoEntry::Global(*g, prev));
                                }
                            }
                            out_insts.push(inst);
                            continue;
                        }
                        Op::GlobalSet(g, v) => {
                            let gi = *g as usize;
                            if gi < st.globals.len() {
                                let ep = if st.fn_writes_global(*g) {
                                    st.glob_epoch
                                } else {
                                    0
                                };
                                let prev = st.globals[gi].replace((ep, *v));
                                st.undo.push(UndoEntry::Global(*g, prev));
                            }
                            out_insts.push(inst);
                            continue;
                        }
                        Op::TableGet { table, idx } => {
                            if let Some(res) = inst.res {
                                let ep = if st.fn_writes_table(*table) {
                                    st.tbl_epoch
                                } else {
                                    0
                                };
                                let tkey = (*table, *idx);
                                if let Some(&(tep, tv)) = st.tables.get(&tkey)
                                    && tep == ep
                                {
                                    st.subst[res.0 as usize] = tv;
                                    st.any_subst = true;
                                    continue;
                                }
                                let prev = st.tables.insert(tkey, (ep, res));
                                st.undo.push(UndoEntry::Table(tkey, prev));
                            }
                            out_insts.push(inst);
                            continue;
                        }
                        Op::TableSet { table, .. } => {
                            let _ = table;
                            st.tbl_epoch = st.tbl_epoch.wrapping_add(1);
                            out_insts.push(inst);
                            continue;
                        }
                        Op::Call { func, .. } => {
                            let f = *func as usize;
                            if st.effects.writes_mem[f] {
                                st.clear_mem_facts();
                            }
                            if st.effects.grows_mem[f] {
                                st.grow_epoch = st.grow_epoch.wrapping_add(1);
                            }
                            if st.effects.writes_all_globals[f]
                                || (0..st.effects.global_words)
                                    .any(|w| st.effects.written_globals[f][w] != 0)
                            {
                                st.glob_epoch = st.glob_epoch.wrapping_add(1);
                            }
                            if st.effects.writes_all_tables[f] || st.effects.written_tables[f] != 0
                            {
                                st.tbl_epoch = st.tbl_epoch.wrapping_add(1);
                            }
                            out_insts.push(inst);
                            continue;
                        }
                        Op::CallIndirect { .. } => {
                            st.clear_mem_facts();
                            if st.module.fixed32.is_none() {
                                st.grow_epoch = st.grow_epoch.wrapping_add(1);
                            }
                            st.glob_epoch = st.glob_epoch.wrapping_add(1);
                            st.tbl_epoch = st.tbl_epoch.wrapping_add(1);
                            out_insts.push(inst);
                            continue;
                        }
                        _ => {}
                    }

                    // Post-inlining `const32` constant folding + select folding (simplifies inlined helpers
                    // like `blot:allocate(const)` directly into constants and dead branches).
                    if let Some(res) = inst.res {
                        match inst.op {
                            Op::I32Eqz(a) => {
                                if let Some(ca) = st.const32[a.0 as usize] {
                                    inst.op = Op::ConstI32(i32::from(ca == 0));
                                }
                            }
                            Op::I32BinImm(bop, a, imm) => {
                                if let Some(ca) = st.const32[a.0 as usize]
                                    && let Some(v) = fold_i32_bin_const(bop, ca, imm)
                                {
                                    inst.op = Op::ConstI32(v);
                                }
                            }
                            Op::I32Bin(bop, a, b) => {
                                if let (Some(ca), Some(cb)) =
                                    (st.const32[a.0 as usize], st.const32[b.0 as usize])
                                    && let Some(v) = fold_i32_bin_const(bop, ca, cb)
                                {
                                    inst.op = Op::ConstI32(v);
                                }
                            }
                            Op::I32CmpImm(cop, a, imm) => {
                                if let Some(ca) = st.const32[a.0 as usize] {
                                    inst.op =
                                        Op::ConstI32(i32::from(fold_i32_cmp_const(cop, ca, imm)));
                                }
                            }
                            Op::I32Cmp(cop, a, b) => {
                                if let (Some(ca), Some(cb)) =
                                    (st.const32[a.0 as usize], st.const32[b.0 as usize])
                                {
                                    inst.op =
                                        Op::ConstI32(i32::from(fold_i32_cmp_const(cop, ca, cb)));
                                }
                            }
                            Op::Select { cond, a, b } => {
                                if let Some(c) = st.const32[cond.0 as usize] {
                                    st.subst[res.0 as usize] = if c != 0 { a } else { b };
                                    st.any_subst = true;
                                    continue;
                                } else if a == b {
                                    st.subst[res.0 as usize] = a;
                                    st.any_subst = true;
                                    continue;
                                }
                            }
                            Op::CmpSelect { kind, a, b, tv, fv } => {
                                let known = match kind {
                                    CmpBranchKind::I32(cop) => st.const32[a.0 as usize]
                                        .zip(st.const32[b.0 as usize])
                                        .map(|(ca, cb)| fold_i32_cmp_const(cop, ca, cb)),
                                    CmpBranchKind::I32Imm(cop, imm) => st.const32[a.0 as usize]
                                        .map(|ca| fold_i32_cmp_const(cop, ca, imm)),
                                    _ => None,
                                };
                                if let Some(take_tv) = known {
                                    st.subst[res.0 as usize] = if take_tv { tv } else { fv };
                                    st.any_subst = true;
                                    continue;
                                } else if tv == fv {
                                    st.subst[res.0 as usize] = tv;
                                    st.any_subst = true;
                                    continue;
                                }
                            }
                            _ => {}
                        }
                    }

                    // Pure expression GVN + constant / non-wrap-bound tracking.
                    if let Some(res) = inst.res {
                        if let Some(key) = canonical_gvn_key(inst.op) {
                            if let Some(&prev_v) = st.exprs.get(&key) {
                                st.subst[res.0 as usize] = prev_v;
                                st.any_subst = true;
                                continue;
                            }
                            st.insert_expr(key, res);
                        }
                        match inst.op {
                            Op::ConstI32(c) => {
                                st.insert_const32(res, c);
                                st.tighten_bound(res, u64::from(c as u32));
                            }
                            Op::I32BinImm(IntBinOp::Add, base, k) if k > 0 => {
                                st.insert_add_const(res, base, k as u32);
                                if let Some(b) = st.non_wrap_bound[base.0 as usize]
                                    && b.saturating_add(u64::from(k as u32)) <= u32::MAX as u64
                                {
                                    st.tighten_bound(res, b + u64::from(k as u32));
                                }
                            }
                            Op::I32BinImm(IntBinOp::And, _, m) => {
                                st.tighten_bound(res, u64::from(m as u32));
                            }
                            Op::I32BinImm(IntBinOp::ShrU, _, sh) if (1..32).contains(&sh) => {
                                st.tighten_bound(res, u64::from(u32::MAX >> (sh as u32)));
                            }
                            Op::I32BinImm(IntBinOp::Shl, base, sh) if (1..32).contains(&sh) => {
                                if let Some(b) = st.non_wrap_bound[base.0 as usize]
                                    && b <= u64::from(u32::MAX >> (sh as u32))
                                {
                                    st.tighten_bound(res, b << (sh as u32));
                                }
                            }
                            Op::Select { a, b, .. } | Op::CmpSelect { tv: a, fv: b, .. } => {
                                if let (Some(ba), Some(bb)) = (
                                    st.non_wrap_bound[a.0 as usize],
                                    st.non_wrap_bound[b.0 as usize],
                                ) {
                                    st.tighten_bound(res, ba.max(bb));
                                }
                            }
                            _ => {}
                        }
                    }
                    out_insts.push(inst);
                }
                bb.insts = out_insts;
                if st.any_subst {
                    rewrite_term_uses(&mut bb.term, &mut pool, &ir.br_tables, &st.subst);
                }
                // Constant-fold newly known conditional branches after inlining/GVN.
                match bb.term {
                    Term::BranchIf {
                        cond,
                        then_bb,
                        then_args,
                        else_bb,
                        else_args,
                    } => {
                        if let Some(c) = st.const32[cond.0 as usize] {
                            bb.term = if c != 0 {
                                Term::Jump {
                                    target: then_bb,
                                    args: then_args,
                                }
                            } else {
                                Term::Jump {
                                    target: else_bb,
                                    args: else_args,
                                }
                            };
                            folded_any_branch = true;
                        }
                    }
                    Term::CmpBranch {
                        kind,
                        a,
                        b,
                        then_bb,
                        then_args,
                        else_bb,
                        else_args,
                    } => {
                        let known = match kind {
                            CmpBranchKind::I32(cop) => st.const32[a.0 as usize]
                                .zip(st.const32[b.0 as usize])
                                .map(|(ca, cb)| fold_i32_cmp_const(cop, ca, cb)),
                            CmpBranchKind::I32Imm(cop, imm) => {
                                st.const32[a.0 as usize].map(|ca| fold_i32_cmp_const(cop, ca, imm))
                            }
                            _ => None,
                        };
                        if let Some(take_then) = known {
                            bb.term = if take_then {
                                Term::Jump {
                                    target: then_bb,
                                    args: then_args,
                                }
                            } else {
                                Term::Jump {
                                    target: else_bb,
                                    args: else_args,
                                }
                            };
                            folded_any_branch = true;
                        }
                    }
                    _ => {}
                }

                // Compute optional branch-refined range facts for uniquely dominated children.
                let term_snap = bb.term;
                // Push dom_children in reverse so they visit in forward order.
                for &child in dom.dom_children[bid as usize].iter().rev() {
                    let mut ef = EdgeFact::None;
                    if dom.preds[child as usize].len() == 1
                        && dom.preds[child as usize][0] == bid
                        && let Term::CmpBranch {
                            kind,
                            a,
                            b,
                            then_bb,
                            else_bb,
                            ..
                        } = term_snap
                        && then_bb != else_bb
                    {
                        let on_then = child == then_bb.0;
                        let on_else = child == else_bb.0;
                        if on_then || on_else {
                            match kind {
                                CmpBranchKind::I32Imm(cop, imm) => {
                                    let eff = if on_then { cop } else { cop.invert_int() };
                                    let ku = imm as u32;
                                    match eff {
                                        CmpOp::LtU if ku > 0 => {
                                            ef = EdgeFact::Bound(a, u64::from(ku - 1));
                                        }
                                        CmpOp::LeU => {
                                            ef = EdgeFact::Bound(a, u64::from(ku));
                                        }
                                        _ => {}
                                    }
                                }
                                CmpBranchKind::I32(cop) => {
                                    let eff = if on_then { cop } else { cop.invert_int() };
                                    match eff {
                                        CmpOp::LtU => ef = EdgeFact::LtU(a, b),
                                        CmpOp::GtU => ef = EdgeFact::LtU(b, a),
                                        _ => {}
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    stack.push(DfsAction::Enter(child, ef));
                }
            }
        }
    }
    ir.val_pool = pool;
    Ok(folded_any_branch)
}

/// Converts trivial single-predecessor `if/else` diamonds with <= 2 strictly pure non-trapping
/// GP instructions per arm into branchless `Op::CmpSelect` / `Op::Select` + `Term::Jump(merge)`.
/// Returns `true` if any diamond was converted (so caller can compact merged blocks).
fn if_convert_trivial_diamonds(ir: &mut IrFunc) -> bool {
    let n = ir.blocks.len();
    if n < 3 {
        return false;
    }
    let is_trivial_pure = |op: &Op| -> bool {
        matches!(
            *op,
            Op::ConstI32(_)
                | Op::ConstI64(_)
                | Op::I32Eqz(_)
                | Op::I64Eqz(_)
                | Op::I32WrapI64(_)
                | Op::I64ExtendI32S(_)
                | Op::I64ExtendI32U(_)
                | Op::I32Extend8S(_)
                | Op::I32Extend16S(_)
                | Op::I64Extend8S(_)
                | Op::I64Extend16S(_)
                | Op::I64Extend32S(_)
                | Op::I32BinImm(
                    IntBinOp::Add
                        | IntBinOp::Sub
                        | IntBinOp::And
                        | IntBinOp::Or
                        | IntBinOp::Xor
                        | IntBinOp::Shl
                        | IntBinOp::ShrS
                        | IntBinOp::ShrU,
                    _,
                    _
                )
                | Op::I64BinImm(
                    IntBinOp::Add
                        | IntBinOp::Sub
                        | IntBinOp::And
                        | IntBinOp::Or
                        | IntBinOp::Xor
                        | IntBinOp::Shl
                        | IntBinOp::ShrS
                        | IntBinOp::ShrU,
                    _,
                    _
                )
                | Op::I32Bin(
                    IntBinOp::Add
                        | IntBinOp::Sub
                        | IntBinOp::And
                        | IntBinOp::Or
                        | IntBinOp::Xor
                        | IntBinOp::Shl
                        | IntBinOp::ShrS
                        | IntBinOp::ShrU,
                    _,
                    _
                )
                | Op::I64Bin(
                    IntBinOp::Add
                        | IntBinOp::Sub
                        | IntBinOp::And
                        | IntBinOp::Or
                        | IntBinOp::Xor
                        | IntBinOp::Shl
                        | IntBinOp::ShrS
                        | IntBinOp::ShrU,
                    _,
                    _
                )
        )
    };

    // Exact incoming edge counts so we never speculatively hoist a shared block.
    let mut edge_in = vec![0u32; n];
    for bb in &ir.blocks {
        match bb.term {
            Term::Jump { target, .. } => {
                edge_in[target.0 as usize] = edge_in[target.0 as usize].saturating_add(1);
            }
            Term::BranchIf {
                then_bb, else_bb, ..
            }
            | Term::CmpBranch {
                then_bb, else_bb, ..
            } => {
                edge_in[then_bb.0 as usize] = edge_in[then_bb.0 as usize].saturating_add(1);
                edge_in[else_bb.0 as usize] = edge_in[else_bb.0 as usize].saturating_add(1);
            }
            Term::BrTable { table_idx, .. } => {
                let td = &ir.br_tables[table_idx as usize];
                for &(t, _) in &td.unique_edges {
                    edge_in[t.0 as usize] = edge_in[t.0 as usize].saturating_add(1);
                }
            }
            Term::Return { .. } | Term::Unreachable => {}
        }
    }

    let mut converted_any = false;
    let mut dead_arm = vec![false; n];
    for bi in 0..n {
        if dead_arm[bi] {
            continue;
        }
        let (cmp_info, then_bb, then_args, else_bb, else_args) = match ir.blocks[bi].term {
            Term::BranchIf {
                cond,
                then_bb,
                then_args,
                else_bb,
                else_args,
            } => (Err(cond), then_bb, then_args, else_bb, else_args),
            Term::CmpBranch {
                kind,
                a,
                b,
                then_bb,
                then_args,
                else_bb,
                else_args,
            } => (Ok((kind, a, b)), then_bb, then_args, else_bb, else_args),
            _ => continue,
        };
        let ti = then_bb.0 as usize;
        let ei = else_bb.0 as usize;
        if ti == ei || ti == bi || ei == bi || dead_arm[ti] || dead_arm[ei] {
            continue;
        }
        let is_trivial_arm = |idx: usize| -> bool {
            idx != 0
                && !dead_arm[idx]
                && edge_in[idx] == 1
                && ir.blocks[idx].params.is_empty()
                && ir.blocks[idx].insts.len() <= 2
                && ir.blocks[idx]
                    .insts
                    .iter()
                    .all(|ins| is_trivial_pure(&ins.op))
        };

        // Case 1: Classic 2-arm diamond (`bi -> ti -> merge`, `bi -> ei -> merge`).
        // Case 2: Single-arm triangle (`bi -> ti -> merge`, `bi -> merge` directly via `else_bb`).
        // Case 3: Single-arm triangle (`bi -> merge` directly via `then_bb`, `bi -> ei -> merge`).
        let (merge_bb, ta, ea, kill_t, kill_e) = if then_args.len == 0
            && else_args.len == 0
            && is_trivial_arm(ti)
            && is_trivial_arm(ei)
            && let (
                Term::Jump {
                    target: tm,
                    args: ta,
                },
                Term::Jump {
                    target: em,
                    args: ea,
                },
            ) = (ir.blocks[ti].term, ir.blocks[ei].term)
            && tm == em
            && ta.len == ea.len
            && tm.0 as usize != ti
            && tm.0 as usize != ei
        {
            (tm, ta, ea, true, true)
        } else if then_args.len == 0
            && is_trivial_arm(ti)
            && let Term::Jump {
                target: tm,
                args: ta,
            } = ir.blocks[ti].term
            && tm == else_bb
            && ta.len == else_args.len
            && tm.0 as usize != ti
        {
            (tm, ta, else_args, true, false)
        } else if else_args.len == 0
            && is_trivial_arm(ei)
            && let Term::Jump {
                target: em,
                args: ea,
            } = ir.blocks[ei].term
            && em == then_bb
            && ea.len == then_args.len
            && em.0 as usize != ei
        {
            (em, then_args, ea, false, true)
        } else {
            continue;
        };

        // Check differing positions: allow up to 2 GP values (lowered to branchless `cmovcc`).
        let t_slice = ir.pool_slice(ta);
        let e_slice = ir.pool_slice(ea);
        let mut diffs = 0usize;
        let mut all_gp = true;
        for (&tv, &ev) in t_slice.iter().zip(e_slice.iter()) {
            if tv != ev {
                diffs += 1;
                if !ir.val_tys[tv.0 as usize].is_gp()
                    || ir.val_tys[tv.0 as usize] != ir.val_tys[ev.0 as usize]
                {
                    all_gp = false;
                    break;
                }
            }
        }
        if !all_gp || diffs > 2 {
            continue;
        }
        if ir.val_tys.len() + diffs > MAX_IR_VALS_PER_FN
            || ir.val_pool.len() + (ta.len as usize) > MAX_VAL_POOL_PER_FN
        {
            continue;
        }

        // Speculatively hoist trivial arm(s) into `bi` and emit `CmpSelect`/`Select` per diff.
        // Push merged edge args into `val_pool` BEFORE mutating any blocks so an error leaves CFG intact.
        let t_vals: Vec<Val> = t_slice.to_vec();
        let e_vals: Vec<Val> = e_slice.to_vec();
        let mut sel_insts = Vec::with_capacity(diffs);
        let mut merged_args = Vec::with_capacity(t_vals.len());
        for (tv, ev) in t_vals.into_iter().zip(e_vals) {
            if tv == ev {
                merged_args.push(tv);
            } else {
                let ty = ir.val_tys[tv.0 as usize];
                let sel_v = ir.push_val(ty);
                let sel_op = match cmp_info {
                    Ok((kind, a, b)) => Op::CmpSelect {
                        kind,
                        a,
                        b,
                        tv,
                        fv: ev,
                    },
                    Err(cond) => Op::Select { cond, a: tv, b: ev },
                };
                sel_insts.push(Inst {
                    res: Some(sel_v),
                    op: sel_op,
                });
                merged_args.push(sel_v);
            }
        }
        let Ok(range) = ir.push_pool(&merged_args) else {
            continue;
        };
        if kill_t {
            let t_insts = std::mem::take(&mut ir.blocks[ti].insts);
            ir.blocks[ti].term = Term::Unreachable;
            dead_arm[ti] = true;
            ir.blocks[bi].insts.extend(t_insts);
        }
        if kill_e {
            let e_insts = std::mem::take(&mut ir.blocks[ei].insts);
            ir.blocks[ei].term = Term::Unreachable;
            dead_arm[ei] = true;
            ir.blocks[bi].insts.extend(e_insts);
        }
        ir.blocks[bi].insts.extend(sel_insts);
        ir.blocks[bi].term = Term::Jump {
            target: merge_bb,
            args: range,
        };
        converted_any = true;
    }
    converted_any
}

/// Complete per-function Stage 2 IR optimization sequence (runs in parallel after inlining & effect analysis).
pub fn optimize_func_stage2(
    ir: &mut IrFunc,
    module: &X64Module,
    effects: &ModuleEffects,
    budget: Option<&CompileBudget>,
) -> Res<()> {
    // 1. Compact any jump chains left by inlining and prune redundant/dead block params FIRST so
    //    dead loop/if local carries never block if-conversion, LICM, GVN, or regalloc!
    let mut dom = rebuild_cfg_and_prune(ir, budget)?;
    let _ = prune_block_params(ir, &dom, budget)?;
    // 2. Convert trivial pure `if/else` diamonds into branchless `CmpSelect` and re-compact if any converted.
    if if_convert_trivial_diamonds(ir) {
        dom = rebuild_cfg_and_prune(ir, budget)?;
        let _ = prune_block_params(ir, &dom, budget)?;
    }
    // 3. LICM (incl. guarded loop-invariant DivU/RemU hoisting) + Dominator GVN/BCE/Store-to-Load + peephole/DCE.
    if run_licm(ir, &dom, module, effects, budget)? {
        dom = rebuild_cfg_and_prune(ir, budget)?;
    }
    let folded_branch = run_dom_gvn_and_bce(ir, &dom, module, effects, budget)?;
    let prev_blocks = ir.blocks.len();
    optimize_ir_with_budget(ir, budget)?;
    if folded_branch || (prev_blocks > 1 && ir.blocks.iter().skip(1).any(|b| b.preds == 0)) {
        dom = rebuild_cfg_and_prune(ir, budget)?;
    }
    // 4. Final block-param pruning cleans up any newly redundant/dead phis exposed by GVN/branch folding.
    let _ = prune_block_params(ir, &dom, budget)?;
    Ok(())
}

// ============================================================================
// 6. Strictly Coupled 2-Phase Stack-Check Elision (8a) -> Direct-Call Trap-Check Elision (8b)
// ============================================================================

fn op_may_trap_locally(op: &Op) -> bool {
    match *op {
        Op::I32Bin(bop, _, _) | Op::I64Bin(bop, _, _) => {
            matches!(
                bop,
                IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU
            )
        }
        Op::I32BinImm(bop, _, imm) | Op::I64BinImm(bop, _, imm) => {
            matches!(
                bop,
                IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU
            ) && (imm == 0 || (bop == IntBinOp::DivS && imm == -1))
        }
        Op::Trunc(t, _) => t.can_trap(),
        Op::Load { checked: false, .. }
        | Op::Store { checked: false, .. }
        | Op::BoundsCheck { .. }
        | Op::TableGet { .. }
        | Op::TableSet { .. }
        | Op::MemCopy { .. }
        | Op::MemFill { .. }
        | Op::MemCopyConst {
            dst_checked: false, ..
        }
        | Op::MemCopyConst {
            src_checked: false, ..
        }
        | Op::MemFillConst { checked: false, .. }
        | Op::CallIndirect { .. }
        | Op::Trap(_) => true,
        _ => false,
    }
}

/// Per-function post-opt/post-regalloc input to Step 8a/8b stack & trap-check elision.
#[derive(Clone)]
pub struct FnElideInput {
    pub own_stack_bytes: i32,
    pub local_may_trap: bool,
    pub calls_import_or_indirect: bool,
    pub max_helper_bytes: i32,
    /// Unique sorted direct defined callee indices `callee_bi` (`0..n_defs`).
    pub direct_def_callees: Vec<u32>,
}

pub fn try_compute_fn_elide_input(
    ir: &IrFunc,
    alloc: &AllocResult,
    n_imports: usize,
    n_defs: usize,
) -> Option<FnElideInput> {
    let own_stack_bytes = alloc
        .frame_size
        .saturating_add((alloc.used_callee_gp.len() as i32) * 8)
        .saturating_add(8);
    let mut local_may_trap = false;
    let mut calls_import_or_indirect = false;
    let mut max_helper_bytes = 0i32;
    let mut direct_def_callees: Vec<u32> = Vec::new();
    for bb in &ir.blocks {
        if matches!(bb.term, Term::Unreachable) {
            local_may_trap = true;
        }
        for inst in &bb.insts {
            if op_may_trap_locally(&inst.op) {
                local_may_trap = true;
            }
            match inst.op {
                Op::Call { func, .. } => {
                    let f = func as usize;
                    if f < n_imports {
                        calls_import_or_indirect = true;
                    } else if f - n_imports < n_defs {
                        if direct_def_callees.len() == direct_def_callees.capacity() {
                            direct_def_callees.try_reserve(1).ok()?;
                        }
                        direct_def_callees.push((f - n_imports) as u32);
                    }
                }
                Op::CallIndirect { .. } => {
                    calls_import_or_indirect = true;
                }
                Op::Trunc(_, _) | Op::MemGrow(_) | Op::MemCopy { .. } | Op::MemFill { .. } => {
                    max_helper_bytes = max_helper_bytes.max(PURE_HELPER_STACK_BYTES);
                }
                _ => {}
            }
        }
    }
    direct_def_callees.sort_unstable();
    direct_def_callees.dedup();
    Some(FnElideInput {
        own_stack_bytes,
        local_may_trap,
        calls_import_or_indirect,
        max_helper_bytes,
        direct_def_callees,
    })
}

/// Output of Step 8a/8b stack & trap-check elision across all defined functions.
pub struct ElideOutput {
    pub needs_stack_check: Vec<bool>,
    pub elided_callee_stack_bytes: Vec<i32>,
    pub fn_can_trap: Vec<bool>,
}

/// Fallibly runs Step 8a + Step 8b strictly from per-function `FnElideInput` summaries,
/// returning `Ok(None)` on allocation failure so optional cache work can fall back cleanly.
pub fn try_run_stack_and_trap_elision_from_summaries(
    inputs: &[FnElideInput],
    module: &X64Module,
    budget: Option<&CompileBudget>,
) -> Res<Option<ElideOutput>> {
    if let Some(bg) = budget {
        bg.check_deadline()?;
    }
    let n_imports = module.imports.len();
    let n_defs = inputs.len();
    let n_total = n_imports + n_defs;

    let setup = (|| -> Option<_> {
        let mut edges: Vec<Vec<usize>> = Vec::new();
        edges.try_reserve_exact(n_defs).ok()?;
        for inp in inputs {
            let mut row = Vec::new();
            row.try_reserve_exact(inp.direct_def_callees.len()).ok()?;
            for &c in &inp.direct_def_callees {
                row.push(c as usize);
            }
            edges.push(row);
        }
        let (in_cycle, topo_bottom_up) = try_call_graph_sccs_from_edges(&edges)?;
        let mut cannot_elide = Vec::new();
        cannot_elide.try_reserve_exact(n_defs).ok()?;
        cannot_elide.resize(n_defs, false);
        let mut elided = Vec::new();
        elided.try_reserve_exact(n_defs).ok()?;
        elided.resize(n_defs, false);
        let mut subtree_bytes = Vec::new();
        subtree_bytes.try_reserve_exact(n_defs).ok()?;
        subtree_bytes.resize(n_defs, 0i32);
        let mut needs_stack_check = Vec::new();
        needs_stack_check.try_reserve_exact(n_defs).ok()?;
        needs_stack_check.resize(n_defs, true);
        let mut elided_callee_stack_bytes = Vec::new();
        elided_callee_stack_bytes.try_reserve_exact(n_defs).ok()?;
        elided_callee_stack_bytes.resize(n_defs, 0i32);
        let mut fn_can_trap = Vec::new();
        fn_can_trap.try_reserve_exact(n_total).ok()?;
        fn_can_trap.resize(n_total, true);
        Some((
            in_cycle,
            topo_bottom_up,
            cannot_elide,
            elided,
            subtree_bytes,
            needs_stack_check,
            elided_callee_stack_bytes,
            fn_can_trap,
        ))
    })();
    let Some((
        in_cycle,
        topo_bottom_up,
        mut cannot_elide,
        mut elided,
        mut subtree_bytes,
        mut needs_stack_check,
        mut elided_callee_stack_bytes,
        mut fn_can_trap,
    )) = setup
    else {
        return Ok(None);
    };

    for &(_, f) in &module.exported_funcs {
        if f >= n_imports && f - n_imports < n_defs {
            cannot_elide[f - n_imports] = true;
        }
    }
    if let Some(st_fi) = module.start {
        let f = st_fi as usize;
        if f >= n_imports && f - n_imports < n_defs {
            cannot_elide[f - n_imports] = true;
        }
    }
    for ci in &module.ci_sigs {
        for &(_, fi) in &ci.targets {
            let f = fi as usize;
            if f >= n_imports && f - n_imports < n_defs {
                cannot_elide[f - n_imports] = true;
            }
        }
    }
    for (bi, inp) in inputs.iter().enumerate() {
        if in_cycle[bi] || inp.calls_import_or_indirect {
            cannot_elide[bi] = true;
        }
    }

    for &bi in &topo_bottom_up {
        if cannot_elide[bi] {
            continue;
        }
        let inp = &inputs[bi];
        if inp.direct_def_callees.iter().any(|&c| !elided[c as usize]) {
            continue;
        }
        let max_callee = inp
            .direct_def_callees
            .iter()
            .map(|&c| subtree_bytes[c as usize])
            .max()
            .unwrap_or(0)
            .max(inp.max_helper_bytes);
        let total = inp.own_stack_bytes.saturating_add(max_callee);
        if total <= ELIDE_MAX_SUBTREE_STACK_BYTES {
            elided[bi] = true;
            subtree_bytes[bi] = total;
        }
    }

    for bi in 0..n_defs {
        if elided[bi] {
            needs_stack_check[bi] = false;
            elided_callee_stack_bytes[bi] = 0;
        } else {
            needs_stack_check[bi] = true;
            let reserve = inputs[bi]
                .direct_def_callees
                .iter()
                .filter(|&&c| elided[c as usize])
                .map(|&c| subtree_bytes[c as usize])
                .max()
                .unwrap_or(0);
            elided_callee_stack_bytes[bi] = reserve;
        }
    }

    for bi in 0..n_defs {
        let fi = n_imports + bi;
        fn_can_trap[fi] = needs_stack_check[bi]
            || inputs[bi].local_may_trap
            || inputs[bi].calls_import_or_indirect;
    }
    let mut changed = true;
    let mut fix_steps = 0usize;
    while changed {
        changed = false;
        for (bi, inp) in inputs.iter().enumerate() {
            fix_steps += 1;
            if let Some(bg) = budget
                && (fix_steps & 4095) == 0
            {
                bg.check_deadline()?;
            }
            let fi = n_imports + bi;
            if fn_can_trap[fi] {
                continue;
            }
            if inp
                .direct_def_callees
                .iter()
                .any(|&c| fn_can_trap[n_imports + c as usize])
            {
                fn_can_trap[fi] = true;
                changed = true;
            }
        }
    }

    Ok(Some(ElideOutput {
        needs_stack_check,
        elided_callee_stack_bytes,
        fn_can_trap,
    }))
}

/// Runs Step 8a + Step 8b strictly from per-function `FnElideInput` summaries.
pub fn run_stack_and_trap_elision_from_summaries(
    inputs: &[FnElideInput],
    module: &X64Module,
    budget: Option<&CompileBudget>,
) -> Res<ElideOutput> {
    try_run_stack_and_trap_elision_from_summaries(inputs, module, budget)?
        .ok_or_else(|| "x64: out of memory allocating stack/trap elision buffers".to_string())
}

/// Runs Step 8a (native-frame stack-check elision using final post-regalloc `frame_size`)
/// and strictly after it Step 8b (transitive direct-call trap-check elision).
/// Returns `fn_can_trap: Vec<bool>` indexed by Wasm `func_index` (`0..n_imports + n_defs`).
pub fn run_stack_and_trap_elision(
    irs: &mut [IrFunc],
    allocs: &[AllocResult],
    module: &X64Module,
    budget: Option<&CompileBudget>,
) -> Res<Vec<bool>> {
    let n_imports = module.imports.len();
    let n_defs = irs.len();
    let mut inputs = Vec::new();
    inputs
        .try_reserve_exact(n_defs)
        .map_err(|_| "x64: out of memory allocating elision summaries".to_string())?;
    for (ir, alloc) in irs.iter().zip(allocs) {
        let inp = try_compute_fn_elide_input(ir, alloc, n_imports, n_defs)
            .ok_or_else(|| "x64: out of memory allocating elision summary".to_string())?;
        inputs.push(inp);
    }
    let out = run_stack_and_trap_elision_from_summaries(&inputs, module, budget)?;
    for (bi, ir) in irs.iter_mut().enumerate() {
        ir.needs_stack_check = out.needs_stack_check[bi];
        ir.elided_callee_stack_bytes = out.elided_callee_stack_bytes[bi];
    }
    Ok(out.fn_can_trap)
}
