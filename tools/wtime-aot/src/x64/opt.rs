//! Stage 2 SSA IR optimization pipeline & interprocedural analyses (`#![forbid(unsafe_code)]`).
//!
//! Dependency-ordered passes (`CONTINUE.md` §5.1):
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
    Block, BlockId, BrTableData, CmpBranchKind, CmpOp, FloatBinOp, FloatUnOp, Inst, IntBinOp, IrFunc,
    LoadKind, Op, Term, TruncOp, Val, ValRange, optimize_ir_with_budget,
};
use super::module::{CompileBudget, MAX_BLOCKS_PER_FN, MAX_IR_VALS_PER_FN, MAX_VAL_POOL_PER_FN, X64Module};
use super::regalloc::AllocResult;

type Res<T> = Result<T, String>;

const INLINE_MAX_CALLEE_INSTS: usize = 48;
const INLINE_MAX_CALLEE_BLOCKS: usize = 12;
const INLINE_MAX_CALLEE_VALS: usize = 128;
const INLINE_MAX_CALLER_INSTS: usize = 4096;
const INLINE_MAX_CALLER_BLOCKS: usize = 512;
const INLINE_MAX_CALLER_VALS: usize = 8192;
const INLINE_MAX_ADDED_PER_CALLER: usize = 768;
const INLINE_MAX_MODULE_ADDED_INSTS: usize = 250_000;
const LICM_MAX_HOIST_PER_LOOP: usize = 8;
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
        Ok(CfgDom { rpo, rpo_pos, preds, succs, idom, dom_children })
    }

    #[inline]
    pub fn dominates(&self, a: u32, mut b: u32) -> bool {
        if a == b {
            return true;
        }
        if self.rpo_pos[a as usize] == u32::MAX || self.rpo_pos[b as usize] == u32::MAX {
            return false;
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
        | Op::Store { addr: a, val: b, .. }
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
        Term::BranchIf { ref mut cond, then_args, else_args, .. } => {
            *cond = resolve_subst(subst, *cond);
            rw_range(pool, then_args);
            rw_range(pool, else_args);
        }
        Term::CmpBranch { kind, ref mut a, ref mut b, then_args, else_args, .. } => {
            *a = resolve_subst(subst, *a);
            if !matches!(kind, CmpBranchKind::I32Imm(..) | CmpBranchKind::I64Imm(..)) {
                *b = resolve_subst(subst, *b);
            } else {
                *b = *a;
            }
            rw_range(pool, then_args);
            rw_range(pool, else_args);
        }
        Term::BrTable { ref mut idx, table_idx } => {
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
                Term::BranchIf { then_bb, else_bb, .. } | Term::CmpBranch { then_bb, else_bb, .. } => {
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
        let mut old_bt_opt: Vec<Option<BrTableData>> = old_br_tables.into_iter().map(Some).collect();
        let mut new_br_tables: Vec<BrTableData> = Vec::new();
        let mut bt_remap = vec![u32::MAX; old_bt_opt.len()];
        for bb in &mut new_blocks {
            match &mut bb.term {
                Term::Jump { target, .. } => *target = map_b(*target),
                Term::BranchIf { then_bb, else_bb, .. } | Term::CmpBranch { then_bb, else_bb, .. } => {
                    *then_bb = map_b(*then_bb);
                    *else_bb = map_b(*else_bb);
                }
                Term::BrTable { table_idx, .. } => {
                    let old_ti = *table_idx as usize;
                    let new_ti = if bt_remap[old_ti] != u32::MAX {
                        bt_remap[old_ti]
                    } else {
                        let mut td = old_bt_opt[old_ti].take().expect("br_table entry referenced once");
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
                Term::BranchIf { then_bb, then_args, else_bb, else_args, .. }
                | Term::CmpBranch { then_bb, then_args, else_bb, else_args, .. } => {
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
        let mut live_inst: Vec<Vec<bool>> = ir.blocks.iter().map(|bb| vec![false; bb.insts.len()]).collect();
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
                    inst.op.for_each_use(&ir.val_pool, |v| mark(v, &mut live_val, &mut worklist));
                }
            }
            match bb.term {
                Term::Jump { .. } | Term::Unreachable => {}
                Term::BranchIf { cond, .. } => mark(cond, &mut live_val, &mut worklist),
                Term::CmpBranch { kind, a, b, .. } => {
                    mark(a, &mut live_val, &mut worklist);
                    if !matches!(kind, CmpBranchKind::I32Imm(_, _) | CmpBranchKind::I64Imm(_, _)) {
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
                new_pool.extend_from_slice(&old_pool[r.start as usize..r.start as usize + r.len as usize]);
                ValRange { start, len: r.len }
            };
            let push_filtered = |new_pool: &mut Vec<Val>, target: BlockId, r: ValRange| -> ValRange {
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
                    ValRange { start, len: new_len }
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
                    Term::BranchIf { then_bb, then_args, else_bb, else_args, .. }
                    | Term::CmpBranch { then_bb, then_args, else_bb, else_args, .. } => {
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
                let mut new_unique: Vec<(BlockId, ValRange)> = Vec::with_capacity(td.unique_edges.len());
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
// 2. Budgeted Small-Leaf Inlining (`CONTINUE.md` §5.1.2)
// ============================================================================

/// Computes Tarjan SCCs on the defined-function call graph and returns `(in_cycle, topo_bottom_up)`.
fn call_graph_sccs(irs: &[IrFunc], n_imports: usize) -> (Vec<bool>, Vec<usize>) {
    let n = irs.len();
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut seen_stamp = vec![u32::MAX; n];
    for (bi, ir) in irs.iter().enumerate() {
        let stamp = bi as u32;
        for bb in &ir.blocks {
            for inst in &bb.insts {
                if let Op::Call { func, .. } = inst.op {
                    let f = func as usize;
                    if f >= n_imports {
                        let c = f - n_imports;
                        if c < n && seen_stamp[c] != stamp {
                            seen_stamp[c] = stamp;
                            edges[bi].push(c);
                        }
                    }
                }
            }
        }
    }

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
                    let cyclic = scc.len() > 1 || (scc.len() == 1 && edges[scc[0]].contains(&scc[0]));
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
    if caller.blocks.len().saturating_add(callee.blocks.len()).saturating_add(1) > MAX_BLOCKS_PER_FN {
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
                    needed_pool = needed_pool.saturating_add(args.len as usize).saturating_add(rets.len as usize);
                }
                _ => {}
            }
        }
        match cb.term {
            Term::Jump { args, .. } => needed_pool = needed_pool.saturating_add(args.len as usize),
            Term::BranchIf { then_args, else_args, .. } | Term::CmpBranch { then_args, else_args, .. } => {
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
        Ok(ValRange { start: start as u32, len: r.len })
    };

    // Clone br_tables with remapped targets and arg ranges.
    for td in &callee.br_tables {
        let mut targets = Vec::with_capacity(td.targets.len());
        let mut unique_edges = Vec::with_capacity(td.unique_edges.len());
        let mut edge_map: HashMap<(BlockId, u32, u32), (BlockId, ValRange)> = HashMap::new();
        let mut map_edge = |e: (BlockId, ValRange), pool: &mut Vec<Val>| -> Res<(BlockId, ValRange)> {
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
        caller.br_tables.push(BrTableData { targets, default, unique_edges });
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
                Op::Select { cond, a, b } => Op::Select { cond: map_v(cond), a: map_v(a), b: map_v(b) },
                Op::CmpSelect { kind, a, b, tv, fv } => Op::CmpSelect {
                    kind,
                    a: map_v(a),
                    b: map_v(b),
                    tv: map_v(tv),
                    fv: map_v(fv),
                },
                Op::GlobalGet(g) => Op::GlobalGet(g),
                Op::GlobalSet(g, a) => Op::GlobalSet(g, map_v(a)),
                Op::TableGet { table, idx } => Op::TableGet { table, idx: map_v(idx) },
                Op::TableSet { table, idx, val } => Op::TableSet { table, idx: map_v(idx), val: map_v(val) },
                Op::BoundsCheck { addr, end_off } => Op::BoundsCheck { addr: map_v(addr), end_off },
                Op::Load { kind, addr, offset, checked } => Op::Load { kind, addr: map_v(addr), offset, checked },
                Op::Store { kind, addr, val, offset, checked } => {
                    Op::Store { kind, addr: map_v(addr), val: map_v(val), offset, checked }
                }
                Op::MemSize => Op::MemSize,
                Op::MemGrow(a) => Op::MemGrow(map_v(a)),
                Op::MemCopy { dst, src, len } => Op::MemCopy { dst: map_v(dst), src: map_v(src), len: map_v(len) },
                Op::MemFill { dst, val, len } => Op::MemFill { dst: map_v(dst), val: map_v(val), len: map_v(len) },
                Op::MemCopyConst { dst, src, len } => Op::MemCopyConst { dst: map_v(dst), src: map_v(src), len },
                Op::MemFillConst { dst, val, len } => Op::MemFillConst { dst: map_v(dst), val: map_v(val), len },
                Op::V128Splat { lane_bytes, val } => Op::V128Splat { lane_bytes, val: map_v(val) },
                Op::I64x2ExtractLane { v, lane } => Op::I64x2ExtractLane { v: map_v(v), lane },
                Op::I64x2ReplaceLane { v, x, lane } => Op::I64x2ReplaceLane { v: map_v(v), x: map_v(x), lane },
                Op::Call { func, args, rets } => Op::Call {
                    func,
                    args: map_range(&mut caller.val_pool, &callee.val_pool, args)?,
                    rets: map_range(&mut caller.val_pool, &callee.val_pool, rets)?,
                },
                Op::CallIndirect { ci_id, idx, args, rets } => Op::CallIndirect {
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
            Term::BranchIf { cond, then_bb, then_args, else_bb, else_args } => Term::BranchIf {
                cond: map_v(cond),
                then_bb: map_b(then_bb),
                then_args: map_range(&mut caller.val_pool, &callee.val_pool, then_args)?,
                else_bb: map_b(else_bb),
                else_args: map_range(&mut caller.val_pool, &callee.val_pool, else_args)?,
            },
            Term::CmpBranch { kind, a, b, then_bb, then_args, else_bb, else_args } => Term::CmpBranch {
                kind,
                a: map_v(a),
                b: map_v(b),
                then_bb: map_b(then_bb),
                then_args: map_range(&mut caller.val_pool, &callee.val_pool, then_args)?,
                else_bb: map_b(else_bb),
                else_args: map_range(&mut caller.val_pool, &callee.val_pool, else_args)?,
            },
            Term::BrTable { idx, table_idx } => Term::BrTable { idx: map_v(idx), table_idx: base_bt + table_idx },
            Term::Return { vals } => Term::Jump {
                target: cont_bb,
                args: map_range(&mut caller.val_pool, &callee.val_pool, vals)?,
            },
            Term::Unreachable => Term::Unreachable,
        };
        caller.blocks.push(Block { params, insts, term, is_loop_header: false, preds: 0 });
    }

    caller.blocks[call_bb].term = Term::Jump { target: BlockId(base_bb), args: call_args };
    Ok(())
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
    let (in_cycle, topo_bottom_up) = call_graph_sccs(irs, n_imports);
    let mut inst_counts: Vec<usize> = irs.iter().map(count_insts).collect();
    let mut total_added_insts = 0usize;

    // Bottom-up topological order naturally handles chained small helpers (`a -> b -> c`) in one pass!
    let mut inline_steps = 0usize;
    for (step, &caller_bi) in topo_bottom_up.iter().enumerate() {
        if let Some(bg) = budget
            && (step & 63) == 0
        {
            bg.check_deadline()?;
        }
        if total_added_insts >= INLINE_MAX_MODULE_ADDED_INSTS {
            break;
        }
        let mut caller_insts = inst_counts[caller_bi];
        if caller_insts >= INLINE_MAX_CALLER_INSTS {
            continue;
        }
        let mut caller_added = 0usize;
        let mut did_inline = false;

        // Scan blocks dynamically (splitting pushes new blocks at the end).
        let mut bi = 0usize;
        while bi < irs[caller_bi].blocks.len() {
            let mut ii = 0usize;
            while ii < irs[caller_bi].blocks[bi].insts.len() {
                inline_steps += 1;
                if let Some(bg) = budget
                    && (inline_steps & 4095) == 0
                {
                    bg.check_deadline()?;
                }
                if caller_added >= INLINE_MAX_ADDED_PER_CALLER
                    || total_added_insts >= INLINE_MAX_MODULE_ADDED_INSTS
                    || caller_insts >= INLINE_MAX_CALLER_INSTS
                {
                    break;
                }
                let Op::Call { func, args, rets } = irs[caller_bi].blocks[bi].insts[ii].op else {
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
                let c_insts = inst_counts[callee_bi];
                let c_blocks = irs[callee_bi].blocks.len();
                let c_vals = irs[callee_bi].val_tys.len();
                if !irs[callee_bi].br_tables.is_empty()
                    || c_insts > INLINE_MAX_CALLEE_INSTS
                    || c_blocks > INLINE_MAX_CALLEE_BLOCKS
                    || c_vals > INLINE_MAX_CALLEE_VALS
                    || irs[caller_bi].blocks.len() + c_blocks + 1 > INLINE_MAX_CALLER_BLOCKS
                    || irs[caller_bi].val_tys.len() + c_vals > INLINE_MAX_CALLER_VALS
                {
                    ii += 1;
                    continue;
                }
                // Borrow disjoint `caller` (&mut) and `callee` (&) directly via `split_at_mut` (zero cloning!).
                let ok = if caller_bi < callee_bi {
                    let (lo, hi) = irs.split_at_mut(callee_bi);
                    clone_callee_into_caller(&mut lo[caller_bi], &hi[0], bi, ii, args, rets).is_ok()
                } else {
                    let (lo, hi) = irs.split_at_mut(caller_bi);
                    clone_callee_into_caller(&mut hi[0], &lo[callee_bi], bi, ii, args, rets).is_ok()
                };
                if ok {
                    let cost = c_insts.max(1);
                    caller_insts += cost;
                    caller_added += cost;
                    total_added_insts += cost;
                    did_inline = true;
                    // `bi` now ends with `Jump(callee_entry)`; tail moved to `cont_bb` (visited later in `while bi`).
                    break;
                }
                ii += 1;
            }
            bi += 1;
        }
        if did_inline {
            inst_counts[caller_bi] = caller_insts;
            // Only compact CFG sequentially if this caller is itself small enough to be inlined upstream;
            // all larger callers compact in parallel inside `optimize_func_stage2`!
            if caller_insts <= INLINE_MAX_CALLEE_INSTS && irs[caller_bi].blocks.len() > INLINE_MAX_CALLEE_BLOCKS {
                rebuild_cfg_and_prune(&mut irs[caller_bi], budget)?;
            }
        }
    }
    Ok(())
}

// ============================================================================
// 3. Interprocedural Effect Analysis (`CONTINUE.md` §5.1.3)
// ============================================================================

pub struct ModuleEffects {
    pub writes_mem: Vec<bool>,
    pub grows_mem: Vec<bool>,
    pub writes_all_globals: Vec<bool>,
    pub written_globals: Vec<Vec<u64>>,
    pub global_words: usize,
    pub writes_all_tables: Vec<bool>,
    pub written_tables: Vec<u64>,
}

pub fn compute_module_effects(
    irs: &[IrFunc],
    module: &X64Module,
    budget: Option<&CompileBudget>,
) -> Res<ModuleEffects> {
    if let Some(bg) = budget {
        bg.check_deadline()?;
    }
    let n_imports = module.imports.len();
    let n_total = n_imports + irs.len();
    let global_words = module.globals.len().div_ceil(64);
    let fixed_mem = module.fixed32.is_some();

    let mut writes_mem = vec![false; n_total];
    let mut grows_mem = vec![false; n_total];
    let mut writes_all_globals = vec![false; n_total];
    let mut written_globals = vec![vec![0u64; global_words]; n_total];
    let mut writes_all_tables = vec![false; n_total];
    let mut written_tables = vec![0u64; n_total];

    // Imports may invoke arbitrary host callbacks (`active_allocate`, `set_global`, memory grow).
    for fi in 0..n_imports {
        writes_mem[fi] = true;
        grows_mem[fi] = !fixed_mem;
        writes_all_globals[fi] = true;
        writes_all_tables[fi] = true;
    }

    let mut direct_callees: Vec<Vec<usize>> = vec![Vec::new(); irs.len()];
    let mut seen_callee = vec![u32::MAX; n_total];
    let mut scan_steps = 0usize;
    for (bi, ir) in irs.iter().enumerate() {
        let fi = n_imports + bi;
        let stamp = bi as u32;
        for bb in &ir.blocks {
            for inst in &bb.insts {
                scan_steps += 1;
                if let Some(bg) = budget
                    && (scan_steps & 4095) == 0
                {
                    bg.check_deadline()?;
                }
                match inst.op {
                    Op::Store { .. }
                    | Op::MemCopy { .. }
                    | Op::MemFill { .. }
                    | Op::MemCopyConst { .. }
                    | Op::MemFillConst { .. } => {
                        writes_mem[fi] = true;
                    }
                    Op::MemGrow(_) => {
                        writes_mem[fi] = true;
                        grows_mem[fi] = !fixed_mem;
                    }
                    Op::GlobalSet(g, _) => {
                        let gi = g as usize;
                        if (gi >> 6) < global_words {
                            written_globals[fi][gi >> 6] |= 1u64 << (gi & 63);
                        }
                    }
                    Op::TableSet { table, .. } => {
                        if table < 64 {
                            written_tables[fi] |= 1u64 << table;
                        } else {
                            writes_all_tables[fi] = true;
                        }
                    }
                    Op::CallIndirect { .. } => {
                        writes_mem[fi] = true;
                        grows_mem[fi] = !fixed_mem;
                        writes_all_globals[fi] = true;
                        writes_all_tables[fi] = true;
                    }
                    Op::Call { func, .. } => {
                        let callee = func as usize;
                        if callee < n_total && seen_callee[callee] != stamp {
                            seen_callee[callee] = stamp;
                            direct_callees[bi].push(callee);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let mut changed = true;
    let mut fix_steps = 0usize;
    while changed {
        changed = false;
        for (bi, callees) in direct_callees.iter().enumerate() {
            let fi = n_imports + bi;
            for &c in callees {
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

    Ok(ModuleEffects {
        writes_mem,
        grows_mem,
        writes_all_globals,
        written_globals,
        global_words,
        writes_all_tables,
        written_tables,
    })
}

// ============================================================================
// 4. Conservative LICM (`CONTINUE.md` §5.1.4)
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
        Op::I32Bin(bop, _, _) | Op::I64Bin(bop, _, _) => {
            !matches!(bop, IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU)
        }
        Op::I32BinImm(bop, _, imm) | Op::I64BinImm(bop, _, imm) => {
            !matches!(bop, IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU)
                || (imm != 0 && !(bop == IntBinOp::DivS && imm == -1))
        }
        Op::Trunc(t, _) => !t.can_trap(),
        // NEVER hoist Load, Store, BoundsCheck, TableGet, TableSet, Call, CallIndirect, MemGrow, MemCopy*, MemFill*, Trap!
        _ => false,
    }
}

/// Conservative LICM: hoists strictly non-trapping loop-invariant pure ops, uninvalidated
/// `GlobalGet(g)`, and uninvalidated `MemSize` out of natural loops into a single preheader.
pub fn run_licm(
    ir: &mut IrFunc,
    dom: &CfgDom,
    module: &X64Module,
    effects: &ModuleEffects,
    budget: Option<&CompileBudget>,
) -> Res<()> {
    let n = ir.blocks.len();
    if n <= 1 {
        return Ok(());
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
        return Ok(());
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
    loops.sort_by_key(|(_, body)| body.len());

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
                while i < ir.blocks[b as usize].insts.len() && hoisted_count < LICM_MAX_HOIST_PER_LOOP {
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
    }
    Ok(())
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
        if matches!(bop, IntBinOp::Add | IntBinOp::Mul | IntBinOp::And | IntBinOp::Or | IntBinOp::Xor) && a.0 > b.0 {
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
        Op::I32Bin(bop, a, b) if !matches!(bop, IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU) => {
            let (x, y) = comm_int(bop, a, b);
            GvnKey::I32Bin(bop, x, y)
        }
        Op::I64Bin(bop, a, b) if !matches!(bop, IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU) => {
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

enum UndoEntry {
    Expr(GvnKey, Option<Val>),
    Const32(Val, Option<i32>),
    AddConst(Val, Option<(Val, u32)>),
    Bound(Val, Option<u64>),
    CheckedMax(Val, Option<(u32, u64)>),
    Global(u32, Option<(u32, Val)>),
    MemSize(Option<(u32, Val)>),
    Table((u32, Val), Option<(u32, Val)>),
    Load((LoadKind, Val, u32), Option<(u32, Val)>),
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
    mem_max_bytes: u64,

    subst: Vec<Val>,
    any_subst: bool,
    exprs: HashMap<GvnKey, Val>,
    const32: Vec<Option<i32>>,
    add_const: Vec<Option<(Val, u32)>>,
    non_wrap_bound: Vec<Option<u64>>,
    checked_max: Vec<Option<(u32, u64)>>,
    globals: Vec<Option<(u32, Val)>>,
    mem_size: Option<(u32, Val)>,
    tables: HashMap<(u32, Val), (u32, Val)>,
    loads: HashMap<(LoadKind, Val, u32), (u32, Val)>,
    undo: Vec<UndoEntry>,

    grow_epoch: u32,
    mem_epoch: u32,
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
        let ep = self.grow_epoch;
        let slot = &mut self.checked_max[addr.0 as usize];
        if slot.is_none_or(|(ce, cm)| ce != ep || end_off > cm) {
            let prev = slot.replace((ep, end_off));
            self.undo.push(UndoEntry::CheckedMax(addr, prev));
        }
        // If total linear memory capacity is strictly < 4 GiB, any successful check proves
        // `u64::from(addr) + end_off <= mem_max_bytes`, so `addr <= mem_max_bytes - end_off`!
        if self.mem_max_bytes <= u32::MAX as u64 && end_off <= self.mem_max_bytes {
            self.tighten_bound(addr, self.mem_max_bytes - end_off);
        }
    }

    fn is_checked(&self, addr: Val, end_off: u64) -> bool {
        self.checked_max[addr.0 as usize]
            .is_some_and(|(ce, cm)| ce == self.grow_epoch && cm >= end_off)
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
                UndoEntry::Expr(k, Some(v)) => { self.exprs.insert(k, v); }
                UndoEntry::Expr(k, None) => { self.exprs.remove(&k); }
                UndoEntry::Const32(k, prev) => { self.const32[k.0 as usize] = prev; }
                UndoEntry::AddConst(k, prev) => { self.add_const[k.0 as usize] = prev; }
                UndoEntry::Bound(k, prev) => { self.non_wrap_bound[k.0 as usize] = prev; }
                UndoEntry::CheckedMax(k, prev) => { self.checked_max[k.0 as usize] = prev; }
                UndoEntry::Global(k, prev) => { self.globals[k as usize] = prev; }
                UndoEntry::MemSize(prev) => { self.mem_size = prev; }
                UndoEntry::Table(k, Some(v)) => { self.tables.insert(k, v); }
                UndoEntry::Table(k, None) => { self.tables.remove(&k); }
                UndoEntry::Load(k, Some(v)) => { self.loads.insert(k, v); }
                UndoEntry::Load(k, None) => { self.loads.remove(&k); }
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
        IntBinOp::RemS if cb != 0 => Some(if ca == i32::MIN && cb == -1 { 0 } else { ca.wrapping_rem(cb) }),
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
    let mem_max_bytes = module.memory.map_or(0u64, |(_, max_p)| max_p.saturating_mul(65_536));

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
        loads: HashMap::new(),
        undo: Vec::new(),
        grow_epoch: 1,
        mem_epoch: 1,
        glob_epoch: 1,
        tbl_epoch: 1,
    };

    // Iterative DFS over dominator tree: `(block_id, next_child_idx, undo_mark, saved_epochs)`.
    enum DfsAction {
        Enter(u32),
        Exit { mark: usize, epochs: (u32, u32, u32, u32) },
    }
    let mut stack: Vec<DfsAction> = vec![DfsAction::Enter(0)];
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
                st.mem_epoch = epochs.1;
                st.glob_epoch = epochs.2;
                st.tbl_epoch = epochs.3;
            }
            DfsAction::Enter(bid) => {
                let mark = st.undo.len();
                let saved_epochs = (st.grow_epoch, st.mem_epoch, st.glob_epoch, st.tbl_epoch);
                stack.push(DfsAction::Exit { mark, epochs: saved_epochs });

                // At multi-predecessor joins/loops, conservatively bump mutable epochs (read-only facts persist!).
                if dom.preds[bid as usize].len() > 1 {
                    if st.fn_may_grow {
                        st.grow_epoch = st.grow_epoch.wrapping_add(1);
                    }
                    if st.fn_writes_mem {
                        st.mem_epoch = st.mem_epoch.wrapping_add(1);
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
                                inst.op = Op::MemCopyConst { dst, src, len: c as u32 };
                            }
                        }
                        Op::MemFill { dst, val, len } => {
                            if let Some(c) = st.const32[len.0 as usize]
                                && (c as u32) <= 64
                            {
                                inst.op = Op::MemFillConst { dst, val, len: c as u32 };
                            }
                        }
                        _ => {}
                    }

                    // Wrap-safe addressing-mode folding + cross-block dominator BCE + Load/Global/Table/MemSize/Expr GVN.
                    match &mut inst.op {
                        Op::BoundsCheck { addr, end_off } => {
                            if st.is_checked(*addr, *end_off) {
                                continue;
                            }
                            st.record_checked(*addr, *end_off);
                            out_insts.push(inst);
                            continue;
                        }
                        Op::Load { kind, addr, offset, checked } => {
                            let (na, noff) = st.try_fold_addr(*addr, *offset);
                            *addr = na;
                            *offset = noff;
                            let end = u64::from(*offset) + kind.bytes();
                            if *checked || st.is_checked(*addr, end) {
                                *checked = true;
                            }
                            st.record_checked(*addr, end);
                            let lkey = (*kind, *addr, *offset);
                            let expected_ep = if st.fn_writes_mem { st.mem_epoch } else { 0 };
                            if let Some(res) = inst.res {
                                if let Some(&(ep, prev_v)) = st.loads.get(&lkey)
                                    && ep == expected_ep
                                {
                                    // Dominated by an identical load with no intervening memory write -> CSE!
                                    st.subst[res.0 as usize] = prev_v;
                                    st.any_subst = true;
                                    continue;
                                }
                                let prev = st.loads.insert(lkey, (expected_ep, res));
                                st.undo.push(UndoEntry::Load(lkey, prev));
                                match *kind {
                                    LoadKind::I32Load8U => st.tighten_bound(res, 255),
                                    LoadKind::I32Load16U => st.tighten_bound(res, 65_535),
                                    _ => {}
                                }
                            }
                            out_insts.push(inst);
                            continue;
                        }
                        Op::Store { kind, addr, offset, checked, .. } => {
                            let (na, noff) = st.try_fold_addr(*addr, *offset);
                            *addr = na;
                            *offset = noff;
                            let end = u64::from(*offset) + kind.bytes();
                            if *checked || st.is_checked(*addr, end) {
                                *checked = true;
                            }
                            st.record_checked(*addr, end);
                            st.mem_epoch = st.mem_epoch.wrapping_add(1);
                            out_insts.push(inst);
                            continue;
                        }
                        Op::MemCopy { .. } | Op::MemFill { .. } | Op::MemCopyConst { .. } | Op::MemFillConst { .. } => {
                            st.mem_epoch = st.mem_epoch.wrapping_add(1);
                            out_insts.push(inst);
                            continue;
                        }
                        Op::MemGrow(_) => {
                            st.mem_epoch = st.mem_epoch.wrapping_add(1);
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
                                    let ep = if st.fn_writes_global(*g) { st.glob_epoch } else { 0 };
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
                                let ep = if st.fn_writes_global(*g) { st.glob_epoch } else { 0 };
                                let prev = st.globals[gi].replace((ep, *v));
                                st.undo.push(UndoEntry::Global(*g, prev));
                            }
                            out_insts.push(inst);
                            continue;
                        }
                        Op::TableGet { table, idx } => {
                            if let Some(res) = inst.res {
                                let ep = if st.fn_writes_table(*table) { st.tbl_epoch } else { 0 };
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
                                st.mem_epoch = st.mem_epoch.wrapping_add(1);
                            }
                            if st.effects.grows_mem[f] {
                                st.grow_epoch = st.grow_epoch.wrapping_add(1);
                            }
                            if st.effects.writes_all_globals[f]
                                || (0..st.effects.global_words).any(|w| st.effects.written_globals[f][w] != 0)
                            {
                                st.glob_epoch = st.glob_epoch.wrapping_add(1);
                            }
                            if st.effects.writes_all_tables[f] || st.effects.written_tables[f] != 0 {
                                st.tbl_epoch = st.tbl_epoch.wrapping_add(1);
                            }
                            out_insts.push(inst);
                            continue;
                        }
                        Op::CallIndirect { .. } => {
                            st.mem_epoch = st.mem_epoch.wrapping_add(1);
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
                                if let (Some(ca), Some(cb)) = (st.const32[a.0 as usize], st.const32[b.0 as usize])
                                    && let Some(v) = fold_i32_bin_const(bop, ca, cb)
                                {
                                    inst.op = Op::ConstI32(v);
                                }
                            }
                            Op::I32CmpImm(cop, a, imm) => {
                                if let Some(ca) = st.const32[a.0 as usize] {
                                    inst.op = Op::ConstI32(i32::from(fold_i32_cmp_const(cop, ca, imm)));
                                }
                            }
                            Op::I32Cmp(cop, a, b) => {
                                if let (Some(ca), Some(cb)) = (st.const32[a.0 as usize], st.const32[b.0 as usize]) {
                                    inst.op = Op::ConstI32(i32::from(fold_i32_cmp_const(cop, ca, cb)));
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
                                    CmpBranchKind::I32Imm(cop, imm) => {
                                        st.const32[a.0 as usize].map(|ca| fold_i32_cmp_const(cop, ca, imm))
                                    }
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
                            }
                            Op::I32BinImm(IntBinOp::And, _, m) if m >= 0 => {
                                st.tighten_bound(res, u64::from(m as u32));
                            }
                            Op::I32BinImm(IntBinOp::ShrU, _, sh) if (1..32).contains(&sh) => {
                                st.tighten_bound(res, u64::from(u32::MAX >> (sh as u32)));
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
                    Term::BranchIf { cond, then_bb, then_args, else_bb, else_args } => {
                        if let Some(c) = st.const32[cond.0 as usize] {
                            bb.term = if c != 0 {
                                Term::Jump { target: then_bb, args: then_args }
                            } else {
                                Term::Jump { target: else_bb, args: else_args }
                            };
                            folded_any_branch = true;
                        }
                    }
                    Term::CmpBranch { kind, a, b, then_bb, then_args, else_bb, else_args } => {
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
                                Term::Jump { target: then_bb, args: then_args }
                            } else {
                                Term::Jump { target: else_bb, args: else_args }
                            };
                            folded_any_branch = true;
                        }
                    }
                    _ => {}
                }

                // Push dom_children in reverse so they visit in forward order.
                for &child in dom.dom_children[bid as usize].iter().rev() {
                    stack.push(DfsAction::Enter(child));
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
                    IntBinOp::Add | IntBinOp::Sub | IntBinOp::And | IntBinOp::Or | IntBinOp::Xor | IntBinOp::Shl | IntBinOp::ShrS | IntBinOp::ShrU,
                    _,
                    _
                )
                | Op::I64BinImm(
                    IntBinOp::Add | IntBinOp::Sub | IntBinOp::And | IntBinOp::Or | IntBinOp::Xor | IntBinOp::Shl | IntBinOp::ShrS | IntBinOp::ShrU,
                    _,
                    _
                )
                | Op::I32Bin(
                    IntBinOp::Add | IntBinOp::Sub | IntBinOp::And | IntBinOp::Or | IntBinOp::Xor | IntBinOp::Shl | IntBinOp::ShrS | IntBinOp::ShrU,
                    _,
                    _
                )
                | Op::I64Bin(
                    IntBinOp::Add | IntBinOp::Sub | IntBinOp::And | IntBinOp::Or | IntBinOp::Xor | IntBinOp::Shl | IntBinOp::ShrS | IntBinOp::ShrU,
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
            Term::BranchIf { then_bb, else_bb, .. } | Term::CmpBranch { then_bb, else_bb, .. } => {
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
            Term::BranchIf { cond, then_bb, then_args, else_bb, else_args } => {
                (Err(cond), then_bb, then_args, else_bb, else_args)
            }
            Term::CmpBranch { kind, a, b, then_bb, then_args, else_bb, else_args } => {
                (Ok((kind, a, b)), then_bb, then_args, else_bb, else_args)
            }
            _ => continue,
        };
        if then_args.len != 0 || else_args.len != 0 {
            continue;
        }
        let ti = then_bb.0 as usize;
        let ei = else_bb.0 as usize;
        if ti == ei || ti == bi || ei == bi || ti == 0 || ei == 0 || dead_arm[ti] || dead_arm[ei] {
            continue;
        }
        if edge_in[ti] != 1 || edge_in[ei] != 1 {
            continue;
        }
        if !ir.blocks[ti].params.is_empty() || !ir.blocks[ei].params.is_empty() {
            continue;
        }
        if ir.blocks[ti].insts.len() > 2 || ir.blocks[ei].insts.len() > 2 {
            continue;
        }
        if !ir.blocks[ti].insts.iter().all(|ins| is_trivial_pure(&ins.op))
            || !ir.blocks[ei].insts.iter().all(|ins| is_trivial_pure(&ins.op))
        {
            continue;
        }
        let (Term::Jump { target: tm, args: ta }, Term::Jump { target: em, args: ea }) =
            (ir.blocks[ti].term, ir.blocks[ei].term)
        else {
            continue;
        };
        if tm != em || ta.len != ea.len || tm.0 as usize == ti || tm.0 as usize == ei {
            continue;
        }
        // Check differing positions: allow up to 2 GP values (lowered to branchless `cmovcc`).
        let t_slice = ir.pool_slice(ta);
        let e_slice = ir.pool_slice(ea);
        let mut diffs = 0usize;
        let mut all_gp = true;
        for (&tv, &ev) in t_slice.iter().zip(e_slice.iter()) {
            if tv != ev {
                diffs += 1;
                if !ir.val_tys[tv.0 as usize].is_gp() || ir.val_tys[tv.0 as usize] != ir.val_tys[ev.0 as usize] {
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

        // Speculatively hoist both trivial arms into `bi` and emit `CmpSelect`/`Select` per diff!
        let t_vals: Vec<Val> = t_slice.to_vec();
        let e_vals: Vec<Val> = e_slice.to_vec();
        let t_insts = std::mem::take(&mut ir.blocks[ti].insts);
        let e_insts = std::mem::take(&mut ir.blocks[ei].insts);
        ir.blocks[ti].term = Term::Unreachable;
        ir.blocks[ei].term = Term::Unreachable;
        dead_arm[ti] = true;
        dead_arm[ei] = true;

        ir.blocks[bi].insts.extend(t_insts);
        ir.blocks[bi].insts.extend(e_insts);
        let mut merged_args = Vec::with_capacity(t_vals.len());
        for (tv, ev) in t_vals.into_iter().zip(e_vals) {
            if tv == ev {
                merged_args.push(tv);
            } else {
                let ty = ir.val_tys[tv.0 as usize];
                let sel_v = ir.push_val(ty);
                let sel_op = match cmp_info {
                    Ok((kind, a, b)) => Op::CmpSelect { kind, a, b, tv, fv: ev },
                    Err(cond) => Op::Select { cond, a: tv, b: ev },
                };
                ir.blocks[bi].insts.push(Inst { res: Some(sel_v), op: sel_op });
                merged_args.push(sel_v);
            }
        }
        let range = match ir.push_pool(&merged_args) {
            Ok(r) => r,
            Err(_) => continue,
        };
        ir.blocks[bi].term = Term::Jump { target: tm, args: range };
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
    // 3. LICM + Dominator GVN/BCE/post-inlining constant folding + peephole fusion/DCE.
    run_licm(ir, &dom, module, effects, budget)?;
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
            matches!(bop, IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU)
        }
        Op::I32BinImm(bop, _, imm) | Op::I64BinImm(bop, _, imm) => {
            matches!(bop, IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU)
                && (imm == 0 || (bop == IntBinOp::DivS && imm == -1))
        }
        Op::Trunc(t, _) => t.can_trap(),
        Op::Load { checked: false, .. }
        | Op::Store { checked: false, .. }
        | Op::BoundsCheck { .. }
        | Op::TableGet { .. }
        | Op::TableSet { .. }
        | Op::MemCopy { .. }
        | Op::MemFill { .. }
        | Op::MemCopyConst { .. }
        | Op::MemFillConst { .. }
        | Op::CallIndirect { .. }
        | Op::Trap(_) => true,
        _ => false,
    }
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
    if let Some(bg) = budget {
        bg.check_deadline()?;
    }
    let n_imports = module.imports.len();
    let n_defs = irs.len();
    let n_total = n_imports + n_defs;
    let (in_cycle, topo_bottom_up) = call_graph_sccs(irs, n_imports);

    // ---- Step 8a: Native-Frame Stack-Check Elision (§5.1.8a) ----
    let mut cannot_elide = vec![false; n_defs];
    // Rule 1: Exported, `start`, or referenced in ANY `call_indirect` table MUST retain stack check!
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

    let mut direct_def_callees: Vec<Vec<usize>> = vec![Vec::new(); n_defs];
    let mut seen_callee = vec![u32::MAX; n_defs];
    let mut calls_import_or_indirect = vec![false; n_defs];
    let mut max_helper_bytes = vec![0i32; n_defs];
    let mut local_may_trap = vec![false; n_defs];
    let mut scan_steps = 0usize;

    for (bi, ir) in irs.iter().enumerate() {
        let stamp = bi as u32;
        if in_cycle[bi] {
            cannot_elide[bi] = true;
        }
        for bb in &ir.blocks {
            if matches!(bb.term, Term::Unreachable) {
                local_may_trap[bi] = true;
            }
            for inst in &bb.insts {
                scan_steps += 1;
                if let Some(bg) = budget
                    && (scan_steps & 4095) == 0
                {
                    bg.check_deadline()?;
                }
                if op_may_trap_locally(&inst.op) {
                    local_may_trap[bi] = true;
                }
                match inst.op {
                    Op::Call { func, .. } => {
                        let f = func as usize;
                        if f < n_imports {
                            calls_import_or_indirect[bi] = true;
                            cannot_elide[bi] = true;
                        } else if f - n_imports < n_defs {
                            let c = f - n_imports;
                            if seen_callee[c] != stamp {
                                seen_callee[c] = stamp;
                                direct_def_callees[bi].push(c);
                            }
                        }
                    }
                    Op::CallIndirect { .. } => {
                        calls_import_or_indirect[bi] = true;
                        cannot_elide[bi] = true;
                    }
                    Op::Trunc(_, _) | Op::MemGrow(_) | Op::MemCopy { .. } | Op::MemFill { .. } => {
                        max_helper_bytes[bi] = max_helper_bytes[bi].max(PURE_HELPER_STACK_BYTES);
                    }
                    _ => {}
                }
            }
        }
    }

    // Bottom-up exact worst-case transitive native stack use (`own_bytes = frame_size + 8*saved_gp + 8`).
    let mut elided = vec![false; n_defs];
    let mut subtree_bytes = vec![0i32; n_defs];
    for &bi in &topo_bottom_up {
        let own_bytes = allocs[bi]
            .frame_size
            .saturating_add((allocs[bi].used_callee_gp.len() as i32) * 8)
            .saturating_add(8); // return address pushed by `call`
        if cannot_elide[bi] || calls_import_or_indirect[bi] {
            continue;
        }
        // Rule 2: Calls ONLY other elided callees or pure helpers (NEVER a retaining callee!).
        if direct_def_callees[bi].iter().any(|&c| !elided[c]) {
            continue;
        }
        let max_callee = direct_def_callees[bi]
            .iter()
            .map(|&c| subtree_bytes[c])
            .max()
            .unwrap_or(0)
            .max(max_helper_bytes[bi]);
        let total = own_bytes.saturating_add(max_callee);
        // Rule 3: Exact subtree native stack use <= 8 KiB.
        if total <= ELIDE_MAX_SUBTREE_STACK_BYTES {
            elided[bi] = true;
            subtree_bytes[bi] = total;
        }
    }

    // Apply Step 8a decisions & Rule 4 (every retaining caller reserves `max_elided_subtree_bytes`).
    for bi in 0..n_defs {
        if elided[bi] {
            irs[bi].needs_stack_check = false;
            irs[bi].elided_callee_stack_bytes = 0;
        } else {
            irs[bi].needs_stack_check = true;
            let reserve = direct_def_callees[bi]
                .iter()
                .filter(|&&c| elided[c])
                .map(|&c| subtree_bytes[c])
                .max()
                .unwrap_or(0);
            irs[bi].elided_callee_stack_bytes = reserve;
        }
    }

    // ---- Step 8b: Direct-Call Trap-Check Elision (strictly AFTER Step 8a!) ----
    let mut fn_can_trap = vec![true; n_total];
    for bi in 0..n_defs {
        let fi = n_imports + bi;
        // Qualifies only if `!needs_stack_check` AND no local traps AND no imports/indirect calls!
        fn_can_trap[fi] = irs[bi].needs_stack_check || local_may_trap[bi] || calls_import_or_indirect[bi];
    }
    let mut changed = true;
    let mut fix_steps = 0usize;
    while changed {
        changed = false;
        for (bi, callees) in direct_def_callees.iter().enumerate() {
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
            if callees.iter().any(|&c| fn_can_trap[n_imports + c]) {
                fn_can_trap[fi] = true;
                changed = true;
            }
        }
    }
    Ok(fn_can_trap)
}
