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

use std::collections::{HashMap as StdHashMap, HashSet as StdHashSet};
use std::hash::{BuildHasher, Hasher};

#[derive(Default)]
struct FxHasher(u64);

impl Hasher for FxHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, mut bytes: &[u8]) {
        const K: u64 = 0x517cc1b727220a95;
        while bytes.len() >= 8 {
            let v = u64::from_ne_bytes(bytes[..8].try_into().unwrap());
            self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(K);
            bytes = &bytes[8..];
        }
        if bytes.len() >= 4 {
            let v = u32::from_ne_bytes(bytes[..4].try_into().unwrap()) as u64;
            self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(K);
            bytes = &bytes[4..];
        }
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ u64::from(b)).wrapping_mul(K);
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.0 = (self.0.rotate_left(5) ^ u64::from(i)).wrapping_mul(0x517cc1b727220a95);
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.0 = (self.0.rotate_left(5) ^ u64::from(i)).wrapping_mul(0x517cc1b727220a95);
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.0 = (self.0.rotate_left(5) ^ u64::from(i)).wrapping_mul(0x517cc1b727220a95);
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x517cc1b727220a95);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.write_u64(i as u64);
    }
}

#[derive(Default, Clone, Copy)]
struct FxBuildHasher;

impl BuildHasher for FxBuildHasher {
    type Hasher = FxHasher;

    #[inline]
    fn build_hasher(&self) -> FxHasher {
        FxHasher(0)
    }
}

type HashMap<K, V> = StdHashMap<K, V, FxBuildHasher>;
type HashSet<K> = StdHashSet<K, FxBuildHasher>;

#[inline]
fn fx_map<K, V>() -> HashMap<K, V> {
    StdHashMap::with_hasher(FxBuildHasher)
}

#[inline]
fn fx_map_cap<K, V>(cap: usize) -> HashMap<K, V> {
    StdHashMap::with_capacity_and_hasher(cap, FxBuildHasher)
}

#[inline]
fn fx_set<K>() -> HashSet<K> {
    StdHashSet::with_hasher(FxBuildHasher)
}

use super::ir::{
    Block, BlockId, BrTableData, CmpBranchKind, CmpOp, FloatBinOp, FloatUnOp, Inst, IntBinOp,
    IrFunc, LoadKind, Op, OptIrScratch, SimdLaneKind, SimdSplatKind, StoreKind, Term, TruncOp, Ty,
    Val, ValRange, optimize_ir_with_budget_and_scratch,
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
        if n == 1 {
            let mut s0 = Vec::new();
            let mut p0 = Vec::new();
            let mut has_self = false;
            ir.blocks[0].term.for_each_succ(&ir.br_tables, |s| {
                if s.0 == 0 && !has_self {
                    has_self = true;
                    s0.push(0);
                    p0.push(0);
                }
            });
            return Ok(CfgDom {
                rpo: vec![0],
                rpo_pos: vec![0],
                preds: vec![p0],
                succs: vec![s0],
                idom: vec![0],
                dom_children: vec![Vec::new()],
            });
        }
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
        | Op::V128ExtractLane { v: a, .. }
        | Op::V128Un { a, .. }
        | Op::V128ToI32 { a, .. } => r(a),
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
        | Op::V128LoadLane {
            addr: a, vec: b, ..
        }
        | Op::V128StoreLane {
            addr: a, vec: b, ..
        }
        | Op::V128ReplaceLane { v: a, x: b, .. }
        | Op::V128Shuffle { a, b, .. }
        | Op::V128Bin { a, b, .. }
        | Op::V128Shift { v: a, shift: b, .. }
        | Op::MemCopyConst { dst: a, src: b, .. }
        | Op::MemFillConst { dst: a, val: b, .. }
        | Op::MemByteSum {
            base: a, count: b, ..
        } => {
            r(a);
            r(b);
        }
        Op::V128Ternary { a, b, c, .. } => {
            r(a);
            r(b);
            r(c);
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
    if n == 1 {
        let mut self_edges = 0u32;
        ir.blocks[0].term.for_each_succ(&ir.br_tables, |s| {
            if s.0 == 0 {
                self_edges = self_edges.saturating_add(1);
            }
        });
        ir.blocks[0].preds = self_edges;
        ir.blocks[0].is_loop_header = self_edges > 0;
        let dom = CfgDom::compute_with_budget(ir, budget)?;
        return Ok(dom);
    }
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
        let has_non_entry_params = ir.blocks.iter().skip(1).any(|b| !b.params.is_empty());
        let mut any_subst = false;
        if has_non_entry_params {
            let mut subst: Vec<Val> = (0..n_vals as u32).map(Val).collect();
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
        let mut inst_base = Vec::with_capacity(n_blocks);
        let mut total_insts = 0usize;
        for bb in &ir.blocks {
            inst_base.push(total_insts);
            total_insts += bb.insts.len();
        }
        let mut live_inst = vec![false; total_insts];
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
            let base = inst_base[bi];
            for (ii, inst) in bb.insts.iter().enumerate() {
                if inst.op.has_side_effect() {
                    live_inst[base + ii] = true;
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
                    let flat = inst_base[bu] + iu;
                    if !live_inst[flat] {
                        live_inst[flat] = true;
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
            let base = inst_base[bi];
            if live_inst[base..base + bb.insts.len()].iter().any(|&k| !k) {
                drop_any_inst = true;
            }
            if bi > 0 && bb.params.iter().any(|p| !live_val[p.0 as usize]) {
                drop_any_param = true;
            }
        }

        if drop_any_inst {
            for (bi, bb) in ir.blocks.iter_mut().enumerate() {
                let base = inst_base[bi];
                let slice = &live_inst[base..base + bb.insts.len()];
                if slice.iter().any(|&k| !k) {
                    let mut idx = 0usize;
                    bb.insts.retain(|_| {
                        let keep = slice[idx];
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
                let mut edge_map: HashMap<(BlockId, u32, u32), ValRange> = fx_map();
                let mut seen_new: HashSet<(BlockId, u32, u32)> = fx_set();
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
        let mut self_edges = 0u32;
        if let Some(b0) = ir.blocks.first_mut() {
            b0.term.for_each_succ(&ir.br_tables, |s| {
                if s.0 == 0 {
                    self_edges = self_edges.saturating_add(1);
                }
            });
            b0.preds = self_edges;
            b0.is_loop_header = self_edges > 0;
        }
        ir.early_pruned = 2;
        return Ok(());
    }
    let blocks_before = ir.blocks.len();
    let dom = rebuild_cfg_and_prune(ir, budget)?;
    let pruned = prune_block_params(ir, &dom, budget)?;
    ir.early_pruned = if ir.blocks.len() != blocks_before || pruned {
        1
    } else {
        2
    };
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
        let mut edge_map: HashMap<(BlockId, u32, u32), (BlockId, ValRange)> = fx_map();
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
                Op::BoundsCheck {
                    addr,
                    end_off,
                    for_bulk,
                } => Op::BoundsCheck {
                    addr: map_v(addr),
                    end_off,
                    for_bulk,
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
                Op::V128LoadLane {
                    width,
                    addr,
                    vec,
                    offset,
                    lane,
                    checked,
                } => Op::V128LoadLane {
                    width,
                    addr: map_v(addr),
                    vec: map_v(vec),
                    offset,
                    lane,
                    checked,
                },
                Op::V128StoreLane {
                    width,
                    addr,
                    vec,
                    offset,
                    lane,
                    checked,
                } => Op::V128StoreLane {
                    width,
                    addr: map_v(addr),
                    vec: map_v(vec),
                    offset,
                    lane,
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
                Op::MemByteSum {
                    base,
                    count,
                    offset,
                } => Op::MemByteSum {
                    base: map_v(base),
                    count: map_v(count),
                    offset,
                },
                Op::V128Splat { kind, val } => Op::V128Splat {
                    kind,
                    val: map_v(val),
                },
                Op::V128ExtractLane { kind, v, lane } => Op::V128ExtractLane {
                    kind,
                    v: map_v(v),
                    lane,
                },
                Op::V128ReplaceLane { kind, v, x, lane } => Op::V128ReplaceLane {
                    kind,
                    v: map_v(v),
                    x: map_v(x),
                    lane,
                },
                Op::V128Shuffle { a, b, lanes_idx } => Op::V128Shuffle {
                    a: map_v(a),
                    b: map_v(b),
                    lanes_idx: base_v128 + lanes_idx,
                },
                Op::V128Un { op, a } => Op::V128Un { op, a: map_v(a) },
                Op::V128Bin { op, a, b } => Op::V128Bin {
                    op,
                    a: map_v(a),
                    b: map_v(b),
                },
                Op::V128Ternary { op, a, b, c } => Op::V128Ternary {
                    op,
                    a: map_v(a),
                    b: map_v(b),
                    c: map_v(c),
                },
                Op::V128ToI32 { op, a } => Op::V128ToI32 { op, a: map_v(a) },
                Op::V128Shift { op, v, shift } => Op::V128Shift {
                    op,
                    v: map_v(v),
                    shift: map_v(shift),
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
    caller.early_pruned = 0;
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
    inline_small_helpers_subset_parallel(
        irs,
        in_cycle,
        topo_bottom_up,
        total_call_counts,
        module,
        initial_added_insts,
        mode,
        1,
        budget,
    )
}

struct CallerInlineOutcome {
    caller_insts: usize,
    caller_added: usize,
    did_inline: bool,
    hit_module_cap: bool,
    inlined: Vec<u32>,
    track_ok: bool,
    became_inlinable: bool,
}

#[allow(clippy::too_many_arguments)]
fn inline_single_caller(
    caller_ir: &mut IrFunc,
    caller_bi: usize,
    caller_init_insts: usize,
    irs_ro: &[Option<IrFunc>],
    inst_counts: &[usize],
    in_cycle: &[bool],
    ext_ref: &[bool],
    total_call_counts: &[u32],
    inlined_ro: &[Vec<u32>],
    n_imports: usize,
    n_defs: usize,
    max_total_add: usize,
    mut track_ok: bool,
    warm: bool,
    budget: Option<&CompileBudget>,
) -> Res<Option<CallerInlineOutcome>> {
    let mut caller_insts = caller_init_insts;
    let mut caller_in_loop: Option<Vec<bool>> = None;
    let mut caller_added = 0usize;
    let mut did_inline = false;
    let mut hit_module_cap = false;
    let mut local_inlined: Vec<u32> = Vec::new();
    let mut inline_steps = 0usize;

    let mut bi = 0usize;
    while bi < caller_ir.blocks.len() {
        let mut ii = 0usize;
        while ii < caller_ir.blocks[bi].insts.len() {
            inline_steps += 1;
            if let Some(bg) = budget
                && (inline_steps & 4095) == 0
            {
                bg.check_deadline()?;
            }
            if caller_added >= max_total_add {
                hit_module_cap = true;
                break;
            }
            if caller_added >= INLINE_MAX_ADDED_PER_CALLER
                || caller_insts >= INLINE_MAX_CALLER_INSTS
            {
                break;
            }
            let Op::Call { func, args, rets } = caller_ir.blocks[bi].insts[ii].op else {
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
            let Some(callee_ir) = irs_ro[callee_bi].as_ref() else {
                ii += 1;
                continue;
            };
            let c_insts = inst_counts[callee_bi];
            let c_blocks = callee_ir.blocks.len();
            let c_vals = callee_ir.val_tys.len();
            let is_single_site =
                !ext_ref[callee_bi] && total_call_counts.get(callee_bi).copied().unwrap_or(0) == 1;
            let block_in_loop = if is_single_site {
                false
            } else {
                if caller_in_loop.is_none() {
                    let has_loop = caller_ir.blocks.iter().any(|b| b.is_loop_header);
                    let v = if !has_loop {
                        Vec::new()
                    } else {
                        let Some(v) = try_compute_in_loop_blocks(caller_ir) else {
                            if warm {
                                return Ok(None);
                            }
                            return Err(
                                "x64: out of memory computing loop blocks during inlining".into()
                            );
                        };
                        v
                    };
                    caller_in_loop = Some(v);
                }
                caller_in_loop
                    .as_ref()
                    .and_then(|v| v.get(bi).copied())
                    .unwrap_or(false)
            };
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
            if caller_in_loop.is_none() {
                let v = if caller_ir.blocks.iter().any(|b| b.is_loop_header) {
                    let Some(v) = try_compute_in_loop_blocks(caller_ir) else {
                        if warm {
                            return Ok(None);
                        }
                        return Err(
                            "x64: out of memory computing loop blocks during inlining".into()
                        );
                    };
                    v
                } else {
                    Vec::new()
                };
                caller_in_loop = Some(v);
            }
            if let Some(ref mut cil) = caller_in_loop {
                let needed_blocks = caller_ir.blocks.len() + c_blocks + 1;
                let extra_loop_slots = needed_blocks.saturating_sub(cil.len());
                if cil.try_reserve(extra_loop_slots).is_err() {
                    if warm {
                        return Ok(None);
                    }
                    return Err("x64: out of memory extending loop blocks during inlining".into());
                }
            }
            let ok = clone_callee_into_caller(caller_ir, callee_ir, bi, ii, args, rets).is_ok();
            if ok {
                let new_blocks_len = caller_ir.blocks.len();
                if let Some(ref mut cil) = caller_in_loop {
                    cil.resize(new_blocks_len, block_in_loop);
                }
                let cost = c_insts.max(1);
                caller_insts += cost;
                caller_added += cost;
                did_inline = true;
                if track_ok {
                    let src = &inlined_ro[callee_bi];
                    if local_inlined.try_reserve(1 + src.len()).is_ok() {
                        local_inlined.push(callee_bi as u32);
                        local_inlined.extend_from_slice(src);
                    } else {
                        track_ok = false;
                        local_inlined.clear();
                    }
                }
                break;
            }
            ii += 1;
        }
        bi += 1;
    }

    let mut became_inlinable = false;
    if did_inline {
        if track_ok {
            local_inlined.sort_unstable();
            local_inlined.dedup();
        }
        if caller_insts <= INLINE_MAX_SINGLE_CALLEE_INSTS
            && caller_ir.blocks.len() > INLINE_MAX_SINGLE_CALLEE_BLOCKS
        {
            rebuild_cfg_and_prune(caller_ir, budget)?;
        }
        if track_ok && is_inlinable_ir(caller_ir) {
            became_inlinable = true;
        }
    }
    Ok(Some(CallerInlineOutcome {
        caller_insts,
        caller_added,
        did_inline,
        hit_module_cap,
        inlined: local_inlined,
        track_ok,
        became_inlinable,
    }))
}

/// Parallel call-graph-stratum bottom-up small-helper inlining.
/// Guarantees bit-for-bit identical IR and `InlineTrack` to sequential `topo_bottom_up` inlining
/// by processing callers in dependency strata and chunking parallel strata strictly within the
/// worst-case remaining `INLINE_MAX_MODULE_ADDED_INSTS` headroom.
#[allow(clippy::too_many_arguments)]
pub fn inline_small_helpers_subset_parallel(
    irs: &mut [Option<IrFunc>],
    in_cycle: &[bool],
    topo_bottom_up: &[usize],
    total_call_counts: &[u32],
    module: &X64Module,
    initial_added_insts: usize,
    mode: InlineMode,
    n_workers: usize,
    budget: Option<&CompileBudget>,
) -> Res<Option<InlineTrack>> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
                was_inlinable[bi] = is_inlinable_ir(ir);
            }
        }
    }
    let mut total_added_insts = initial_added_insts;
    let mut hit_module_cap = total_added_insts >= INLINE_MAX_MODULE_ADDED_INSTS;
    if hit_module_cap {
        return Ok(Some(InlineTrack {
            inlined_callees,
            added_insts,
            was_inlinable,
            hit_module_cap: true,
            tracking_complete: track_ok,
        }));
    }

    let max_worst_per_caller = INLINE_MAX_ADDED_PER_CALLER + INLINE_MAX_SINGLE_CALLEE_INSTS;
    let try_build_strata = || -> Option<Vec<Vec<usize>>> {
        let mut stratum = Vec::new();
        stratum.try_reserve_exact(n_defs).ok()?;
        stratum.resize(n_defs, 0u32);
        let mut max_s = 0u32;
        for &bi in topo_bottom_up {
            let Some(ir) = irs[bi].as_ref() else {
                continue;
            };
            let mut s = 0u32;
            for bb in &ir.blocks {
                for inst in &bb.insts {
                    if let Op::Call { func, .. } = inst.op {
                        let f = func as usize;
                        if f >= n_imports {
                            let c = f - n_imports;
                            if c != bi && c < n_defs && !in_cycle[c] && irs[c].is_some() {
                                s = s.max(stratum[c].saturating_add(1));
                            }
                        }
                    }
                }
            }
            stratum[bi] = s;
            max_s = max_s.max(s);
        }
        let n_strata = (max_s as usize).checked_add(1)?;
        let mut buckets: Vec<Vec<usize>> = Vec::new();
        buckets.try_reserve_exact(n_strata).ok()?;
        for _ in 0..n_strata {
            buckets.push(Vec::new());
        }
        for &bi in topo_bottom_up {
            if irs[bi].is_some() && inst_counts[bi] < INLINE_MAX_CALLER_INSTS {
                let b = &mut buckets[stratum[bi] as usize];
                if b.len() == b.capacity() {
                    b.try_reserve(1).ok()?;
                }
                b.push(bi);
            }
        }
        Some(buckets)
    };

    let parallel_strata = if n_workers > 1 {
        try_build_strata()
    } else {
        None
    };

    if let Some(strata) = parallel_strata {
        let mut fell_back_to_seq = false;
        'strata: for bucket in &strata {
            if let Some(bg) = budget {
                bg.check_deadline()?;
            }
            if bucket.is_empty() {
                continue;
            }
            let mut off = 0usize;
            while off < bucket.len() {
                if total_added_insts >= INLINE_MAX_MODULE_ADDED_INSTS {
                    hit_module_cap = true;
                    break 'strata;
                }
                let rem_cap = INLINE_MAX_MODULE_ADDED_INSTS - total_added_insts;
                let safe_chunk = rem_cap / max_worst_per_caller;
                let rem_items = bucket.len() - off;
                if rem_items >= 8 && safe_chunk >= 8 {
                    let take = rem_items.min(safe_chunk);
                    let chunk = &bucket[off..off + take];
                    let mut extracted: Vec<(usize, IrFunc)> = Vec::new();
                    if extracted.try_reserve_exact(take).is_err() {
                        if warm {
                            return Ok(None);
                        }
                        fell_back_to_seq = true;
                        break 'strata;
                    }
                    for &bi in chunk {
                        extracted.push((bi, irs[bi].take().unwrap()));
                    }
                    let mut out_slots: Vec<Option<(usize, IrFunc, CallerInlineOutcome)>> =
                        Vec::new();
                    if out_slots.try_reserve_exact(take).is_err() {
                        for (bi, ir) in extracted {
                            irs[bi] = Some(ir);
                        }
                        if warm {
                            return Ok(None);
                        }
                        fell_back_to_seq = true;
                        break 'strata;
                    }
                    for _ in 0..take {
                        out_slots.push(None);
                    }
                    let irs_ro: &[Option<IrFunc>] = irs;
                    let inst_counts_ro: &[usize] = &inst_counts;
                    let inlined_ro: &[Vec<u32>] = &inlined_callees;
                    let ext_ref_ro: &[bool] = &ext_ref;
                    let work = Mutex::new(extracted);
                    let results = Mutex::new(out_slots);
                    let next_out = AtomicUsize::new(0);
                    let failed = AtomicBool::new(false);
                    let oom_warm = AtomicBool::new(false);
                    let first_err: Mutex<Option<(usize, String)>> = Mutex::new(None);
                    let workers = n_workers.min(take);
                    let run_w = || {
                        loop {
                            if failed.load(Ordering::Acquire) {
                                break;
                            }
                            let item = work.lock().unwrap_or_else(|g| g.into_inner()).pop();
                            let Some((bi, mut caller_ir)) = item else {
                                break;
                            };
                            match inline_single_caller(
                                &mut caller_ir,
                                bi,
                                inst_counts_ro[bi],
                                irs_ro,
                                inst_counts_ro,
                                in_cycle,
                                ext_ref_ro,
                                total_call_counts,
                                inlined_ro,
                                n_imports,
                                n_defs,
                                rem_cap,
                                track_ok,
                                warm,
                                budget,
                            ) {
                                Ok(Some(outcome)) => {
                                    let idx = next_out.fetch_add(1, Ordering::Relaxed);
                                    results.lock().unwrap_or_else(|g| g.into_inner())[idx] =
                                        Some((bi, caller_ir, outcome));
                                }
                                Ok(None) => {
                                    oom_warm.store(true, Ordering::Release);
                                    failed.store(true, Ordering::Release);
                                    break;
                                }
                                Err(e) => {
                                    failed.store(true, Ordering::Release);
                                    let mut g = first_err.lock().unwrap_or_else(|g| g.into_inner());
                                    if g.as_ref().is_none_or(|(p, _)| bi < *p) {
                                        *g = Some((bi, e));
                                    }
                                    break;
                                }
                            }
                        }
                    };
                    std::thread::scope(|s| {
                        for _ in 1..workers {
                            let _ = std::thread::Builder::new().spawn_scoped(s, run_w);
                        }
                        run_w();
                    });
                    if oom_warm.load(Ordering::Acquire) {
                        return Ok(None);
                    }
                    if let Some((_, err)) =
                        first_err.into_inner().unwrap_or_else(|g| g.into_inner())
                    {
                        return Err(err);
                    }
                    for (bi, caller_ir, out) in results
                        .into_inner()
                        .unwrap_or_else(|g| g.into_inner())
                        .into_iter()
                        .flatten()
                    {
                        irs[bi] = Some(caller_ir);
                        if !out.track_ok && track_ok {
                            track_ok = false;
                            inlined_callees.clear();
                        }
                        if out.did_inline {
                            inst_counts[bi] = out.caller_insts;
                            total_added_insts += out.caller_added;
                            if track_ok {
                                added_insts[bi] = out.caller_added as u32;
                                inlined_callees[bi] = out.inlined;
                                if out.became_inlinable {
                                    was_inlinable[bi] = true;
                                }
                            }
                        }
                    }
                    off += take;
                } else {
                    let bi = bucket[off];
                    let Some(mut caller_ir) = irs[bi].take() else {
                        off += 1;
                        continue;
                    };
                    let outcome = inline_single_caller(
                        &mut caller_ir,
                        bi,
                        inst_counts[bi],
                        irs,
                        &inst_counts,
                        in_cycle,
                        &ext_ref,
                        total_call_counts,
                        &inlined_callees,
                        n_imports,
                        n_defs,
                        rem_cap,
                        track_ok,
                        warm,
                        budget,
                    )?;
                    irs[bi] = Some(caller_ir);
                    let Some(out) = outcome else {
                        return Ok(None);
                    };
                    if !out.track_ok && track_ok {
                        track_ok = false;
                        inlined_callees.clear();
                    }
                    if out.did_inline {
                        inst_counts[bi] = out.caller_insts;
                        total_added_insts += out.caller_added;
                        if track_ok {
                            added_insts[bi] = out.caller_added as u32;
                            inlined_callees[bi] = out.inlined;
                            if out.became_inlinable {
                                was_inlinable[bi] = true;
                            }
                        }
                    }
                    if out.hit_module_cap || total_added_insts >= INLINE_MAX_MODULE_ADDED_INSTS {
                        hit_module_cap = true;
                        break 'strata;
                    }
                    off += 1;
                }
            }
        }
        if !fell_back_to_seq {
            return Ok(Some(InlineTrack {
                inlined_callees,
                added_insts,
                was_inlinable,
                hit_module_cap,
                tracking_complete: track_ok,
            }));
        }
    }

    for (step, &caller_bi) in topo_bottom_up.iter().enumerate() {
        if let Some(bg) = budget
            && (step & 63) == 0
        {
            bg.check_deadline()?;
        }
        if irs[caller_bi].is_none() || inst_counts[caller_bi] >= INLINE_MAX_CALLER_INSTS {
            continue;
        }
        if total_added_insts >= INLINE_MAX_MODULE_ADDED_INSTS {
            hit_module_cap = true;
            break;
        }
        let rem_cap = INLINE_MAX_MODULE_ADDED_INSTS - total_added_insts;
        let mut caller_ir = irs[caller_bi].take().unwrap();
        let outcome = inline_single_caller(
            &mut caller_ir,
            caller_bi,
            inst_counts[caller_bi],
            irs,
            &inst_counts,
            in_cycle,
            &ext_ref,
            total_call_counts,
            &inlined_callees,
            n_imports,
            n_defs,
            rem_cap,
            track_ok,
            warm,
            budget,
        )?;
        irs[caller_bi] = Some(caller_ir);
        let Some(out) = outcome else {
            return Ok(None);
        };
        if !out.track_ok && track_ok {
            track_ok = false;
            inlined_callees.clear();
        }
        if out.did_inline {
            inst_counts[caller_bi] = out.caller_insts;
            total_added_insts += out.caller_added;
            if track_ok {
                added_insts[caller_bi] = out.caller_added as u32;
                inlined_callees[caller_bi] = out.inlined;
                if out.became_inlinable {
                    was_inlinable[caller_bi] = true;
                }
            }
        }
        if out.hit_module_cap || total_added_insts >= INLINE_MAX_MODULE_ADDED_INSTS {
            hit_module_cap = true;
            break;
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
                    memory64: false,
                    early_pruned: 0,
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
#[derive(Clone, PartialEq, Eq)]
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
                | Op::V128StoreLane { .. }
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
    try_compute_module_effects_from_lookup(summaries.len(), |i| &summaries[i], None, module, budget)
}

/// Zero-clone, optionally topologically-ordered `ModuleEffects` computation.
pub fn try_compute_module_effects_from_lookup<'a, F>(
    n_defs: usize,
    get: F,
    topo_bottom_up: Option<&[usize]>,
    module: &X64Module,
    budget: Option<&CompileBudget>,
) -> Res<Option<ModuleEffects>>
where
    F: Fn(usize) -> &'a FnOwnEffects,
{
    if let Some(bg) = budget {
        bg.check_deadline()?;
    }
    let n_imports = module.imports.len();
    let n_total = n_imports + n_defs;
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
    for bi in 0..n_defs {
        let s = get(bi);
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
        let step_fn = |bi: usize,
                       changed: &mut bool,
                       fix_steps: &mut usize,
                       writes_mem: &mut [bool],
                       grows_mem: &mut [bool],
                       writes_all_globals: &mut [bool],
                       written_globals: &mut [Vec<u64>],
                       writes_all_tables: &mut [bool],
                       written_tables: &mut [u64]|
         -> Res<()> {
            let s = get(bi);
            let fi = n_imports + bi;
            for &c_u32 in &s.direct_callees {
                let c = c_u32 as usize;
                *fix_steps = fix_steps.saturating_add(global_words.max(1));
                if let Some(bg) = budget
                    && *fix_steps >= 4096
                {
                    *fix_steps = 0;
                    bg.check_deadline()?;
                }
                if writes_mem[c] && !writes_mem[fi] {
                    writes_mem[fi] = true;
                    *changed = true;
                }
                if grows_mem[c] && !grows_mem[fi] {
                    grows_mem[fi] = true;
                    *changed = true;
                }
                if writes_all_globals[c] && !writes_all_globals[fi] {
                    writes_all_globals[fi] = true;
                    *changed = true;
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
                            *changed = true;
                        }
                    }
                }
                if writes_all_tables[c] && !writes_all_tables[fi] {
                    writes_all_tables[fi] = true;
                    *changed = true;
                } else if !writes_all_tables[fi] {
                    let old = written_tables[fi];
                    let nv = old | written_tables[c];
                    if nv != old {
                        written_tables[fi] = nv;
                        *changed = true;
                    }
                }
            }
            Ok(())
        };
        if let Some(order) = topo_bottom_up {
            for &bi in order {
                step_fn(
                    bi,
                    &mut changed,
                    &mut fix_steps,
                    &mut writes_mem,
                    &mut grows_mem,
                    &mut writes_all_globals,
                    &mut written_globals,
                    &mut writes_all_tables,
                    &mut written_tables,
                )?;
            }
        } else {
            for bi in 0..n_defs {
                step_fn(
                    bi,
                    &mut changed,
                    &mut fix_steps,
                    &mut writes_mem,
                    &mut grows_mem,
                    &mut writes_all_globals,
                    &mut written_globals,
                    &mut writes_all_tables,
                    &mut written_tables,
                )?;
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
        | Op::V128ExtractLane { .. }
        | Op::V128ReplaceLane { .. }
        | Op::V128Shuffle { .. }
        | Op::V128Un { .. }
        | Op::V128Bin { .. }
        | Op::V128Ternary { .. }
        | Op::V128ToI32 { .. }
        | Op::V128Shift { .. } => true,
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
    let mut has_backedge = false;
    'chk_licm: for &b in &dom.rpo {
        let bp = dom.rpo_pos[b as usize];
        for &s in &dom.succs[b as usize] {
            if dom.rpo_pos[s as usize] <= bp && dom.dominates(s, b) {
                has_backedge = true;
                break 'chk_licm;
            }
        }
    }
    if !has_backedge {
        return Ok(false);
    }
    let mut loops_by_hdr: HashMap<u32, HashSet<u32>> = fx_map();
    for &b in &dom.rpo {
        let bp = dom.rpo_pos[b as usize];
        for &s in &dom.succs[b as usize] {
            if dom.rpo_pos[s as usize] <= bp && dom.dominates(s, b) {
                let body = loops_by_hdr.entry(s).or_insert_with(fx_set);
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
        let mut loop_written_globals: Vec<u64> = vec![0u64; effects.global_words];
        for &b in &body {
            for inst in &ir.blocks[b as usize].insts {
                match inst.op {
                    Op::MemGrow(_) => {
                        if !fixed_mem {
                            loop_grows_mem = true;
                        }
                    }
                    Op::GlobalSet(g, _) => {
                        let gi = g as usize;
                        if (gi >> 6) < loop_written_globals.len() {
                            loop_written_globals[gi >> 6] |= 1u64 << (gi & 63);
                        }
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
                            for (dst, &src) in loop_written_globals
                                .iter_mut()
                                .zip(effects.written_globals[f].iter())
                            {
                                *dst |= src;
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
                            let gi = g as usize;
                            let written = (gi >> 6) < loop_written_globals.len()
                                && (loop_written_globals[gi >> 6] & (1u64 << (gi & 63))) != 0;
                            module.pinned_global != Some(g) && !loop_writes_all_globals && !written
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
    let mut touched_blocks: HashSet<u32> = fx_set();
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
                    } else if ndb != u32::MAX
                        && body_set.contains(&ndb)
                        && let Some(num_ii) = ir.blocks[ndb as usize].insts.iter().position(|ins| {
                            ins.res == Some(num) && matches!(ins.op, Op::ConstI32(_))
                        })
                    {
                        let key = (ndb, num_ii);
                        if !to_hoist.contains(&key) {
                            to_hoist.push(key);
                        }
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
        let mut by_block: HashMap<u32, Vec<usize>> = fx_map();
        for &(bb_id, ii) in &to_hoist {
            by_block.entry(bb_id).or_default().push(ii);
        }
        let mut extracted: HashMap<(u32, usize), Inst> = fx_map_cap(to_hoist.len());
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
    offset: u64,
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
    NonZero(Val, bool),
    Global(u32, Option<(u32, Val)>),
    MemSize(Option<(u32, Val)>),
    Table((u32, Val), Option<(u32, Val)>),
    MemPush,
    MemRemove(usize, MemFact),
    MemClear(Vec<MemFact>),
}

/// Worker-local reusable scratch buffers for `optimize_func_stage2_with_scratch`, eliminating
/// per-function `HashMap`/`Vec` heap churn across parallel compilation threads.
pub struct Stage2Scratch {
    subst: Vec<Val>,
    exprs: HashMap<GvnKey, Val>,
    const32: Vec<Option<i32>>,
    add_const: Vec<Option<(Val, u32)>>,
    non_wrap_bound: Vec<Option<u64>>,
    checked_max: Vec<Option<u64>>,
    non_zero: Vec<bool>,
    globals: Vec<Option<(u32, Val)>>,
    tables: HashMap<(u32, Val), (u32, Val)>,
    mem_facts: Vec<MemFact>,
    undo: Vec<UndoEntry>,
    fn_written_globals: Vec<u64>,
    out_insts: Vec<Inst>,
    opt_ir: OptIrScratch,
}

impl Default for Stage2Scratch {
    fn default() -> Self {
        Self {
            subst: Vec::new(),
            exprs: fx_map(),
            const32: Vec::new(),
            add_const: Vec::new(),
            non_wrap_bound: Vec::new(),
            checked_max: Vec::new(),
            non_zero: Vec::new(),
            globals: Vec::new(),
            tables: fx_map(),
            mem_facts: Vec::new(),
            undo: Vec::new(),
            fn_written_globals: Vec::new(),
            out_insts: Vec::new(),
            opt_ir: OptIrScratch::default(),
        }
    }
}

struct GvnState<'a> {
    module: &'a X64Module,
    effects: &'a ModuleEffects,
    fn_may_grow: bool,
    fn_writes_mem: bool,
    fn_writes_all_globals: bool,
    fn_writes_any_global: bool,
    fn_written_globals: Vec<u64>,
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
    non_zero: Vec<bool>,
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
    #[inline]
    fn fn_writes_global(&self, g: u32) -> bool {
        if self.fn_writes_all_globals {
            return true;
        }
        let gi = g as usize;
        (gi >> 6) < self.fn_written_globals.len()
            && (self.fn_written_globals[gi >> 6] & (1u64 << (gi & 63))) != 0
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

    fn mark_non_zero(&mut self, v: Val) {
        let slot = &mut self.non_zero[v.0 as usize];
        if !*slot {
            *slot = true;
            self.undo.push(UndoEntry::NonZero(v, false));
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
        if !self.module.memory64
            && self.mem_max_bytes <= u32::MAX as u64
            && end_off <= self.mem_max_bytes
        {
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
        offset: u64,
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

    /// Looks up a dominating exact 4-byte or 8-byte `Load`/`Store` fact at `(addr, offset)`
    /// for bit-exact `MemCopyConst` store-to-load forwarding.
    fn find_exact_lane_fact(
        &self,
        addr: Val,
        const_addr: Option<u64>,
        offset: u64,
        width: u64,
    ) -> Option<(Val, StoreKind)> {
        for fact in self.mem_facts.iter().rev() {
            let same_loc = (fact.addr == addr && fact.offset == offset)
                || (fact.const_addr.is_some() && fact.const_addr == const_addr);
            if !same_loc || fact.size != width {
                continue;
            }
            match (width, fact.kind) {
                (8, MemFactKind::Load(LoadKind::F64) | MemFactKind::Store(StoreKind::F64)) => {
                    return Some((fact.val, StoreKind::F64));
                }
                (8, MemFactKind::Load(LoadKind::I64) | MemFactKind::Store(StoreKind::I64)) => {
                    return Some((fact.val, StoreKind::I64));
                }
                (4, MemFactKind::Load(LoadKind::F32) | MemFactKind::Store(StoreKind::F32)) => {
                    return Some((fact.val, StoreKind::F32));
                }
                (4, MemFactKind::Load(LoadKind::I32) | MemFactKind::Store(StoreKind::I32)) => {
                    return Some((fact.val, StoreKind::I32));
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
        offset: u64,
        sz: u64,
    ) {
        let mut i = self.mem_facts.len();
        while i > 0 {
            i -= 1;
            let f = self.mem_facts[i];
            let disjoint = (f.addr == addr && ranges_disjoint(f.offset, f.size, offset, sz))
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

    fn try_fold_addr(&self, addr: Val, offset: u64) -> (Val, u64) {
        if !self.module.memory64
            && let Some((base, k)) = self.add_const[addr.0 as usize]
            && let Some(bound) = self.non_wrap_bound[base.0 as usize]
            && bound.saturating_add(u64::from(k)) <= u32::MAX as u64
            && offset.saturating_add(u64::from(k)) <= i32::MAX as u64
        {
            return (base, offset + u64::from(k));
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
                UndoEntry::NonZero(k, prev) => {
                    self.non_zero[k.0 as usize] = prev;
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
        IntBinOp::DivU | IntBinOp::DivUNz if cb != 0 => Some(((ca as u32) / (cb as u32)) as i32),
        IntBinOp::RemS if cb != 0 => Some(if ca == i32::MIN && cb == -1 {
            0
        } else {
            ca.wrapping_rem(cb)
        }),
        IntBinOp::RemU | IntBinOp::RemUNz if cb != 0 => Some(((ca as u32) % (cb as u32)) as i32),
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
/// addressing-mode folding. Returns `(folded_any_branch, ir_mutated)`.
fn run_dom_gvn_and_bce_with_scratch(
    ir: &mut IrFunc,
    dom: &CfgDom,
    module: &X64Module,
    effects: &ModuleEffects,
    budget: Option<&CompileBudget>,
    scratch: &mut Stage2Scratch,
) -> Res<(bool, bool)> {
    if dom.rpo.is_empty() {
        return Ok((false, false));
    }
    let mut folded_any_branch = false;
    let mut ir_mutated = false;

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
    let mut fn_writes_any_global = false;
    let mut fn_written_globals = std::mem::take(&mut scratch.fn_written_globals);
    fn_written_globals.clear();
    fn_written_globals.resize(effects.global_words, 0u64);
    let mut fn_writes_all_tables = false;
    let mut fn_written_tables = 0u64;
    for &b in &dom.rpo {
        for inst in &ir.blocks[b as usize].insts {
            match inst.op {
                Op::Store { .. }
                | Op::V128StoreLane { .. }
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
                    let gi = g as usize;
                    if (gi >> 6) < fn_written_globals.len() {
                        fn_written_globals[gi >> 6] |= 1u64 << (gi & 63);
                        fn_writes_any_global = true;
                    }
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
                    fn_writes_any_global = true;
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
                        fn_writes_any_global = true;
                    } else {
                        for (dst, &src) in fn_written_globals
                            .iter_mut()
                            .zip(effects.written_globals[f].iter())
                        {
                            if src != 0 {
                                *dst |= src;
                                fn_writes_any_global = true;
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
    let mut subst = std::mem::take(&mut scratch.subst);
    subst.clear();
    subst.extend((0..n_vals as u32).map(Val));
    let mut exprs = std::mem::take(&mut scratch.exprs);
    exprs.clear();
    let mut const32 = std::mem::take(&mut scratch.const32);
    const32.clear();
    const32.resize(n_vals, None);
    let mut add_const = std::mem::take(&mut scratch.add_const);
    add_const.clear();
    add_const.resize(n_vals, None);
    let mut non_wrap_bound = std::mem::take(&mut scratch.non_wrap_bound);
    non_wrap_bound.clear();
    non_wrap_bound.resize(n_vals, None);
    let mut checked_max = std::mem::take(&mut scratch.checked_max);
    checked_max.clear();
    checked_max.resize(n_vals, None);
    let mut non_zero = std::mem::take(&mut scratch.non_zero);
    non_zero.clear();
    non_zero.resize(n_vals, false);
    let mut globals = std::mem::take(&mut scratch.globals);
    globals.clear();
    globals.resize(module.globals.len(), None);
    let mut tables = std::mem::take(&mut scratch.tables);
    tables.clear();
    let mut mem_facts = std::mem::take(&mut scratch.mem_facts);
    mem_facts.clear();
    let mut undo = std::mem::take(&mut scratch.undo);
    undo.clear();

    let mut st = GvnState {
        module,
        effects,
        fn_may_grow,
        fn_writes_mem,
        fn_writes_all_globals,
        fn_writes_any_global,
        fn_written_globals,
        fn_writes_all_tables,
        fn_written_tables,
        mem_min_bytes,
        mem_max_bytes,
        subst,
        any_subst: false,
        exprs,
        const32,
        add_const,
        non_wrap_bound,
        checked_max,
        non_zero,
        globals,
        mem_size: None,
        tables,
        mem_facts,
        undo,
        grow_epoch: 1,
        glob_epoch: 1,
        tbl_epoch: 1,
    };

    #[derive(Clone, Copy)]
    enum EdgeFact {
        None,
        Bound(Val, u64),
        LtU(Val, Val),
        NonZero(Val),
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

    let mut out_insts = std::mem::take(&mut scratch.out_insts);
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

                // Apply branch-refined range & non-zero facts when this block is uniquely dominated by one branch edge.
                match edge_fact {
                    EdgeFact::None => {}
                    EdgeFact::Bound(v, b) => st.tighten_bound(v, b),
                    EdgeFact::LtU(x, y) => {
                        st.mark_non_zero(y);
                        if let Some(by) = st.non_wrap_bound[y.0 as usize]
                            && by > 0
                        {
                            st.tighten_bound(x, by - 1);
                        }
                        if let Some(cy) = st.checked_max[y.0 as usize] {
                            st.record_checked(x, cy.saturating_add(1));
                        }
                    }
                    EdgeFact::NonZero(v) => st.mark_non_zero(v),
                }

                // At multi-predecessor joins/loops, conservatively invalidate mutable state (read-only facts persist!).
                if dom.preds[bid as usize].len() > 1 {
                    if st.fn_may_grow {
                        st.grow_epoch = st.grow_epoch.wrapping_add(1);
                    }
                    if st.fn_writes_mem {
                        st.clear_mem_facts();
                    }
                    if st.fn_writes_all_globals || st.fn_writes_any_global {
                        st.glob_epoch = st.glob_epoch.wrapping_add(1);
                    }
                    if st.fn_writes_all_tables || st.fn_written_tables != 0 {
                        st.tbl_epoch = st.tbl_epoch.wrapping_add(1);
                    }
                }

                let bb = &mut ir.blocks[bid as usize];
                out_insts.clear();
                for mut inst in bb.insts.drain(..) {
                    if st.any_subst {
                        rewrite_op_uses(&mut inst.op, &mut pool, &st.subst);
                    }

                    // Promote newly-known constant `MemCopy`/`MemFill` lengths (<= 64) after inlining/GVN.
                    if !module.memory64 {
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
                                    ir_mutated = true;
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
                                    ir_mutated = true;
                                }
                            }
                            _ => {}
                        }
                    }

                    // Wrap-safe addressing-mode folding + cross-block dominator BCE + alias-aware Load/Store forwarding.
                    match &mut inst.op {
                        Op::BoundsCheck { addr, end_off, .. } => {
                            if st.is_checked(*addr, *end_off) {
                                ir_mutated = true;
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
                            if na != *addr || noff != *offset {
                                *addr = na;
                                *offset = noff;
                                ir_mutated = true;
                            }
                            let sz = kind.bytes();
                            let end = (*offset).saturating_add(sz);
                            let is_safe = *checked || st.is_checked(*addr, end);
                            if is_safe {
                                *checked = true;
                            }
                            let const_addr = if module.memory64 {
                                None
                            } else {
                                st.const32[addr.0 as usize]
                                    .map(|c| u64::from(c as u32).saturating_add(*offset))
                            };
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
                            if na != *addr || noff != *offset {
                                *addr = na;
                                *offset = noff;
                                ir_mutated = true;
                            }
                            let sz = kind.bytes();
                            let end = (*offset).saturating_add(sz);
                            if *checked || st.is_checked(*addr, end) {
                                *checked = true;
                            }
                            st.record_checked(*addr, end);
                            let const_addr = if module.memory64 {
                                None
                            } else {
                                st.const32[addr.0 as usize]
                                    .map(|c| u64::from(c as u32).saturating_add(*offset))
                            };
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
                        Op::V128LoadLane {
                            width,
                            addr,
                            offset,
                            checked,
                            ..
                        } => {
                            let (na, noff) = st.try_fold_addr(*addr, *offset);
                            if na != *addr || noff != *offset {
                                *addr = na;
                                *offset = noff;
                                ir_mutated = true;
                            }
                            let end = (*offset).saturating_add(u64::from(*width));
                            if *checked || st.is_checked(*addr, end) {
                                *checked = true;
                            }
                            st.record_checked(*addr, end);
                            out_insts.push(inst);
                            continue;
                        }
                        Op::V128StoreLane {
                            width,
                            addr,
                            offset,
                            checked,
                            ..
                        } => {
                            let (na, noff) = st.try_fold_addr(*addr, *offset);
                            if na != *addr || noff != *offset {
                                *addr = na;
                                *offset = noff;
                                ir_mutated = true;
                            }
                            let sz = u64::from(*width);
                            let end = (*offset).saturating_add(sz);
                            if *checked || st.is_checked(*addr, end) {
                                *checked = true;
                            }
                            st.record_checked(*addr, end);
                            let const_addr = if module.memory64 {
                                None
                            } else {
                                st.const32[addr.0 as usize]
                                    .map(|c| u64::from(c as u32).saturating_add(*offset))
                            };
                            st.kill_aliasing_mem_facts(*addr, const_addr, *offset, sz);
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
                            let dst_ok = *dst_checked || st.is_checked(*dst, end);
                            let src_ok = *src_checked || st.is_checked(*src, end);

                            // If every lane of `src[0..len]` is already live in dominating SSA
                            // `mem_facts` (`Load`/`Store` of exact 4/8-byte lanes, <= 4 stores),
                            // lower `MemCopyConst` directly to `BoundsCheck`(s) + checked `Store`s.
                            // This eliminates store-to-load forwarding (STLF) stalls and redundant
                            // loads while preserving exact overlap (`memmove`) and zero-write OOB
                            // trap semantics (all source values are already snapped in SSA regs
                            // and both bounds checks execute before any store).
                            let mut forwarded_lanes: [(u32, Val, StoreKind); 4] =
                                [(0, Val(0), StoreKind::I64); 4];
                            let mut n_lanes = 0usize;
                            if *len > 0 && *len <= 32 && (*len & 3) == 0 {
                                let (mut s_base, mut s_off) = st.try_fold_addr(*src, 0);
                                if s_off.saturating_add(end) > i32::MAX as u64 {
                                    s_base = *src;
                                    s_off = 0;
                                }
                                let s_const_base = if module.memory64 {
                                    None
                                } else {
                                    st.const32[s_base.0 as usize].map(|c| u64::from(c as u32))
                                };
                                let mut cur = 0u32;
                                let mut ok = true;
                                while cur < *len {
                                    if n_lanes >= 4 {
                                        ok = false;
                                        break;
                                    }
                                    let off = s_off + u64::from(cur);
                                    let ca = s_const_base.map(|cb| cb.saturating_add(off));
                                    if cur + 8 <= *len
                                        && let Some((v, sk)) =
                                            st.find_exact_lane_fact(s_base, ca, off, 8)
                                    {
                                        forwarded_lanes[n_lanes] = (cur, v, sk);
                                        n_lanes += 1;
                                        cur += 8;
                                    } else if let Some((v, sk)) =
                                        st.find_exact_lane_fact(s_base, ca, off, 4)
                                    {
                                        forwarded_lanes[n_lanes] = (cur, v, sk);
                                        n_lanes += 1;
                                        cur += 4;
                                    } else {
                                        ok = false;
                                        break;
                                    }
                                }
                                if !ok {
                                    n_lanes = 0;
                                }
                            }

                            if n_lanes > 0 {
                                ir_mutated = true;
                                if !dst_ok {
                                    out_insts.push(Inst {
                                        res: None,
                                        op: Op::BoundsCheck {
                                            addr: *dst,
                                            end_off: end,
                                            for_bulk: true,
                                        },
                                    });
                                }
                                if !src_ok {
                                    out_insts.push(Inst {
                                        res: None,
                                        op: Op::BoundsCheck {
                                            addr: *src,
                                            end_off: end,
                                            for_bulk: true,
                                        },
                                    });
                                }
                                st.record_checked(*dst, end);
                                st.record_checked(*src, end);
                                let (mut d_base, mut d_off) = st.try_fold_addr(*dst, 0);
                                if d_off.saturating_add(end) > i32::MAX as u64 {
                                    d_base = *dst;
                                    d_off = 0;
                                }
                                for &(lane_off, val, store_kind) in &forwarded_lanes[..n_lanes] {
                                    let off = d_off + u64::from(lane_off);
                                    let sz = store_kind.bytes();
                                    let ca = if module.memory64 {
                                        None
                                    } else {
                                        st.const32[d_base.0 as usize]
                                            .map(|c| u64::from(c as u32).saturating_add(off))
                                    };
                                    st.kill_aliasing_mem_facts(d_base, ca, off, sz);
                                    st.push_mem_fact(MemFact {
                                        addr: d_base,
                                        const_addr: ca,
                                        offset: off,
                                        size: sz,
                                        kind: MemFactKind::Store(store_kind),
                                        val,
                                        val_ty: ir.val_tys[val.0 as usize],
                                    });
                                    out_insts.push(Inst {
                                        res: None,
                                        op: Op::Store {
                                            kind: store_kind,
                                            addr: d_base,
                                            val,
                                            offset: off,
                                            checked: true,
                                        },
                                    });
                                }
                                continue;
                            }

                            if dst_ok {
                                *dst_checked = true;
                            }
                            if src_ok {
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
                                    ir_mutated = true;
                                }
                            }
                            Op::I32BinImm(bop, a, imm) => {
                                if let Some(ca) = st.const32[a.0 as usize]
                                    && let Some(v) = fold_i32_bin_const(bop, ca, imm)
                                {
                                    inst.op = Op::ConstI32(v);
                                    ir_mutated = true;
                                }
                            }
                            Op::I32Bin(bop, a, b) => {
                                if let (Some(ca), Some(cb)) =
                                    (st.const32[a.0 as usize], st.const32[b.0 as usize])
                                    && let Some(v) = fold_i32_bin_const(bop, ca, cb)
                                {
                                    inst.op = Op::ConstI32(v);
                                    ir_mutated = true;
                                } else if matches!(bop, IntBinOp::DivU | IntBinOp::RemU)
                                    && st.non_zero[b.0 as usize]
                                {
                                    let nz_op = if bop == IntBinOp::DivU {
                                        IntBinOp::DivUNz
                                    } else {
                                        IntBinOp::RemUNz
                                    };
                                    inst.op = Op::I32Bin(nz_op, a, b);
                                    ir_mutated = true;
                                }
                            }
                            Op::I64Bin(bop @ (IntBinOp::DivU | IntBinOp::RemU), a, b) => {
                                if st.non_zero[b.0 as usize] {
                                    let nz_op = if bop == IntBinOp::DivU {
                                        IntBinOp::DivUNz
                                    } else {
                                        IntBinOp::RemUNz
                                    };
                                    inst.op = Op::I64Bin(nz_op, a, b);
                                    ir_mutated = true;
                                }
                            }
                            Op::I32CmpImm(cop, a, imm) => {
                                if let Some(ca) = st.const32[a.0 as usize] {
                                    inst.op =
                                        Op::ConstI32(i32::from(fold_i32_cmp_const(cop, ca, imm)));
                                    ir_mutated = true;
                                }
                            }
                            Op::I32Cmp(cop, a, b) => {
                                if let (Some(ca), Some(cb)) =
                                    (st.const32[a.0 as usize], st.const32[b.0 as usize])
                                {
                                    inst.op =
                                        Op::ConstI32(i32::from(fold_i32_cmp_const(cop, ca, cb)));
                                    ir_mutated = true;
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
                                if c != 0 {
                                    st.mark_non_zero(res);
                                }
                            }
                            Op::ConstI64(c) if c != 0 => {
                                st.mark_non_zero(res);
                            }
                            Op::I32BinImm(IntBinOp::Or, _, m)
                            | Op::I64BinImm(IntBinOp::Or, _, m)
                                if m != 0 =>
                            {
                                st.mark_non_zero(res);
                            }
                            Op::I32BinImm(IntBinOp::Add, base, k) if k > 0 => {
                                st.insert_add_const(res, base, k as u32);
                                if let Some(b) = st.non_wrap_bound[base.0 as usize]
                                    && b.saturating_add(u64::from(k as u32)) <= u32::MAX as u64
                                {
                                    st.tighten_bound(res, b + u64::from(k as u32));
                                    st.mark_non_zero(res);
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
                bb.insts.extend_from_slice(&out_insts);
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
                                        CmpOp::Ne if imm == 0 => {
                                            ef = EdgeFact::NonZero(a);
                                        }
                                        CmpOp::GtU => {
                                            ef = EdgeFact::NonZero(a);
                                        }
                                        CmpOp::GeU if ku >= 1 => {
                                            ef = EdgeFact::NonZero(a);
                                        }
                                        CmpOp::GtS if imm >= 0 => {
                                            ef = EdgeFact::NonZero(a);
                                        }
                                        CmpOp::LtS if imm <= 0 => {
                                            ef = EdgeFact::NonZero(a);
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
                                CmpBranchKind::I64Imm(cop, imm) => {
                                    let eff = if on_then { cop } else { cop.invert_int() };
                                    match eff {
                                        CmpOp::Ne if imm == 0 => ef = EdgeFact::NonZero(a),
                                        CmpOp::GtU => ef = EdgeFact::NonZero(a),
                                        CmpOp::GeU if imm != 0 => ef = EdgeFact::NonZero(a),
                                        CmpOp::GtS if imm >= 0 => ef = EdgeFact::NonZero(a),
                                        CmpOp::LtS if imm <= 0 => ef = EdgeFact::NonZero(a),
                                        _ => {}
                                    }
                                }
                                CmpBranchKind::I64(cop) => {
                                    let eff = if on_then { cop } else { cop.invert_int() };
                                    match eff {
                                        CmpOp::LtU => ef = EdgeFact::NonZero(b),
                                        CmpOp::GtU => ef = EdgeFact::NonZero(a),
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
    // Return scratch buffers for reuse across functions on this worker thread.
    scratch.subst = st.subst;
    scratch.exprs = st.exprs;
    scratch.const32 = st.const32;
    scratch.add_const = st.add_const;
    scratch.non_wrap_bound = st.non_wrap_bound;
    scratch.checked_max = st.checked_max;
    scratch.non_zero = st.non_zero;
    scratch.globals = st.globals;
    scratch.tables = st.tables;
    scratch.mem_facts = st.mem_facts;
    scratch.undo = st.undo;
    scratch.fn_written_globals = st.fn_written_globals;
    scratch.out_insts = out_insts;
    Ok((
        folded_any_branch,
        ir_mutated || st.any_subst || folded_any_branch,
    ))
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

/// Splits `BranchIf(Or(c1, c2), then, else)` and `BranchIf(And(c1, c2), then, else)` where `c1`
/// and `c2` are pure boolean-producing (`0`/`1`) comparison ops into two short-circuiting
/// conditional branches so `optimize_ir_with_budget` fuses both directly into `CmpBranch`
/// (eliminating `setcc + movzx + or/and + test` in hot inner loops).
fn split_short_circuit_branches(ir: &mut IrFunc) -> bool {
    if !ir
        .blocks
        .iter()
        .any(|b| matches!(b.term, Term::BranchIf { .. }))
    {
        return false;
    }
    let n_vals = ir.val_tys.len();
    let mut use_count = vec![0u32; n_vals];
    let mut is_bool_cmp = vec![false; n_vals];
    for bb in &ir.blocks {
        for inst in &bb.insts {
            if let Some(res) = inst.res
                && matches!(
                    inst.op,
                    Op::I32Cmp(..)
                        | Op::I32CmpImm(..)
                        | Op::I64Cmp(..)
                        | Op::I64CmpImm(..)
                        | Op::F32Cmp(..)
                        | Op::F64Cmp(..)
                        | Op::I32Eqz(_)
                        | Op::I64Eqz(_)
                )
            {
                is_bool_cmp[res.0 as usize] = true;
            }
            inst.op.for_each_use(&ir.val_pool, |v| {
                use_count[v.0 as usize] = use_count[v.0 as usize].saturating_add(1);
            });
        }
        bb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
            use_count[v.0 as usize] = use_count[v.0 as usize].saturating_add(1);
        });
    }

    let orig_blocks = ir.blocks.len();
    let mut split_any = false;
    for bi in 0..orig_blocks {
        if ir.blocks.len() >= MAX_BLOCKS_PER_FN {
            break;
        }
        let Term::BranchIf {
            cond,
            then_bb,
            then_args,
            else_bb,
            else_args,
        } = ir.blocks[bi].term
        else {
            continue;
        };
        if then_bb == else_bb || use_count[cond.0 as usize] != 1 {
            continue;
        }
        let Some((cond_idx, is_or, lhs, rhs)) = ir.blocks[bi]
            .insts
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, ins)| {
                if ins.res != Some(cond) {
                    return None;
                }
                match ins.op {
                    Op::I32Bin(IntBinOp::Or, l, r) if l != r => Some((i, true, l, r)),
                    Op::I32Bin(IntBinOp::And, l, r) if l != r => Some((i, false, l, r)),
                    _ => None,
                }
            })
        else {
            continue;
        };
        if !is_bool_cmp[lhs.0 as usize] || !is_bool_cmp[rhs.0 as usize] {
            continue;
        }
        let lhs_in_bi = ir.blocks[bi].insts[..cond_idx]
            .iter()
            .any(|ins| ins.res == Some(lhs));
        let rhs_idx_in_bi = ir.blocks[bi].insts[..cond_idx]
            .iter()
            .rposition(|ins| ins.res == Some(rhs));
        if !lhs_in_bi && rhs_idx_in_bi.is_none() {
            continue;
        }

        // Duplicate the shared target's edge arguments before mutating the CFG.
        let shared_args = if is_or { then_args } else { else_args };
        let dup_args = if shared_args.len == 0 {
            ValRange::EMPTY
        } else {
            let copy: Vec<Val> = ir.pool_slice(shared_args).to_vec();
            let Ok(r) = ir.push_pool(&copy) else {
                continue;
            };
            r
        };

        // Remove `cond` (guaranteed `cond_idx > rhs_idx` in SSA), and move `rhs` into `mid_bb`
        // if `rhs` is defined in `bi` with `cond` as its sole use.
        ir.blocks[bi].insts.remove(cond_idx);
        let mut mid_insts = Vec::new();
        if let Some(ri) = rhs_idx_in_bi
            && use_count[rhs.0 as usize] == 1
        {
            let rhs_inst = ir.blocks[bi].insts.remove(ri);
            mid_insts.push(rhs_inst);
        }

        let mid_bb = BlockId(ir.blocks.len() as u32);
        if is_or {
            ir.blocks[then_bb.0 as usize].preds =
                ir.blocks[then_bb.0 as usize].preds.saturating_add(1);
            ir.blocks[bi].term = Term::BranchIf {
                cond: lhs,
                then_bb,
                then_args,
                else_bb: mid_bb,
                else_args: ValRange::EMPTY,
            };
            ir.blocks.push(Block {
                params: Vec::new(),
                insts: mid_insts,
                term: Term::BranchIf {
                    cond: rhs,
                    then_bb,
                    then_args: dup_args,
                    else_bb,
                    else_args,
                },
                is_loop_header: false,
                preds: 1,
            });
        } else {
            ir.blocks[else_bb.0 as usize].preds =
                ir.blocks[else_bb.0 as usize].preds.saturating_add(1);
            ir.blocks[bi].term = Term::BranchIf {
                cond: lhs,
                then_bb: mid_bb,
                then_args: ValRange::EMPTY,
                else_bb,
                else_args,
            };
            ir.blocks.push(Block {
                params: Vec::new(),
                insts: mid_insts,
                term: Term::BranchIf {
                    cond: rhs,
                    then_bb,
                    then_args,
                    else_bb,
                    else_args: dup_args,
                },
                is_loop_header: false,
                preds: 1,
            });
        }
        split_any = true;
    }
    split_any
}

#[derive(Clone, Copy, Debug)]
enum LoopBound {
    /// Known exact unsigned 32-bit constant `c` (`0 <= c <= u32::MAX`).
    Const(u64),
    /// Proven `0 <= v <= ((limit - 1) << shift) + add` with no 32-bit wrap along its
    /// derivation whenever `((limit - 1) << shift) + add <= u32::MAX` (and `limit >= 1`).
    ScaleAdd { shift: u32, add: u64 },
}

fn join_loop_bounds(a: LoopBound, b: LoopBound) -> LoopBound {
    match (a, b) {
        (LoopBound::Const(c1), LoopBound::Const(c2)) => LoopBound::Const(c1.max(c2)),
        (LoopBound::Const(c), LoopBound::ScaleAdd { shift, add })
        | (LoopBound::ScaleAdd { shift, add }, LoopBound::Const(c)) => LoopBound::ScaleAdd {
            shift,
            add: add.max(c),
        },
        (
            LoopBound::ScaleAdd { shift: s1, add: a1 },
            LoopBound::ScaleAdd { shift: s2, add: a2 },
        ) => LoopBound::ScaleAdd {
            shift: s1.max(s2),
            add: a1.max(a2),
        },
    }
}

/// Computes the tightest `max_safe_limit: u32` such that for every `1 <= limit <= max_safe_limit`,
/// `addr` neither wraps 32-bit along its derivation nor exceeds `mem_min_bytes` with `end_off`.
fn max_safe_limit_for_bound(bound: LoopBound, end_off: u64, mem_min_bytes: u64) -> Option<u32> {
    match bound {
        LoopBound::Const(c) => {
            let need = u128::from(c) + u128::from(end_off);
            (need <= u128::from(mem_min_bytes)).then_some(u32::MAX)
        }
        LoopBound::ScaleAdd { shift, add } => {
            if add > u64::from(u32::MAX) {
                return None;
            }
            let base_need = u128::from(add) + u128::from(end_off);
            if base_need > u128::from(mem_min_bytes) {
                return None;
            }
            let max_lm1_wrap = (u64::from(u32::MAX) - add) >> shift;
            let max_lm1_mem = ((u128::from(mem_min_bytes) - base_need) >> shift) as u64;
            let max_lm1 = max_lm1_wrap.min(max_lm1_mem);
            let max_limit = max_lm1.saturating_add(1).min(u64::from(u32::MAX)) as u32;
            Some(max_limit)
        }
    }
}

fn remap_op_uses_map(op: &mut Op, val_map: &HashMap<Val, Val>) {
    let r = |v: &mut Val| {
        if let Some(&nv) = val_map.get(v) {
            *v = nv;
        }
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
        | Op::V128ExtractLane { v: a, .. }
        | Op::V128Un { a, .. }
        | Op::V128ToI32 { a, .. } => r(a),
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
        | Op::V128LoadLane {
            addr: a, vec: b, ..
        }
        | Op::V128StoreLane {
            addr: a, vec: b, ..
        }
        | Op::V128ReplaceLane { v: a, x: b, .. }
        | Op::V128Shuffle { a, b, .. }
        | Op::V128Bin { a, b, .. }
        | Op::V128Shift { v: a, shift: b, .. }
        | Op::MemCopyConst { dst: a, src: b, .. }
        | Op::MemFillConst { dst: a, val: b, .. }
        | Op::MemByteSum {
            base: a, count: b, ..
        } => {
            r(a);
            r(b);
        }
        Op::V128Ternary { a, b, c, .. } => {
            r(a);
            r(b);
            r(c);
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
        Op::Call { .. } | Op::CallIndirect { .. } => {
            debug_assert!(
                false,
                "version_checked_counted_loops excludes Call/CallIndirect"
            );
        }
    }
}

/// Preheader-guarded counted-loop BCE versioning: for small single-entry, single-exit loops
/// whose header guards `idx <u limit` (or proven non-negative `idx <s limit`) with loop-invariant
/// `limit`, proves affine/clamped memory addresses in-bounds against `mem_min_bytes` under a single
/// preheader check `limit <=u max_safe_limit`. Clones a bounds-check-free fast loop (also folding
/// wrap-safe `Add(base, k)` into `Load`/`Store` offsets) while preserving the original checked loop
/// on the slow path for exact out-of-bounds trap order and partial side effects.
fn version_checked_counted_loops(
    ir: &mut IrFunc,
    dom: &CfgDom,
    module: &X64Module,
    budget: Option<&CompileBudget>,
) -> Res<bool> {
    const LOOP_VER_MAX_BLOCKS: usize = 4;
    const LOOP_VER_MAX_INSTS: usize = 64;
    const LOOP_VER_MAX_PER_FN: usize = 4;

    if ir.blocks.len() <= 1 || dom.rpo.is_empty() || module.memory64 {
        return Ok(false);
    }
    let mem_min_bytes = module
        .memory
        .map_or(0u64, |(min_p, _)| min_p.saturating_mul(65_536));
    if mem_min_bytes == 0 {
        return Ok(false);
    }

    // Discover natural loops by backedge `b -> s` where `dom.dominates(s, b)`.
    let mut has_backedge = false;
    'chk_ver: for &b in &dom.rpo {
        let bp = dom.rpo_pos[b as usize];
        for &s in &dom.succs[b as usize] {
            if dom.rpo_pos[s as usize] <= bp && dom.dominates(s, b) {
                has_backedge = true;
                break 'chk_ver;
            }
        }
    }
    if !has_backedge {
        return Ok(false);
    }
    let mut loops_by_hdr: HashMap<u32, HashSet<u32>> = fx_map();
    for &b in &dom.rpo {
        let bp = dom.rpo_pos[b as usize];
        for &s in &dom.succs[b as usize] {
            if dom.rpo_pos[s as usize] <= bp && dom.dominates(s, b) {
                let body = loops_by_hdr.entry(s).or_insert_with(fx_set);
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

    let mut loops: Vec<(u32, Vec<u32>)> = loops_by_hdr
        .into_iter()
        .filter(|&(h, _)| h != 0)
        .map(|(h, set)| {
            let mut v: Vec<u32> = set.into_iter().collect();
            v.sort_by_key(|&b| dom.rpo_pos[b as usize]);
            (h, v)
        })
        .collect();
    loops.sort_by_key(|(h, body)| (body.len(), *h));

    let mut def_block = vec![u32::MAX; ir.val_tys.len()];
    let mut def_inst: Vec<Option<Op>> = vec![None; ir.val_tys.len()];
    for (bi, bb) in ir.blocks.iter().enumerate() {
        for &p in &bb.params {
            def_block[p.0 as usize] = bi as u32;
        }
        for inst in &bb.insts {
            if let Some(r) = inst.res {
                def_block[r.0 as usize] = bi as u32;
                def_inst[r.0 as usize] = Some(inst.op);
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

    let mut versioned_any = false;
    let mut versioned_count = 0usize;
    let mut versioned_body_blocks: HashSet<u32> = fx_set();
    let mut versioned_pres: HashSet<u32> = fx_set();
    let mut versioned_exits: HashSet<u32> = fx_set();
    let mut ver_steps = 0usize;

    for (hdr, body) in loops {
        if versioned_count >= LOOP_VER_MAX_PER_FN {
            break;
        }
        if let Some(bg) = budget {
            bg.check_deadline()?;
        }
        if body.len() < 2 || body.len() > LOOP_VER_MAX_BLOCKS {
            continue;
        }
        if body
            .iter()
            .any(|b| versioned_body_blocks.contains(b) || versioned_pres.contains(b))
            || versioned_exits.contains(&hdr)
        {
            continue;
        }
        let body_insts: usize = body
            .iter()
            .map(|&b| ir.blocks[b as usize].insts.len())
            .sum();
        if body_insts > LOOP_VER_MAX_INSTS {
            continue;
        }
        if ir.blocks.len() + body.len() > MAX_BLOCKS_PER_FN {
            break;
        }

        let body_set: HashSet<u32> = body.iter().copied().collect();
        let outside_preds: Vec<u32> = dom.preds[hdr as usize]
            .iter()
            .copied()
            .filter(|p| !body_set.contains(p))
            .collect();
        if outside_preds.len() != 1 {
            continue;
        }
        let pre = outside_preds[0];
        if versioned_body_blocks.contains(&pre) || versioned_pres.contains(&pre) {
            continue;
        }
        if !dom.dominates(pre, hdr) {
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

        // Inspect header terminator: must branch on I32 comparison into `body_entry` vs `exit_bb`.
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
        let then_in = body_set.contains(&then_bb.0);
        let else_in = body_set.contains(&else_bb.0);
        if then_in == else_in {
            continue;
        }
        let body_on_then = then_in;
        let (body_entry, exit_bb, exit_args) = if body_on_then {
            (then_bb.0, else_bb, else_args)
        } else {
            (else_bb.0, then_bb, then_args)
        };
        if body_entry == hdr
            || dom.preds[body_entry as usize].len() != 1
            || dom.preds[body_entry as usize][0] != hdr
        {
            continue;
        }
        if exit_bb.0 == 0
            || versioned_body_blocks.contains(&exit_bb.0)
            || versioned_exits.contains(&exit_bb.0)
            || dom.preds[exit_bb.0 as usize].len() != 1
            || dom.preds[exit_bb.0 as usize][0] != hdr
        {
            continue;
        }

        // Verify body has no calls, memory.grow, br_table, or non-header exits outside `body_set`.
        let mut body_clean = true;
        for &bb_id in &body {
            let bb = &ir.blocks[bb_id as usize];
            if matches!(bb.term, Term::BrTable { .. }) {
                body_clean = false;
                break;
            }
            if bb.insts.iter().any(|ins| {
                matches!(
                    ins.op,
                    Op::Call { .. } | Op::CallIndirect { .. } | Op::MemGrow(_)
                )
            }) {
                body_clean = false;
                break;
            }
            if bb_id != hdr {
                bb.term.for_each_succ(&ir.br_tables, |s| {
                    if !body_set.contains(&s.0) {
                        body_clean = false;
                    }
                });
                if !body_clean {
                    break;
                }
            }
        }
        if !body_clean {
            continue;
        }

        let const_of = |v: Val, def_inst: &[Option<Op>]| -> Option<u64> {
            match def_inst[v.0 as usize] {
                Some(Op::ConstI32(c)) => Some(u64::from(c as u32)),
                _ => None,
            }
        };

        let eff_cop = if body_on_then { cop } else { cop.invert_int() };
        let (idx, limit, signed_cap): (Val, Val, Option<u32>) = match eff_cop {
            CmpOp::LtU => (a, b, None),
            CmpOp::GtU => (b, a, None),
            CmpOp::LtS | CmpOp::GtS => {
                let (cand_idx, cand_lim) = if eff_cop == CmpOp::LtS {
                    (a, b)
                } else {
                    (b, a)
                };
                // For signed `idx <s limit`, verify `idx` is a monotonically increasing
                // non-negative header param `init >= 0` with constant step `1..=65536` on every
                // backedge so `limit <=u (i32::MAX - max_step)` guarantees `0 <= idx < limit`.
                let Some(p_pos) = ir.blocks[hdr as usize]
                    .params
                    .iter()
                    .position(|&p| p == cand_idx)
                else {
                    continue;
                };
                let entry_slice = ir.pool_slice(entry_args);
                if p_pos >= entry_slice.len() {
                    continue;
                }
                let Some(Op::ConstI32(init_v)) = def_inst[entry_slice[p_pos].0 as usize] else {
                    continue;
                };
                if init_v < 0 {
                    continue;
                }
                let mut max_step = 0u32;
                let mut all_backedges_ok = true;
                let mut saw_backedge = false;
                let check_backedge_arg =
                    |args: ValRange, max_step: &mut u32, all_ok: &mut bool, saw: &mut bool| {
                        *saw = true;
                        let sl = ir.pool_slice(args);
                        if p_pos >= sl.len() {
                            *all_ok = false;
                            return;
                        }
                        let nxt = sl[p_pos];
                        let step_opt = match def_inst[nxt.0 as usize] {
                            Some(Op::I32BinImm(IntBinOp::Add, x, s))
                                if x == cand_idx && (1..=65_536).contains(&s) =>
                            {
                                Some(s as u32)
                            }
                            Some(Op::I32Bin(IntBinOp::Add, x, y)) if x == cand_idx => {
                                const_of(y, &def_inst)
                                    .and_then(|c| (1..=65_536).contains(&c).then_some(c as u32))
                            }
                            Some(Op::I32Bin(IntBinOp::Add, x, y)) if y == cand_idx => {
                                const_of(x, &def_inst)
                                    .and_then(|c| (1..=65_536).contains(&c).then_some(c as u32))
                            }
                            _ => None,
                        };
                        if let Some(st) = step_opt {
                            *max_step = (*max_step).max(st);
                        } else {
                            *all_ok = false;
                        }
                    };
                for &bb_id in &body {
                    match ir.blocks[bb_id as usize].term {
                        Term::Jump { target, args } if target.0 == hdr => {
                            check_backedge_arg(
                                args,
                                &mut max_step,
                                &mut all_backedges_ok,
                                &mut saw_backedge,
                            );
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
                            if then_bb.0 == hdr {
                                check_backedge_arg(
                                    then_args,
                                    &mut max_step,
                                    &mut all_backedges_ok,
                                    &mut saw_backedge,
                                );
                            }
                            if else_bb.0 == hdr {
                                check_backedge_arg(
                                    else_args,
                                    &mut max_step,
                                    &mut all_backedges_ok,
                                    &mut saw_backedge,
                                );
                            }
                        }
                        _ => {}
                    }
                    if !all_backedges_ok {
                        break;
                    }
                }
                if !all_backedges_ok || !saw_backedge || max_step == 0 {
                    continue;
                }
                (cand_idx, cand_lim, Some((i32::MAX as u32) - max_step))
            }
            _ => continue,
        };

        if ir.val_tys[idx.0 as usize] != Ty::I32 || ir.val_tys[limit.0 as usize] != Ty::I32 {
            continue;
        }
        let ldb = def_block[limit.0 as usize];
        if ldb == u32::MAX
            || body_set.contains(&ldb)
            || versioned_body_blocks.contains(&ldb)
            || !dom.dominates(ldb, pre)
        {
            continue;
        }

        // Symbolic bound propagation across `body \ {hdr}` in RPO order (`body_entry` dominates all).
        let is_limit_minus_1 = |v: Val| -> bool {
            match def_inst[v.0 as usize] {
                Some(Op::I32BinImm(IntBinOp::Sub, x, 1))
                | Some(Op::I32BinImm(IntBinOp::Add, x, -1))
                    if x == limit =>
                {
                    true
                }
                Some(Op::I32Bin(IntBinOp::Sub, x, y))
                    if x == limit && const_of(y, &def_inst) == Some(1) =>
                {
                    true
                }
                _ => false,
            }
        };
        let is_sub_one = |v: Val, x: Val| -> bool {
            match def_inst[v.0 as usize] {
                Some(Op::I32BinImm(IntBinOp::Sub, a0, 1))
                | Some(Op::I32BinImm(IntBinOp::Add, a0, -1))
                    if a0 == x =>
                {
                    true
                }
                Some(Op::I32Bin(IntBinOp::Sub, a0, b0))
                    if a0 == x && const_of(b0, &def_inst) == Some(1) =>
                {
                    true
                }
                _ => false,
            }
        };
        let get_bound = |v: Val, bounds: &HashMap<Val, LoopBound>| -> Option<LoopBound> {
            if let Some(&b) = bounds.get(&v) {
                return Some(b);
            }
            if v == idx || is_limit_minus_1(v) {
                return Some(LoopBound::ScaleAdd { shift: 0, add: 0 });
            }
            const_of(v, &def_inst).map(LoopBound::Const)
        };

        let mut bounds: HashMap<Val, LoopBound> = fx_map();
        let mut fold_add: HashMap<Val, (Val, u32)> = fx_map();

        // Proven memory operations: `(bb_id, inst_idx, ProvenMemOp)`.
        #[derive(Clone, Copy)]
        enum ProvenMemOp {
            LoadStore,
            MemCopy { dst: bool, src: bool },
            MemFill,
            BoundsCheck,
        }
        let mut proven_ops: HashMap<(u32, usize), ProvenMemOp> = fx_map();
        let mut fold_checked_ops: Vec<(u32, usize, u32)> = Vec::new();
        let mut max_safe_limit: u32 = signed_cap.unwrap_or(u32::MAX);

        let eval_cmp_select_bound = |kind: CmpBranchKind,
                                     ca: Val,
                                     cb: Val,
                                     tv: Val,
                                     fv: Val,
                                     bounds: &HashMap<Val, LoopBound>|
         -> Option<LoopBound> {
            let mut tb = get_bound(tv, bounds);
            let mut fb = get_bound(fv, bounds);

            // 1. Zero-guarded decrement: `(x == 0) ? tv : (x - 1)` or `(x != 0) ? (x - 1) : fv`.
            let eq_zero: Option<(Val, bool)> = match kind {
                CmpBranchKind::I32Imm(CmpOp::Eq, 0) => Some((ca, true)),
                CmpBranchKind::I32Imm(CmpOp::Ne, 0) => Some((ca, false)),
                CmpBranchKind::I32(CmpOp::Eq) if const_of(cb, &def_inst) == Some(0) => {
                    Some((ca, true))
                }
                CmpBranchKind::I32(CmpOp::Eq) if const_of(ca, &def_inst) == Some(0) => {
                    Some((cb, true))
                }
                CmpBranchKind::I32(CmpOp::Ne) if const_of(cb, &def_inst) == Some(0) => {
                    Some((ca, false))
                }
                CmpBranchKind::I32(CmpOp::Ne) if const_of(ca, &def_inst) == Some(0) => {
                    Some((cb, false))
                }
                _ => None,
            };
            if let Some((x, zero_on_true)) = eq_zero
                && let Some(xb) = get_bound(x, bounds)
            {
                if zero_on_true {
                    if is_sub_one(fv, x) {
                        fb = Some(xb);
                    }
                } else if is_sub_one(tv, x) {
                    tb = Some(xb);
                }
            }

            // 2. Limit-clamped arm: `(y >=u limit) ? tv : y` or `(y <u limit) ? y : fv`, plus
            //    general unsigned min/clamp against a bounded value.
            if let CmpBranchKind::I32(ccop) = kind {
                match ccop {
                    CmpOp::GeU => {
                        if cb == limit && fv == ca {
                            fb = Some(LoopBound::ScaleAdd { shift: 0, add: 0 });
                        } else if fv == ca
                            && let Some(bb) = get_bound(cb, bounds)
                        {
                            fb = Some(bb);
                        }
                    }
                    CmpOp::GtU => {
                        if ca == limit && tv == cb {
                            tb = Some(LoopBound::ScaleAdd { shift: 0, add: 0 });
                        } else if fv == ca
                            && let Some(bb) = get_bound(cb, bounds)
                        {
                            fb = Some(bb);
                        }
                    }
                    CmpOp::LtU => {
                        if cb == limit && tv == ca {
                            tb = Some(LoopBound::ScaleAdd { shift: 0, add: 0 });
                        } else if tv == ca
                            && let Some(bb) = get_bound(cb, bounds)
                        {
                            tb = Some(bb);
                        }
                    }
                    CmpOp::LeU => {
                        if ca == limit && fv == cb {
                            fb = Some(LoopBound::ScaleAdd { shift: 0, add: 0 });
                        } else if tv == ca
                            && let Some(bb) = get_bound(cb, bounds)
                        {
                            tb = Some(bb);
                        }
                    }
                    _ => {}
                }
            }

            match (tb, fb) {
                (Some(t), Some(f)) => Some(join_loop_bounds(t, f)),
                _ => None,
            }
        };

        for &bb_id in &body {
            if bb_id == hdr {
                continue;
            }
            for (ii, inst) in ir.blocks[bb_id as usize].insts.iter().enumerate() {
                // Propagate symbolic bounds for I32 definitions.
                if let Some(res) = inst.res
                    && ir.val_tys[res.0 as usize] == Ty::I32
                {
                    let computed: Option<LoopBound> = match inst.op {
                        Op::ConstI32(c) => Some(LoopBound::Const(u64::from(c as u32))),
                        Op::I32BinImm(IntBinOp::Add, x, imm) if imm >= 0 => {
                            let k = imm as u64;
                            match get_bound(x, &bounds) {
                                Some(LoopBound::Const(c)) => Some(LoopBound::Const(u64::from(
                                    (c as u32).wrapping_add(k as u32),
                                ))),
                                Some(LoopBound::ScaleAdd { shift, add }) => {
                                    if let Some(new_add) = add.checked_add(k)
                                        && new_add <= u64::from(u32::MAX)
                                    {
                                        if let Some(&(base0, k0)) = fold_add.get(&x) {
                                            if u64::from(k0) + k <= i32::MAX as u64 {
                                                fold_add.insert(res, (base0, k0 + k as u32));
                                            }
                                        } else if k <= i32::MAX as u64 {
                                            fold_add.insert(res, (x, k as u32));
                                        }
                                        Some(LoopBound::ScaleAdd {
                                            shift,
                                            add: new_add,
                                        })
                                    } else {
                                        None
                                    }
                                }
                                None => None,
                            }
                        }
                        Op::I32Bin(IntBinOp::Add, x, y) => {
                            let pair = if let Some(k) = const_of(y, &def_inst)
                                && k <= i32::MAX as u64
                            {
                                Some((x, k))
                            } else if let Some(k) = const_of(x, &def_inst)
                                && k <= i32::MAX as u64
                            {
                                Some((y, k))
                            } else {
                                None
                            };
                            if let Some((base, k)) = pair {
                                match get_bound(base, &bounds) {
                                    Some(LoopBound::Const(c)) => Some(LoopBound::Const(u64::from(
                                        (c as u32).wrapping_add(k as u32),
                                    ))),
                                    Some(LoopBound::ScaleAdd { shift, add }) => {
                                        if let Some(new_add) = add.checked_add(k)
                                            && new_add <= u64::from(u32::MAX)
                                        {
                                            if let Some(&(base0, k0)) = fold_add.get(&base) {
                                                if u64::from(k0) + k <= i32::MAX as u64 {
                                                    fold_add.insert(res, (base0, k0 + k as u32));
                                                }
                                            } else if k <= i32::MAX as u64 {
                                                fold_add.insert(res, (base, k as u32));
                                            }
                                            Some(LoopBound::ScaleAdd {
                                                shift,
                                                add: new_add,
                                            })
                                        } else {
                                            None
                                        }
                                    }
                                    None => None,
                                }
                            } else {
                                None
                            }
                        }
                        Op::I32BinImm(IntBinOp::Shl, x, imm) => {
                            let sh = (imm as u32) & 31;
                            if sh == 0 {
                                get_bound(x, &bounds)
                            } else if sh <= 8 {
                                match get_bound(x, &bounds) {
                                    Some(LoopBound::Const(c)) => Some(LoopBound::Const(u64::from(
                                        (c as u32).wrapping_shl(sh),
                                    ))),
                                    Some(LoopBound::ScaleAdd { shift, add }) if shift + sh <= 8 => {
                                        add.checked_shl(sh)
                                            .filter(|&a| a <= u64::from(u32::MAX))
                                            .map(|a| LoopBound::ScaleAdd {
                                                shift: shift + sh,
                                                add: a,
                                            })
                                    }
                                    _ => None,
                                }
                            } else {
                                None
                            }
                        }
                        Op::I32BinImm(IntBinOp::Mul, x, imm)
                            if imm > 0 && (imm as u32).is_power_of_two() =>
                        {
                            let sh = (imm as u32).trailing_zeros();
                            if sh == 0 {
                                get_bound(x, &bounds)
                            } else if sh <= 8 {
                                match get_bound(x, &bounds) {
                                    Some(LoopBound::Const(c)) => Some(LoopBound::Const(u64::from(
                                        (c as u32).wrapping_shl(sh),
                                    ))),
                                    Some(LoopBound::ScaleAdd { shift, add }) if shift + sh <= 8 => {
                                        add.checked_shl(sh)
                                            .filter(|&a| a <= u64::from(u32::MAX))
                                            .map(|a| LoopBound::ScaleAdd {
                                                shift: shift + sh,
                                                add: a,
                                            })
                                    }
                                    _ => None,
                                }
                            } else {
                                None
                            }
                        }
                        Op::I32BinImm(IntBinOp::And, _, mask) => {
                            Some(LoopBound::Const(u64::from(mask as u32)))
                        }
                        Op::I32Bin(IntBinOp::And, x, y) => const_of(y, &def_inst)
                            .or_else(|| const_of(x, &def_inst))
                            .map(LoopBound::Const),
                        Op::CmpSelect {
                            kind,
                            a: ca,
                            b: cb,
                            tv,
                            fv,
                        } => eval_cmp_select_bound(kind, ca, cb, tv, fv, &bounds),
                        Op::Select { cond, a: tv, b: fv } => match def_inst[cond.0 as usize] {
                            Some(Op::I32Eqz(x)) => eval_cmp_select_bound(
                                CmpBranchKind::I32Imm(CmpOp::Eq, 0),
                                x,
                                x,
                                tv,
                                fv,
                                &bounds,
                            ),
                            Some(Op::I32Cmp(ccop, ca, cb)) => eval_cmp_select_bound(
                                CmpBranchKind::I32(ccop),
                                ca,
                                cb,
                                tv,
                                fv,
                                &bounds,
                            ),
                            Some(Op::I32CmpImm(ccop, ca, imm)) => eval_cmp_select_bound(
                                CmpBranchKind::I32Imm(ccop, imm),
                                ca,
                                ca,
                                tv,
                                fv,
                                &bounds,
                            ),
                            _ => eval_cmp_select_bound(
                                CmpBranchKind::I32Imm(CmpOp::Ne, 0),
                                cond,
                                cond,
                                tv,
                                fv,
                                &bounds,
                            ),
                        },
                        _ => None,
                    };
                    if let Some(b) = computed {
                        bounds.insert(res, b);
                    }
                }

                // Check whether unchecked memory ops are provably bounded under `limit <= max_safe_limit`.
                match inst.op {
                    Op::Load {
                        kind,
                        addr,
                        offset,
                        checked: false,
                    } => {
                        let end_off = offset.saturating_add(kind.bytes());
                        if let Some(bnd) = get_bound(addr, &bounds)
                            && let Some(lim) = max_safe_limit_for_bound(bnd, end_off, mem_min_bytes)
                        {
                            max_safe_limit = max_safe_limit.min(lim);
                            proven_ops.insert((bb_id, ii), ProvenMemOp::LoadStore);
                        }
                    }
                    Op::Store {
                        kind,
                        addr,
                        offset,
                        checked: false,
                        ..
                    } => {
                        let end_off = offset.saturating_add(kind.bytes());
                        if let Some(bnd) = get_bound(addr, &bounds)
                            && let Some(lim) = max_safe_limit_for_bound(bnd, end_off, mem_min_bytes)
                        {
                            max_safe_limit = max_safe_limit.min(lim);
                            proven_ops.insert((bb_id, ii), ProvenMemOp::LoadStore);
                        }
                    }
                    Op::MemCopyConst {
                        dst,
                        src,
                        len,
                        dst_checked,
                        src_checked,
                    } if !dst_checked || !src_checked => {
                        let end_off = u64::from(len);
                        let mut p_dst = false;
                        let mut p_src = false;
                        if !dst_checked
                            && let Some(bnd) = get_bound(dst, &bounds)
                            && let Some(lim) = max_safe_limit_for_bound(bnd, end_off, mem_min_bytes)
                        {
                            max_safe_limit = max_safe_limit.min(lim);
                            p_dst = true;
                        }
                        if !src_checked
                            && let Some(bnd) = get_bound(src, &bounds)
                            && let Some(lim) = max_safe_limit_for_bound(bnd, end_off, mem_min_bytes)
                        {
                            max_safe_limit = max_safe_limit.min(lim);
                            p_src = true;
                        }
                        if p_dst || p_src {
                            proven_ops.insert(
                                (bb_id, ii),
                                ProvenMemOp::MemCopy {
                                    dst: p_dst,
                                    src: p_src,
                                },
                            );
                        }
                    }
                    Op::MemFillConst {
                        dst,
                        len,
                        checked: false,
                        ..
                    } => {
                        let end_off = u64::from(len);
                        if let Some(bnd) = get_bound(dst, &bounds)
                            && let Some(lim) = max_safe_limit_for_bound(bnd, end_off, mem_min_bytes)
                        {
                            max_safe_limit = max_safe_limit.min(lim);
                            proven_ops.insert((bb_id, ii), ProvenMemOp::MemFill);
                        }
                    }
                    Op::BoundsCheck { addr, end_off, .. } => {
                        if let Some(bnd) = get_bound(addr, &bounds)
                            && let Some(lim) = max_safe_limit_for_bound(bnd, end_off, mem_min_bytes)
                        {
                            max_safe_limit = max_safe_limit.min(lim);
                            proven_ops.insert((bb_id, ii), ProvenMemOp::BoundsCheck);
                        }
                    }
                    Op::Load {
                        kind: _,
                        addr,
                        offset,
                        checked: true,
                    }
                    | Op::Store {
                        kind: _,
                        addr,
                        offset,
                        checked: true,
                        ..
                    } if fold_add.contains_key(&addr) => {
                        let bytes = match inst.op {
                            Op::Load { kind, .. } => kind.bytes(),
                            Op::Store { kind, .. } => kind.bytes(),
                            _ => unreachable!(),
                        };
                        let end_off = offset.saturating_add(bytes);
                        if let Some(bnd) = get_bound(addr, &bounds)
                            && let Some(lim) = max_safe_limit_for_bound(bnd, end_off, mem_min_bytes)
                            && lim >= 4
                        {
                            fold_checked_ops.push((bb_id, ii, lim));
                        }
                    }
                    _ => {}
                }
            }
        }

        if proven_ops.is_empty() || max_safe_limit < 4 {
            continue;
        }
        for (bb_id, ii, lim) in fold_checked_ops {
            if max_safe_limit.min(lim) >= 4 {
                max_safe_limit = max_safe_limit.min(lim);
                proven_ops.insert((bb_id, ii), ProvenMemOp::LoadStore);
            }
        }

        // Check outside uses of values defined inside `body_set`: because both `hdr` and
        // `fast_hdr` will jump to `exit_bb`, any escaping value must be defined in `hdr` and
        // used solely inside `exit_bb`'s dominator subtree so we can promote it to `exit_bb.params`.
        let mut promoted_hdr_vals: Vec<Val> = Vec::new();
        let mut rewrite_blocks: Vec<usize> = Vec::new();
        let mut outside_ok = true;
        for b_idx in 0..ir.blocks.len() {
            ver_steps += 1;
            if let Some(bg) = budget
                && ver_steps >= 4096
            {
                ver_steps = 0;
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
                    if db != hdr || !dom.dominates(exit_bb.0, b_idx as u32) {
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
                inst.op.for_each_use(&ir.val_pool, |v| {
                    check_one(
                        v,
                        &mut outside_ok,
                        &mut block_uses_body,
                        &mut promoted_hdr_vals,
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
                            &mut promoted_hdr_vals,
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

        // Pre-check all IR caps BEFORE mutating any block or pool.
        let added_vals: usize = body
            .iter()
            .map(|&b| {
                ir.blocks[b as usize].params.len()
                    + ir.blocks[b as usize]
                        .insts
                        .iter()
                        .filter(|ins| ins.res.is_some())
                        .count()
            })
            .sum::<usize>()
            + promoted_hdr_vals.len();
        let mut needed_pool = entry_args.len as usize;
        for &bb_id in &body {
            match ir.blocks[bb_id as usize].term {
                Term::Jump { args, .. } => needed_pool += args.len as usize,
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
                    needed_pool += (then_args.len as usize) + (else_args.len as usize);
                }
                Term::Return { vals } => needed_pool += vals.len as usize,
                Term::Unreachable | Term::BrTable { .. } => {}
            }
        }
        if !promoted_hdr_vals.is_empty() {
            needed_pool += (exit_args.len as usize) + 2 * promoted_hdr_vals.len();
        }
        if ir.blocks.len() + body.len() > MAX_BLOCKS_PER_FN
            || ir.val_tys.len() + added_vals > MAX_IR_VALS_PER_FN
            || ir.val_pool.len() + needed_pool > MAX_VAL_POOL_PER_FN
        {
            continue;
        }

        // Allocate `fast_entry_args` and optional `hdr_exit_args_new` first.
        let entry_args_vec: Vec<Val> = ir.pool_slice(entry_args).to_vec();
        let Ok(fast_entry_args) = ir.push_pool(&entry_args_vec) else {
            continue;
        };
        let hdr_exit_args_new: Option<ValRange> = if promoted_hdr_vals.is_empty() {
            None
        } else {
            let mut v = Vec::with_capacity((exit_args.len as usize) + promoted_hdr_vals.len());
            v.extend_from_slice(ir.pool_slice(exit_args));
            v.extend_from_slice(&promoted_hdr_vals);
            let Ok(hr) = ir.push_pool(&v) else {
                continue;
            };
            Some(hr)
        };

        // Assign cloned BlockIds and cloned SSA Vals for every block in `body`.
        let base_new_block = ir.blocks.len() as u32;
        let mut block_map: HashMap<u32, BlockId> = fx_map_cap(body.len());
        for (i, &old_b) in body.iter().enumerate() {
            block_map.insert(old_b, BlockId(base_new_block + i as u32));
        }
        let mut val_map: HashMap<Val, Val> = fx_map_cap(added_vals);
        let mut new_params_per_block: Vec<Vec<Val>> = Vec::with_capacity(body.len());
        for &old_b in &body {
            let new_b = block_map[&old_b].0;
            let old_params: Vec<Val> = ir.blocks[old_b as usize].params.clone();
            let mut nparams = Vec::with_capacity(old_params.len());
            for p in old_params {
                let np = ir.push_val(ir.val_tys[p.0 as usize]);
                def_block.push(new_b);
                def_inst.push(None);
                val_map.insert(p, np);
                nparams.push(np);
            }
            new_params_per_block.push(nparams);
            let old_res_list: Vec<(Val, Op)> = ir.blocks[old_b as usize]
                .insts
                .iter()
                .filter_map(|ins| ins.res.map(|r| (r, ins.op)))
                .collect();
            for (r, _op) in old_res_list {
                let nr = ir.push_val(ir.val_tys[r.0 as usize]);
                def_block.push(new_b);
                def_inst.push(None);
                val_map.insert(r, nr);
            }
        }

        let map_val = |v: Val| -> Val { val_map.get(&v).copied().unwrap_or(v) };
        let map_range = |ir: &mut IrFunc, r: ValRange, extra: &[Val]| -> Res<ValRange> {
            if r.len == 0 && extra.is_empty() {
                return Ok(ValRange::EMPTY);
            }
            let mut v = Vec::with_capacity((r.len as usize) + extra.len());
            for &x in ir.pool_slice(r) {
                v.push(map_val(x));
            }
            for &x in extra {
                v.push(map_val(x));
            }
            ir.push_pool(&v)
        };

        // Construct cloned `fast_*` blocks with proven bounds checks elided and wrap-safe
        // address additions folded into `Load`/`Store` offsets.
        let mut fast_blocks: Vec<Block> = Vec::with_capacity(body.len());
        for (bi_idx, &old_b) in body.iter().enumerate() {
            let old_insts = ir.blocks[old_b as usize].insts.clone();
            let old_term = ir.blocks[old_b as usize].term;
            let old_preds = ir.blocks[old_b as usize].preds;
            let mut fast_insts = Vec::with_capacity(old_insts.len());
            for (ii, mut inst) in old_insts.into_iter().enumerate() {
                let orig_addr_opt = match inst.op {
                    Op::Load { addr, .. } | Op::Store { addr, .. } => Some(addr),
                    _ => None,
                };
                if let Some(r) = inst.res {
                    inst.res = Some(val_map[&r]);
                }
                remap_op_uses_map(&mut inst.op, &val_map);
                if old_b != hdr
                    && let Some(&proven) = proven_ops.get(&(old_b, ii))
                {
                    match (proven, &mut inst.op) {
                        (
                            ProvenMemOp::LoadStore,
                            Op::Load {
                                addr,
                                offset,
                                checked,
                                ..
                            },
                        )
                        | (
                            ProvenMemOp::LoadStore,
                            Op::Store {
                                addr,
                                offset,
                                checked,
                                ..
                            },
                        ) => {
                            *checked = true;
                            if let Some(orig_addr) = orig_addr_opt
                                && let Some(&(orig_base, k)) = fold_add.get(&orig_addr)
                                && (*offset).saturating_add(u64::from(k)) <= i32::MAX as u64
                            {
                                *addr = map_val(orig_base);
                                *offset += u64::from(k);
                            }
                        }
                        (
                            ProvenMemOp::MemCopy {
                                dst: p_dst,
                                src: p_src,
                            },
                            Op::MemCopyConst {
                                dst_checked,
                                src_checked,
                                ..
                            },
                        ) => {
                            if p_dst {
                                *dst_checked = true;
                            }
                            if p_src {
                                *src_checked = true;
                            }
                        }
                        (ProvenMemOp::MemFill, Op::MemFillConst { checked, .. }) => {
                            *checked = true;
                        }
                        (ProvenMemOp::BoundsCheck, Op::BoundsCheck { .. }) => {
                            continue;
                        }
                        _ => {}
                    }
                }
                if let Some(nr) = inst.res {
                    def_inst[nr.0 as usize] = Some(inst.op);
                }
                fast_insts.push(inst);
            }

            let fast_term = match old_term {
                Term::Jump { target, args } => Term::Jump {
                    target: block_map[&target.0],
                    args: map_range(ir, args, &[])?,
                },
                Term::BranchIf {
                    cond,
                    then_bb: tb,
                    then_args: ta,
                    else_bb: eb,
                    else_args: ea,
                } => Term::BranchIf {
                    cond: map_val(cond),
                    then_bb: block_map[&tb.0],
                    then_args: map_range(ir, ta, &[])?,
                    else_bb: block_map[&eb.0],
                    else_args: map_range(ir, ea, &[])?,
                },
                Term::CmpBranch {
                    kind,
                    a: ca,
                    b: cb,
                    then_bb: tb,
                    then_args: ta,
                    else_bb: eb,
                    else_args: ea,
                } => {
                    let na = map_val(ca);
                    let nb =
                        if matches!(kind, CmpBranchKind::I32Imm(..) | CmpBranchKind::I64Imm(..)) {
                            na
                        } else {
                            map_val(cb)
                        };
                    let (ntb, nta) = if let Some(&mapped) = block_map.get(&tb.0) {
                        (mapped, map_range(ir, ta, &[])?)
                    } else {
                        (tb, map_range(ir, ta, &promoted_hdr_vals)?)
                    };
                    let (neb, nea) = if let Some(&mapped) = block_map.get(&eb.0) {
                        (mapped, map_range(ir, ea, &[])?)
                    } else {
                        (eb, map_range(ir, ea, &promoted_hdr_vals)?)
                    };
                    Term::CmpBranch {
                        kind,
                        a: na,
                        b: nb,
                        then_bb: ntb,
                        then_args: nta,
                        else_bb: neb,
                        else_args: nea,
                    }
                }
                Term::Return { vals } => Term::Return {
                    vals: map_range(ir, vals, &[])?,
                },
                Term::Unreachable => Term::Unreachable,
                Term::BrTable { .. } => unreachable!("BrTable excluded above"),
            };

            fast_blocks.push(Block {
                params: std::mem::take(&mut new_params_per_block[bi_idx]),
                insts: fast_insts,
                term: fast_term,
                is_loop_header: old_b == hdr,
                preds: old_preds,
            });
        }
        ir.blocks.extend(fast_blocks);

        // Promote escaping header values to `exit_bb.params` and rewrite outside uses.
        if let Some(hr) = hdr_exit_args_new {
            let mut subst: Vec<Val> = (0..ir.val_tys.len() as u32).map(Val).collect();
            for &hp in &promoted_hdr_vals {
                let ty = ir.val_tys[hp.0 as usize];
                let np = ir.push_val(ty);
                def_block.push(exit_bb.0);
                def_inst.push(None);
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
                for inst in &mut ir.blocks[b_idx].insts {
                    rewrite_op_uses(&mut inst.op, &mut pool, &subst);
                }
                rewrite_term_uses(&mut ir.blocks[b_idx].term, &mut pool, &ir.br_tables, &subst);
            }
            ir.val_pool = pool;
        }

        // Dispatch from `pre` to `fast_hdr` when `limit <=u max_safe_limit`, else original `hdr`.
        let fast_hdr = block_map[&hdr];
        ir.blocks[pre as usize].term = Term::CmpBranch {
            kind: CmpBranchKind::I32Imm(CmpOp::LeU, max_safe_limit as i32),
            a: limit,
            b: limit,
            then_bb: fast_hdr,
            then_args: fast_entry_args,
            else_bb: BlockId(hdr),
            else_args: entry_args,
        };
        ir.blocks[exit_bb.0 as usize].preds = ir.blocks[exit_bb.0 as usize].preds.saturating_add(1);

        versioned_pres.insert(pre);
        versioned_exits.insert(exit_bb.0);
        for &b in &body {
            versioned_body_blocks.insert(b);
        }
        versioned_count += 1;
        versioned_any = true;
    }

    Ok(versioned_any)
}

/// Loop rotation: converts top-tested counted natural loops whose empty header block branches
/// via an integer `CmpBranch` into a single-pred `body_entry` and `exit_bb` into bottom-tested
/// loops with `hdr` acting as the pre-loop entry guard and `body_entry` becoming the new
/// `is_loop_header`. Eliminates the unconditional backedge `jmp` on every iteration.
fn rotate_counted_loops(
    ir: &mut IrFunc,
    dom: &CfgDom,
    budget: Option<&CompileBudget>,
) -> Res<bool> {
    if ir.blocks.len() <= 1 || dom.rpo.is_empty() {
        return Ok(false);
    }

    // Discover natural loops by backedge `b -> s` where `dom.dominates(s, b)`.
    let mut has_backedge = false;
    'chk_ver: for &b in &dom.rpo {
        let bp = dom.rpo_pos[b as usize];
        for &s in &dom.succs[b as usize] {
            if dom.rpo_pos[s as usize] <= bp && dom.dominates(s, b) {
                has_backedge = true;
                break 'chk_ver;
            }
        }
    }
    if !has_backedge {
        return Ok(false);
    }
    let mut loops_by_hdr: HashMap<u32, HashSet<u32>> = fx_map();
    for &b in &dom.rpo {
        let bp = dom.rpo_pos[b as usize];
        for &s in &dom.succs[b as usize] {
            if dom.rpo_pos[s as usize] <= bp && dom.dominates(s, b) {
                let body = loops_by_hdr.entry(s).or_insert_with(fx_set);
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

    let mut loops: Vec<(u32, Vec<u32>)> = loops_by_hdr
        .into_iter()
        .filter(|&(h, _)| h != 0)
        .map(|(h, set)| {
            let mut v: Vec<u32> = set.into_iter().collect();
            v.sort_by_key(|&b| dom.rpo_pos[b as usize]);
            (h, v)
        })
        .collect();
    loops.sort_by_key(|(h, body)| (body.len(), *h));

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

    let mut rotated_any = false;
    let mut touched: HashSet<u32> = fx_set();
    let mut scan_steps = 0usize;

    for (hdr, body) in loops {
        if let Some(bg) = budget {
            bg.check_deadline()?;
        }
        if body.len() < 2 || body.iter().any(|b| touched.contains(b)) {
            continue;
        }
        if !ir.blocks[hdr as usize].insts.is_empty() {
            continue;
        }
        let Term::CmpBranch {
            kind,
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
        if !matches!(
            kind,
            CmpBranchKind::I32(_)
                | CmpBranchKind::I32Imm(_, _)
                | CmpBranchKind::I64(_)
                | CmpBranchKind::I64Imm(_, _)
        ) {
            continue;
        }

        let body_set: HashSet<u32> = body.iter().copied().collect();
        let then_in = body_set.contains(&then_bb.0);
        let else_in = body_set.contains(&else_bb.0);
        if then_in == else_in {
            continue;
        }
        let (body_entry, body_args, exit_bb, exit_args, eff_kind) = if then_in {
            (then_bb.0, then_args, else_bb, else_args, kind)
        } else {
            let inv = match kind {
                CmpBranchKind::I32(c) => CmpBranchKind::I32(c.invert_int()),
                CmpBranchKind::I32Imm(c, imm) => CmpBranchKind::I32Imm(c.invert_int(), imm),
                CmpBranchKind::I64(c) => CmpBranchKind::I64(c.invert_int()),
                CmpBranchKind::I64Imm(c, imm) => CmpBranchKind::I64Imm(c.invert_int(), imm),
                _ => unreachable!(),
            };
            (else_bb.0, else_args, then_bb, then_args, inv)
        };
        if body_entry == hdr
            || exit_bb.0 == hdr
            || exit_bb.0 == 0
            || touched.contains(&exit_bb.0)
            || body_args.len != 0
            || !ir.blocks[body_entry as usize].params.is_empty()
            || dom.preds[body_entry as usize].len() != 1
            || dom.preds[body_entry as usize][0] != hdr
        {
            continue;
        }

        let outside_preds_any = dom.preds[hdr as usize]
            .iter()
            .any(|p| !body_set.contains(p));
        if !outside_preds_any {
            continue;
        }
        let back_preds: Vec<u32> = dom.preds[hdr as usize]
            .iter()
            .copied()
            .filter(|p| body_set.contains(p))
            .collect();
        if back_preds.is_empty() || back_preds.len() > 2 || back_preds.contains(&hdr) {
            continue;
        }
        let hdr_params_len = ir.blocks[hdr as usize].params.len();
        let mut latches: Vec<(u32, ValRange)> = Vec::with_capacity(back_preds.len());
        let mut latches_ok = true;
        for &lp in &back_preds {
            if let Term::Jump { target, args } = ir.blocks[lp as usize].term
                && target.0 == hdr
                && args.len as usize == hdr_params_len
            {
                latches.push((lp, args));
            } else {
                latches_ok = false;
                break;
            }
        }
        if !latches_ok {
            continue;
        }

        // Check outside uses of values defined inside `body_set`:
        // - If any value defined in `body \ {hdr}` escapes, skip.
        // - Any escaping `hdr.params` value must be used only under `exit_bb` with `preds[exit_bb] == [hdr]`
        //   so we can promote it to `exit_bb.params` (receiving `p_i` from `hdr` and `back_i` from each latch).
        let mut promoted_hdr_vals: Vec<Val> = Vec::new();
        let mut rewrite_blocks: Vec<usize> = Vec::new();
        let mut outside_ok = true;
        for b_idx in 0..ir.blocks.len() {
            scan_steps += 1;
            if let Some(bg) = budget
                && scan_steps >= 4096
            {
                scan_steps = 0;
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
                    if db != hdr
                        || dom.preds[exit_bb.0 as usize].len() != 1
                        || dom.preds[exit_bb.0 as usize][0] != hdr
                        || !dom.dominates(exit_bb.0, b_idx as u32)
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
                inst.op.for_each_use(&ir.val_pool, |v| {
                    check_one(
                        v,
                        &mut outside_ok,
                        &mut block_uses_body,
                        &mut promoted_hdr_vals,
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
                            &mut promoted_hdr_vals,
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

        // Pre-check IR caps before allocating vals or pool slices.
        let added_vals = hdr_params_len + promoted_hdr_vals.len();
        let exit_extra_len = (exit_args.len as usize) + promoted_hdr_vals.len();
        let needed_pool =
            hdr_params_len * (1 + latches.len()) + exit_extra_len * (1 + latches.len());
        if ir.val_tys.len() + added_vals > MAX_IR_VALS_PER_FN
            || ir.val_pool.len() + needed_pool > MAX_VAL_POOL_PER_FN
        {
            continue;
        }

        let hdr_params: Vec<Val> = ir.blocks[hdr as usize].params.clone();
        let hdr_param_pos: HashMap<Val, usize> = hdr_params
            .iter()
            .enumerate()
            .map(|(i, &p)| (p, i))
            .collect();
        let old_exit_args: Vec<Val> = ir.pool_slice(exit_args).to_vec();
        let old_latch_args: Vec<(u32, Vec<Val>)> = latches
            .iter()
            .map(|&(lp, r)| (lp, ir.pool_slice(r).to_vec()))
            .collect();

        // Create fresh `body_entry.params` mirroring `hdr.params`.
        let mut body_params: Vec<Val> = Vec::with_capacity(hdr_params_len);
        let mut body_val_map: HashMap<Val, Val> = fx_map_cap(hdr_params_len);
        for &hp in &hdr_params {
            let bp = ir.push_val(ir.val_tys[hp.0 as usize]);
            def_block.push(body_entry);
            body_params.push(bp);
            body_val_map.insert(hp, bp);
        }

        // Prepare pool slices for `hdr` and every `latch` before mutating blocks.
        let hdr_then_args = if hdr_params.is_empty() {
            ValRange::EMPTY
        } else {
            let Ok(r) = ir.push_pool(&hdr_params) else {
                continue;
            };
            r
        };
        let hdr_else_args = if promoted_hdr_vals.is_empty() {
            exit_args
        } else {
            let mut v = Vec::with_capacity(old_exit_args.len() + promoted_hdr_vals.len());
            v.extend_from_slice(&old_exit_args);
            v.extend_from_slice(&promoted_hdr_vals);
            let Ok(r) = ir.push_pool(&v) else {
                continue;
            };
            r
        };

        let mut latch_new_terms: Vec<(u32, Term)> = Vec::with_capacity(old_latch_args.len());
        let mut pool_ok = true;
        for (lp, orig_bargs) in &old_latch_args {
            let rew_bargs: Vec<Val> = orig_bargs
                .iter()
                .map(|&v| body_val_map.get(&v).copied().unwrap_or(v))
                .collect();
            let subst_hdr = |v: Val| -> Val {
                if let Some(&pos) = hdr_param_pos.get(&v) {
                    rew_bargs[pos]
                } else {
                    v
                }
            };
            let la = subst_hdr(a);
            let lb = if matches!(
                eff_kind,
                CmpBranchKind::I32Imm(_, _) | CmpBranchKind::I64Imm(_, _)
            ) {
                la
            } else {
                subst_hdr(b)
            };
            let latch_then_args = if rew_bargs.is_empty() {
                ValRange::EMPTY
            } else {
                match ir.push_pool(&rew_bargs) {
                    Ok(r) => r,
                    Err(_) => {
                        pool_ok = false;
                        break;
                    }
                }
            };
            let latch_else_args = if old_exit_args.is_empty() && promoted_hdr_vals.is_empty() {
                ValRange::EMPTY
            } else {
                let mut v = Vec::with_capacity(old_exit_args.len() + promoted_hdr_vals.len());
                for &ev in &old_exit_args {
                    v.push(subst_hdr(ev));
                }
                for &hp in &promoted_hdr_vals {
                    v.push(subst_hdr(hp));
                }
                match ir.push_pool(&v) {
                    Ok(r) => r,
                    Err(_) => {
                        pool_ok = false;
                        break;
                    }
                }
            };
            latch_new_terms.push((
                *lp,
                Term::CmpBranch {
                    kind: eff_kind,
                    a: la,
                    b: lb,
                    then_bb: BlockId(body_entry),
                    then_args: latch_then_args,
                    else_bb: exit_bb,
                    else_args: latch_else_args,
                },
            ));
        }
        if !pool_ok {
            continue;
        }

        // Rewrite all uses of `hdr.params` inside `body \ {hdr}` to `body_entry.params`.
        if !hdr_params.is_empty() {
            let mut subst: Vec<Val> = (0..ir.val_tys.len() as u32).map(Val).collect();
            for (&hp, &bp) in hdr_params.iter().zip(&body_params) {
                subst[hp.0 as usize] = bp;
            }
            let mut pool = std::mem::take(&mut ir.val_pool);
            for &bb_id in &body {
                if bb_id == hdr {
                    continue;
                }
                for inst in &mut ir.blocks[bb_id as usize].insts {
                    rewrite_op_uses(&mut inst.op, &mut pool, &subst);
                }
                rewrite_term_uses(
                    &mut ir.blocks[bb_id as usize].term,
                    &mut pool,
                    &ir.br_tables,
                    &subst,
                );
            }
            ir.val_pool = pool;
        }

        // Install rotated `body_entry` header, entry guard `hdr`, and bottom-tested `latch`es.
        ir.blocks[body_entry as usize].params = body_params;
        ir.blocks[body_entry as usize].is_loop_header = true;
        ir.blocks[hdr as usize].is_loop_header = false;
        ir.blocks[hdr as usize].term = Term::CmpBranch {
            kind: eff_kind,
            a,
            b,
            then_bb: BlockId(body_entry),
            then_args: hdr_then_args,
            else_bb: exit_bb,
            else_args: hdr_else_args,
        };
        for (lp, term) in latch_new_terms {
            ir.blocks[lp as usize].term = term;
        }

        // Promote escaping `hdr.params` values to `exit_bb.params` and rewrite outside uses.
        if !promoted_hdr_vals.is_empty() {
            let mut subst: Vec<Val> = (0..ir.val_tys.len() as u32).map(Val).collect();
            for &hp in &promoted_hdr_vals {
                let np = ir.push_val(ir.val_tys[hp.0 as usize]);
                def_block.push(exit_bb.0);
                ir.blocks[exit_bb.0 as usize].params.push(np);
                subst.push(np);
                subst[hp.0 as usize] = np;
            }
            let mut pool = std::mem::take(&mut ir.val_pool);
            for b_idx in rewrite_blocks {
                for inst in &mut ir.blocks[b_idx].insts {
                    rewrite_op_uses(&mut inst.op, &mut pool, &subst);
                }
                rewrite_term_uses(&mut ir.blocks[b_idx].term, &mut pool, &ir.br_tables, &subst);
            }
            ir.val_pool = pool;
        }

        for &bb_id in &body {
            touched.insert(bb_id);
        }
        if !promoted_hdr_vals.is_empty() {
            touched.insert(exit_bb.0);
        }
        rotated_any = true;
    }

    Ok(rotated_any)
}

/// Complete per-function Stage 2 IR optimization sequence (runs in parallel after inlining & effect analysis).
#[cfg(test)]
pub fn optimize_func_stage2(
    ir: &mut IrFunc,
    module: &X64Module,
    effects: &ModuleEffects,
    budget: Option<&CompileBudget>,
) -> Res<()> {
    let mut scratch = Stage2Scratch::default();
    optimize_func_stage2_with_scratch(ir, module, effects, budget, &mut scratch)
}

/// Complete per-function Stage 2 IR optimization sequence reusing worker-local `Stage2Scratch`.
pub fn optimize_func_stage2_with_scratch(
    ir: &mut IrFunc,
    module: &X64Module,
    effects: &ModuleEffects,
    budget: Option<&CompileBudget>,
    scratch: &mut Stage2Scratch,
) -> Res<()> {
    // 1. Compact any jump chains left by inlining and prune redundant/dead block params FIRST.
    //    When `ir.early_pruned != 0` (inlining did not touch this function), Phase 5a already
    //    compacted the CFG and pruned dead/redundant block params to fixed-point!
    let early_state = ir.early_pruned;
    let mut dom = if early_state != 0 {
        CfgDom::compute_with_budget(ir, budget)?
    } else {
        rebuild_cfg_and_prune(ir, budget)?
    };
    if ir.blocks.len() == 1 {
        let (_, gvn_changed) =
            run_dom_gvn_and_bce_with_scratch(ir, &dom, module, effects, budget, scratch)?;
        if gvn_changed || early_state != 2 {
            optimize_ir_with_budget_and_scratch(ir, budget, &mut scratch.opt_ir)?;
        }
        if slp_vectorize_f64x2(ir, &dom) {
            optimize_ir_with_budget_and_scratch(ir, budget, &mut scratch.opt_ir)?;
        }
        return Ok(());
    }
    if early_state == 0 {
        let _ = prune_block_params(ir, &dom, budget)?;
    }
    // 2. Convert trivial pure `if/else` diamonds into branchless `CmpSelect` and re-compact if any converted.
    let mut cfg_or_ir_mutated = early_state != 2;
    if if_convert_trivial_diamonds(ir) {
        cfg_or_ir_mutated = true;
        dom = rebuild_cfg_and_prune(ir, budget)?;
        if ir.blocks.len() == 1 {
            let _ = run_dom_gvn_and_bce_with_scratch(ir, &dom, module, effects, budget, scratch)?;
            optimize_ir_with_budget_and_scratch(ir, budget, &mut scratch.opt_ir)?;
            if slp_vectorize_f64x2(ir, &dom) {
                optimize_ir_with_budget_and_scratch(ir, budget, &mut scratch.opt_ir)?;
            }
            return Ok(());
        }
        let _ = prune_block_params(ir, &dom, budget)?;
    }
    // 3. LICM (only when natural loops exist) + Dominator GVN/BCE/Store-to-Load + peephole/DCE.
    let mut has_loops = ir.blocks.iter().any(|b| b.is_loop_header);
    if has_loops && run_licm(ir, &dom, module, effects, budget)? {
        cfg_or_ir_mutated = true;
        dom = rebuild_cfg_and_prune(ir, budget)?;
        has_loops = ir.blocks.iter().any(|b| b.is_loop_header);
    }
    let (folded_branch, gvn_mutated) =
        run_dom_gvn_and_bce_with_scratch(ir, &dom, module, effects, budget, scratch)?;
    let split_sc = split_short_circuit_branches(ir);
    let prev_blocks = ir.blocks.len();
    if cfg_or_ir_mutated || gvn_mutated || split_sc {
        optimize_ir_with_budget_and_scratch(ir, budget, &mut scratch.opt_ir)?;
    }
    let cfg_rebuilt = folded_branch
        || split_sc
        || (prev_blocks > 1 && ir.blocks.iter().skip(1).any(|b| b.preds == 0));
    if cfg_rebuilt {
        dom = rebuild_cfg_and_prune(ir, budget)?;
        has_loops = ir.blocks.iter().any(|b| b.is_loop_header);
    }
    // 4. Prune redundant/dead phis (and dead cross-block vals if CFG rebuilt) exposed by GVN/branch folding.
    if cfg_rebuilt
        || ((cfg_or_ir_mutated || gvn_mutated)
            && ir.blocks.iter().skip(1).any(|b| !b.params.is_empty()))
    {
        let _ = prune_block_params(ir, &dom, budget)?;
    }
    // 5-7. Loop-only passes (BCE versioning, loop rotation, boundary-clamp peeling, byte-sum SIMD):
    //      skip completely on acyclic functions!
    let mut post_rot_changed = false;
    if has_loops {
        if version_checked_counted_loops(ir, &dom, module, budget)? {
            optimize_ir_with_budget_and_scratch(ir, budget, &mut scratch.opt_ir)?;
            dom = rebuild_cfg_and_prune(ir, budget)?;
            if ir.blocks.iter().skip(1).any(|b| !b.params.is_empty()) {
                let _ = prune_block_params(ir, &dom, budget)?;
            }
        }
        if rotate_counted_loops(ir, &dom, budget)? {
            optimize_ir_with_budget_and_scratch(ir, budget, &mut scratch.opt_ir)?;
            dom = rebuild_cfg_and_prune(ir, budget)?;
            if ir.blocks.iter().skip(1).any(|b| !b.params.is_empty()) {
                let _ = prune_block_params(ir, &dom, budget)?;
            }
        }
        post_rot_changed = peel_boundary_clamped_loops(ir, module);
        if post_rot_changed {
            optimize_ir_with_budget_and_scratch(ir, budget, &mut scratch.opt_ir)?;
            dom = rebuild_cfg_and_prune(ir, budget)?;
            if ir.blocks.iter().skip(1).any(|b| !b.params.is_empty()) {
                let _ = prune_block_params(ir, &dom, budget)?;
            }
        }
        if vectorize_byte_sum_loops(ir, module) {
            post_rot_changed = true;
            optimize_ir_with_budget_and_scratch(ir, budget, &mut scratch.opt_ir)?;
            dom = rebuild_cfg_and_prune(ir, budget)?;
            if ir.blocks.iter().skip(1).any(|b| !b.params.is_empty()) {
                let _ = prune_block_params(ir, &dom, budget)?;
            }
        }
    }
    if slp_vectorize_f64x2(ir, &dom) {
        post_rot_changed = true;
        optimize_ir_with_budget_and_scratch(ir, budget, &mut scratch.opt_ir)?;
        dom = rebuild_cfg_and_prune(ir, budget)?;
        if ir.blocks.iter().skip(1).any(|b| !b.params.is_empty()) {
            let _ = prune_block_params(ir, &dom, budget)?;
        }
    }
    let _ = (post_rot_changed, dom);
    Ok(())
}

/// Peels the first (`i = 0`) and last (`i = limit - 1`) iterations of a rotated single-block
/// counted loop `for i in 0..limit` that contains boundary-clamping `CmpSelect` ops (`i == 0`
/// and/or `i + 1 >=u limit`), producing a clamp-free interior loop `for j in 0..(limit - 2)`
/// where `i - 1 == j`, `i == j + 1`, `i + 1 == j + 2` fold directly into checked load/store offsets.
fn peel_boundary_clamped_loops(ir: &mut IrFunc, module: &X64Module) -> bool {
    if module.memory64 {
        return false;
    }
    let orig_blocks = ir.blocks.len();
    let mut changed = false;

    for bi in 1..orig_blocks {
        if ir.blocks.len() + 5 > MAX_BLOCKS_PER_FN
            || ir.val_tys.len() + 128 > MAX_IR_VALS_PER_FN
            || ir.val_pool.len() + 64 > MAX_VAL_POOL_PER_FN
        {
            break;
        }
        let bb = &ir.blocks[bi];
        if !bb.is_loop_header || bb.params.len() != 1 || bb.insts.is_empty() || bb.insts.len() > 32
        {
            continue;
        }
        let iv_param = bb.params[0];
        if ir.val_tys[iv_param.0 as usize] != Ty::I32 {
            continue;
        }
        let Term::CmpBranch {
            kind,
            a,
            b,
            then_bb,
            then_args,
            else_bb,
            else_args,
        } = bb.term
        else {
            continue;
        };
        let bi_id = BlockId(bi as u32);
        if then_bb != bi_id || else_bb == bi_id || then_args.len != 1 || else_args.len != 0 {
            continue;
        }
        let next_i = ir.pool_slice(then_args)[0];
        // Require `next_i = iv_param + 1`.
        let next_i_ok = bb.insts.iter().any(|ins| {
            ins.res == Some(next_i)
                && matches!(ins.op, Op::I32BinImm(IntBinOp::Add, x, 1) if x == iv_param)
        });
        if !next_i_ok {
            continue;
        }
        // Require all instructions in `bi` to be single-result pure or checked loads/stores (no calls/traps).
        let insts_safe = bb.insts.iter().all(|ins| match ins.op {
            Op::Load { checked: true, .. } | Op::Store { checked: true, .. } => true,
            op => !op.has_side_effect(),
        });
        if !insts_safe {
            continue;
        }
        // Collect all defs inside `bi`.
        let mut bi_defs: Vec<Val> = Vec::with_capacity(bb.insts.len() + 1);
        bi_defs.push(iv_param);
        for ins in &bb.insts {
            if let Some(r) = ins.res {
                bi_defs.push(r);
            }
        }
        let is_bi_def = |v: Val| bi_defs.contains(&v);

        // Normalize continuation test to `next_i <u limit`.
        let limit_v = match kind {
            CmpBranchKind::I32(CmpOp::LtU) if a == next_i && !is_bi_def(b) => b,
            CmpBranchKind::I32(CmpOp::GtU) if b == next_i && !is_bi_def(a) => a,
            _ => continue,
        };

        // Check for at least one interior-foldable boundary `CmpSelect` (`i == 0` or `i + 1 >=u limit`).
        let mut const0_vals: Vec<Val> = Vec::new();
        for obb in &ir.blocks {
            for ins in &obb.insts {
                if let Some(r) = ins.res
                    && matches!(ins.op, Op::ConstI32(0))
                {
                    const0_vals.push(r);
                }
            }
        }
        let is_c0 = |v: Val| const0_vals.contains(&v);
        let classify_interior_select = |op: Op| -> Option<Val> {
            let Op::CmpSelect {
                kind: ck,
                a: ca,
                b: cb,
                tv,
                fv,
            } = op
            else {
                return None;
            };
            match ck {
                // `iv == 0` / `iv != 0` / `iv >u 0` / `iv <=u 0` (interior has `iv >= 1`).
                CmpBranchKind::I32Imm(CmpOp::Eq, 0) | CmpBranchKind::I32Imm(CmpOp::LeU, 0)
                    if ca == iv_param =>
                {
                    Some(fv)
                }
                CmpBranchKind::I32Imm(CmpOp::Ne, 0) | CmpBranchKind::I32Imm(CmpOp::GtU, 0)
                    if ca == iv_param =>
                {
                    Some(tv)
                }
                CmpBranchKind::I32(CmpOp::Eq) | CmpBranchKind::I32(CmpOp::LeU)
                    if ca == iv_param && is_c0(cb) =>
                {
                    Some(fv)
                }
                CmpBranchKind::I32(CmpOp::Eq) if cb == iv_param && is_c0(ca) => Some(fv),
                CmpBranchKind::I32(CmpOp::Ne) | CmpBranchKind::I32(CmpOp::GtU)
                    if ca == iv_param && is_c0(cb) =>
                {
                    Some(tv)
                }
                CmpBranchKind::I32(CmpOp::Ne) | CmpBranchKind::I32(CmpOp::LtU)
                    if cb == iv_param && is_c0(ca) =>
                {
                    Some(tv)
                }
                // `next_i >=u limit` / `next_i <u limit` / `next_i == limit` (interior has `next_i <u limit`).
                CmpBranchKind::I32(CmpOp::GeU) | CmpBranchKind::I32(CmpOp::Eq)
                    if ca == next_i && cb == limit_v =>
                {
                    Some(fv)
                }
                CmpBranchKind::I32(CmpOp::LtU) | CmpBranchKind::I32(CmpOp::Ne)
                    if ca == next_i && cb == limit_v =>
                {
                    Some(tv)
                }
                CmpBranchKind::I32(CmpOp::LeU) | CmpBranchKind::I32(CmpOp::Eq)
                    if ca == limit_v && cb == next_i =>
                {
                    Some(fv)
                }
                CmpBranchKind::I32(CmpOp::GtU) | CmpBranchKind::I32(CmpOp::Ne)
                    if ca == limit_v && cb == next_i =>
                {
                    Some(tv)
                }
                _ => None,
            }
        };
        if !bb
            .insts
            .iter()
            .any(|ins| classify_interior_select(ins.op).is_some())
        {
            continue;
        }

        // Verify no outside block uses any value defined in `bi` and no `BrTable` targets `bi`.
        let mut outside_preds = 0usize;
        let mut safe_cfg = true;
        for (ob, obb) in ir.blocks.iter().enumerate() {
            if ob == bi {
                continue;
            }
            for ins in &obb.insts {
                ins.op.for_each_use(&ir.val_pool, |v| {
                    if is_bi_def(v) {
                        safe_cfg = false;
                    }
                });
            }
            obb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
                if is_bi_def(v) {
                    safe_cfg = false;
                }
            });
            match obb.term {
                Term::Jump { target, args } if target == bi_id => {
                    if args.len != 1 {
                        safe_cfg = false;
                    }
                    outside_preds += 1;
                }
                Term::BranchIf {
                    then_bb: tb,
                    then_args: ta,
                    else_bb: eb,
                    else_args: ea,
                    ..
                }
                | Term::CmpBranch {
                    then_bb: tb,
                    then_args: ta,
                    else_bb: eb,
                    else_args: ea,
                    ..
                } => {
                    if tb == bi_id {
                        if ta.len != 1 {
                            safe_cfg = false;
                        }
                        outside_preds += 1;
                    }
                    if eb == bi_id {
                        if ea.len != 1 {
                            safe_cfg = false;
                        }
                        outside_preds += 1;
                    }
                }
                Term::BrTable { table_idx, .. }
                    if ir.br_tables[table_idx as usize]
                        .unique_edges
                        .iter()
                        .any(|&(t, _)| t == bi_id) =>
                {
                    safe_cfg = false;
                }
                _ => {}
            }
            if !safe_cfg {
                break;
            }
        }
        if !safe_cfg || outside_preds == 0 {
            continue;
        }

        let orig_insts: Vec<Inst> = ir.blocks[bi].insts.clone();

        // Helper to clone `orig_insts` substituting `iv_param -> iv_val` and optionally folding
        // boundary `CmpSelect`s + `(j, j+1, j+2)` load/store addressing for the interior loop.
        let clone_body = |iv_val: Val, interior_j: Option<Val>, ir_ref: &mut IrFunc| -> Vec<Inst> {
            let mut vmap: HashMap<Val, Val> = fx_map_cap(orig_insts.len() + 2);
            vmap.insert(iv_param, iv_val);
            let mut j_disp: HashMap<Val, u64> = fx_map();
            if let Some(jv) = interior_j {
                j_disp.insert(jv, 0);
                j_disp.insert(iv_val, 1);
            }
            let map_v = |v: Val, m: &HashMap<Val, Val>| m.get(&v).copied().unwrap_or(v);
            let mut out = Vec::with_capacity(orig_insts.len());
            for ins in &orig_insts {
                if interior_j.is_some()
                    && let Some(r) = ins.res
                    && let Some(chosen) = classify_interior_select(ins.op)
                {
                    let mapped = map_v(chosen, &vmap);
                    vmap.insert(r, mapped);
                    continue;
                }
                let mut op = ins.op;
                // Rewrite operand uses through `vmap` (all ops here have no ValRange args/rets).
                match &mut op {
                    Op::I32Eqz(x)
                    | Op::I64Eqz(x)
                    | Op::I32Clz(x)
                    | Op::I32Ctz(x)
                    | Op::I32Popcnt(x)
                    | Op::I64Clz(x)
                    | Op::I64Ctz(x)
                    | Op::I64Popcnt(x)
                    | Op::I32WrapI64(x)
                    | Op::I64ExtendI32S(x)
                    | Op::I64ExtendI32U(x)
                    | Op::I32Extend8S(x)
                    | Op::I32Extend16S(x)
                    | Op::I64Extend8S(x)
                    | Op::I64Extend16S(x)
                    | Op::I64Extend32S(x)
                    | Op::I32BinImm(_, x, _)
                    | Op::I64BinImm(_, x, _)
                    | Op::I32CmpImm(_, x, _)
                    | Op::I64CmpImm(_, x, _)
                    | Op::F32Un(_, x)
                    | Op::F64Un(_, x)
                    | Op::F32DemoteF64(x)
                    | Op::F64PromoteF32(x)
                    | Op::F32ConvertI32S(x)
                    | Op::F32ConvertI32U(x)
                    | Op::F32ConvertI64S(x)
                    | Op::F32ConvertI64U(x)
                    | Op::F64ConvertI32S(x)
                    | Op::F64ConvertI32U(x)
                    | Op::F64ConvertI64S(x)
                    | Op::F64ConvertI64U(x)
                    | Op::I32ReinterpretF32(x)
                    | Op::I64ReinterpretF64(x)
                    | Op::F32ReinterpretI32(x)
                    | Op::F64ReinterpretI64(x)
                    | Op::Trunc(_, x)
                    | Op::V128Splat { val: x, .. }
                    | Op::V128ExtractLane { v: x, .. }
                    | Op::V128Un { a: x, .. }
                    | Op::V128ToI32 { a: x, .. } => {
                        *x = map_v(*x, &vmap);
                    }
                    Op::I32Bin(_, x, y)
                    | Op::I64Bin(_, x, y)
                    | Op::I32Cmp(_, x, y)
                    | Op::I64Cmp(_, x, y)
                    | Op::F32Cmp(_, x, y)
                    | Op::F64Cmp(_, x, y)
                    | Op::F32Bin(_, x, y)
                    | Op::F64Bin(_, x, y)
                    | Op::V128ReplaceLane { v: x, x: y, .. }
                    | Op::V128Shuffle { a: x, b: y, .. }
                    | Op::V128Bin { a: x, b: y, .. }
                    | Op::V128Shift { v: x, shift: y, .. } => {
                        *x = map_v(*x, &vmap);
                        *y = map_v(*y, &vmap);
                    }
                    Op::Select { cond, a, b } => {
                        *cond = map_v(*cond, &vmap);
                        *a = map_v(*a, &vmap);
                        *b = map_v(*b, &vmap);
                    }
                    Op::CmpSelect {
                        kind: ck,
                        a,
                        b,
                        tv,
                        fv,
                    } => {
                        *a = map_v(*a, &vmap);
                        if !matches!(ck, CmpBranchKind::I32Imm(..) | CmpBranchKind::I64Imm(..)) {
                            *b = map_v(*b, &vmap);
                        } else {
                            *b = *a;
                        }
                        *tv = map_v(*tv, &vmap);
                        *fv = map_v(*fv, &vmap);
                    }
                    Op::Load { addr, offset, .. } => {
                        *addr = map_v(*addr, &vmap);
                        if let Some(jv) = interior_j
                            && let Some(&d) = j_disp.get(addr)
                            && let Some(noff) = offset.checked_add(d)
                            && noff <= i32::MAX as u64
                        {
                            *addr = jv;
                            *offset = noff;
                        }
                    }
                    Op::Store {
                        addr, val, offset, ..
                    } => {
                        *addr = map_v(*addr, &vmap);
                        *val = map_v(*val, &vmap);
                        if let Some(jv) = interior_j
                            && let Some(&d) = j_disp.get(addr)
                            && let Some(noff) = offset.checked_add(d)
                            && noff <= i32::MAX as u64
                        {
                            *addr = jv;
                            *offset = noff;
                        }
                    }
                    Op::V128LoadLane {
                        addr, vec, offset, ..
                    } => {
                        *addr = map_v(*addr, &vmap);
                        *vec = map_v(*vec, &vmap);
                        if let Some(jv) = interior_j
                            && let Some(&d) = j_disp.get(addr)
                            && let Some(noff) = offset.checked_add(d)
                            && noff <= i32::MAX as u64
                        {
                            *addr = jv;
                            *offset = noff;
                        }
                    }
                    Op::ConstI32(_)
                    | Op::ConstI64(_)
                    | Op::ConstF32(_)
                    | Op::ConstF64(_)
                    | Op::ConstV128(_)
                    | Op::GlobalGet(_)
                    | Op::MemSize => {}
                    _ => {}
                }
                // Track `j + k` affine relationships for interior load/store offset folding!
                if interior_j.is_some()
                    && let Some(r) = ins.res
                {
                    match op {
                        Op::I32BinImm(IntBinOp::Add, base, k) => {
                            if let Some(&d) = j_disp.get(&base) {
                                let sum = (d as i64) + i64::from(k);
                                if sum == 0 {
                                    vmap.insert(r, interior_j.unwrap());
                                    continue;
                                } else if (0..=4).contains(&sum) {
                                    let nr = ir_ref.push_val(ir_ref.val_tys[r.0 as usize]);
                                    vmap.insert(r, nr);
                                    j_disp.insert(nr, sum as u64);
                                    out.push(Inst { res: Some(nr), op });
                                    continue;
                                }
                            }
                        }
                        Op::I32BinImm(IntBinOp::Sub, base, k) => {
                            if let Some(&d) = j_disp.get(&base) {
                                let diff = (d as i64) - i64::from(k);
                                if diff == 0 {
                                    vmap.insert(r, interior_j.unwrap());
                                    continue;
                                } else if (0..=4).contains(&diff) {
                                    let nr = ir_ref.push_val(ir_ref.val_tys[r.0 as usize]);
                                    vmap.insert(r, nr);
                                    j_disp.insert(nr, diff as u64);
                                    out.push(Inst { res: Some(nr), op });
                                    continue;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                let new_res = ins.res.map(|r| {
                    let nr = ir_ref.push_val(ir_ref.val_tys[r.0 as usize]);
                    vmap.insert(r, nr);
                    nr
                });
                out.push(Inst { res: new_res, op });
            }
            out
        };

        let pre_peel_id = BlockId(ir.blocks.len() as u32);
        let pre_peel2_id = BlockId(pre_peel_id.0 + 1);
        let peel_first_id = BlockId(pre_peel_id.0 + 2);
        let interior_id = BlockId(pre_peel_id.0 + 3);
        let peel_last_id = BlockId(pre_peel_id.0 + 4);

        let p_init = ir.push_val(Ty::I32);
        let Ok(p_init_r1) = ir.push_pool(&[p_init]) else {
            continue;
        };
        let Ok(p_init_r2) = ir.push_pool(&[p_init]) else {
            continue;
        };
        let c0 = ir.push_val(Ty::I32);
        let int_lim = ir.push_val(Ty::I32);
        let Ok(j0_range) = ir.push_pool(&[c0]) else {
            continue;
        };
        let mut first_insts = vec![
            Inst {
                res: Some(c0),
                op: Op::ConstI32(0),
            },
            Inst {
                res: Some(int_lim),
                op: Op::I32BinImm(IntBinOp::Sub, limit_v, 2),
            },
        ];
        first_insts.extend(clone_body(c0, None, ir));

        let j_param = ir.push_val(Ty::I32);
        let j_plus_1 = ir.push_val(Ty::I32);
        let Ok(j_back_range) = ir.push_pool(&[j_plus_1]) else {
            continue;
        };
        let mut interior_insts = vec![Inst {
            res: Some(j_plus_1),
            op: Op::I32BinImm(IntBinOp::Add, j_param, 1),
        }];
        interior_insts.extend(clone_body(j_plus_1, Some(j_param), ir));

        let last_i = ir.push_val(Ty::I32);
        let mut last_insts = vec![Inst {
            res: Some(last_i),
            op: Op::I32BinImm(IntBinOp::Sub, limit_v, 1),
        }];
        last_insts.extend(clone_body(last_i, None, ir));

        // Retarget all outside edges entering `bi` -> `pre_peel_id`.
        for (ob, obb) in ir.blocks.iter_mut().enumerate() {
            if ob == bi {
                continue;
            }
            match &mut obb.term {
                Term::Jump { target, .. } => {
                    if *target == bi_id {
                        *target = pre_peel_id;
                    }
                }
                Term::BranchIf {
                    then_bb: tb,
                    else_bb: eb,
                    ..
                }
                | Term::CmpBranch {
                    then_bb: tb,
                    else_bb: eb,
                    ..
                } => {
                    if *tb == bi_id {
                        *tb = pre_peel_id;
                    }
                    if *eb == bi_id {
                        *eb = pre_peel_id;
                    }
                }
                _ => {}
            }
        }

        ir.blocks.push(Block {
            params: vec![p_init],
            insts: Vec::new(),
            term: Term::CmpBranch {
                kind: CmpBranchKind::I32Imm(CmpOp::Eq, 0),
                a: p_init,
                b: p_init,
                then_bb: pre_peel2_id,
                then_args: ValRange::EMPTY,
                else_bb: bi_id,
                else_args: p_init_r1,
            },
            is_loop_header: false,
            preds: outside_preds as u32,
        });
        ir.blocks.push(Block {
            params: Vec::new(),
            insts: Vec::new(),
            term: Term::CmpBranch {
                kind: CmpBranchKind::I32Imm(CmpOp::GeU, 3),
                a: limit_v,
                b: limit_v,
                then_bb: peel_first_id,
                then_args: ValRange::EMPTY,
                else_bb: bi_id,
                else_args: p_init_r2,
            },
            is_loop_header: false,
            preds: 1,
        });
        ir.blocks.push(Block {
            params: Vec::new(),
            insts: first_insts,
            term: Term::Jump {
                target: interior_id,
                args: j0_range,
            },
            is_loop_header: false,
            preds: 1,
        });
        ir.blocks.push(Block {
            params: vec![j_param],
            insts: interior_insts,
            term: Term::CmpBranch {
                kind: CmpBranchKind::I32(CmpOp::LtU),
                a: j_plus_1,
                b: int_lim,
                then_bb: interior_id,
                then_args: j_back_range,
                else_bb: peel_last_id,
                else_args: ValRange::EMPTY,
            },
            is_loop_header: true,
            preds: 2,
        });
        ir.blocks.push(Block {
            params: Vec::new(),
            insts: last_insts,
            term: Term::Jump {
                target: else_bb,
                args: ValRange::EMPTY,
            },
            is_loop_header: false,
            preds: 1,
        });
        changed = true;
    }

    changed
}

fn intern_v128_const(ir: &mut IrFunc, bytes: [u8; 16]) -> u32 {
    if let Some(pos) = ir.v128_consts.iter().position(|c| *c == bytes) {
        return pos as u32;
    }
    let idx = ir.v128_consts.len() as u32;
    ir.v128_consts.push(bytes);
    idx
}

/// Vectorizes bounds-check-free rotated byte-sum reduction loops (`acc += mem.u8[i + off]; i += 1;
/// i <u limit`) using 16-byte SSE2 `psadbw` + `paddq` chunks followed by the original scalar tail
/// loop for the `< 16` remainder. 100% wrap-proof and bit-exact modulo `2^32`.
fn vectorize_byte_sum_loops(ir: &mut IrFunc, module: &X64Module) -> bool {
    if module.memory64 {
        return false;
    }
    let orig_blocks = ir.blocks.len();
    let mut changed = false;

    for bi in 1..orig_blocks {
        if ir.blocks.len() + 4 > MAX_BLOCKS_PER_FN
            || ir.val_tys.len() + 32 > MAX_IR_VALS_PER_FN
            || ir.val_pool.len() + 96 > MAX_VAL_POOL_PER_FN
        {
            break;
        }
        let bb = &ir.blocks[bi];
        if !bb.is_loop_header || bb.params.len() < 2 || bb.insts.len() != 3 {
            continue;
        }
        let Term::CmpBranch {
            kind,
            a,
            b,
            then_bb,
            then_args,
            else_bb,
            else_args,
        } = bb.term
        else {
            continue;
        };
        let bi_id = BlockId(bi as u32);
        if then_bb != bi_id || else_bb == bi_id || then_args.len as usize != bb.params.len() {
            continue;
        }

        // Match the 3 loop instructions: checked `I32Load8U(iv, off)`, `acc + byte`, `iv + 1`.
        let mut load_info: Option<(Val, Val, u64)> = None; // (byte_v, addr, offset)
        let mut inc_info: Option<(Val, Val)> = None; // (next_i, iv)
        let mut add_info: Option<(Val, Val, Val)> = None; // (next_acc, x, y)
        for inst in &bb.insts {
            let Some(res) = inst.res else {
                load_info = None;
                break;
            };
            match inst.op {
                Op::Load {
                    kind: LoadKind::I32Load8U,
                    addr,
                    offset,
                    checked: true,
                } if load_info.is_none() => {
                    load_info = Some((res, addr, offset));
                }
                Op::I32BinImm(IntBinOp::Add, base, 1) if inc_info.is_none() => {
                    inc_info = Some((res, base));
                }
                Op::I32Bin(IntBinOp::Add, x, y) if add_info.is_none() => {
                    add_info = Some((res, x, y));
                }
                _ => {
                    load_info = None;
                    break;
                }
            }
        }
        let (Some((byte_v, load_addr, offset)), Some((next_i, iv_param)), Some((next_acc, ax, ay))) =
            (load_info, inc_info, add_info)
        else {
            continue;
        };
        if load_addr != iv_param || offset.checked_add(16).is_none() {
            continue;
        }
        let acc_param = if ax == byte_v {
            ay
        } else if ay == byte_v {
            ax
        } else {
            continue;
        };
        if iv_param == acc_param {
            continue;
        }
        let Some(iv_idx) = bb.params.iter().position(|&p| p == iv_param) else {
            continue;
        };
        let Some(acc_idx) = bb.params.iter().position(|&p| p == acc_param) else {
            continue;
        };
        if ir.val_tys[iv_param.0 as usize] != Ty::I32 || ir.val_tys[acc_param.0 as usize] != Ty::I32
        {
            continue;
        }

        // Verify backedge updates `(iv -> next_i, acc -> next_acc)` and leaves other params invariant.
        let then_slice = ir.pool_slice(then_args);
        if then_slice[iv_idx] != next_i || then_slice[acc_idx] != next_acc {
            continue;
        }
        let mut backedge_ok = true;
        for (j, (&p, &ta)) in bb.params.iter().zip(then_slice).enumerate() {
            if j != iv_idx && j != acc_idx && ta != p {
                backedge_ok = false;
                break;
            }
        }
        if !backedge_ok {
            continue;
        }

        // Collect all values defined inside `bi`.
        let mut bi_defs: Vec<Val> = Vec::with_capacity(bb.params.len() + 3);
        bi_defs.extend_from_slice(&bb.params);
        bi_defs.push(byte_v);
        bi_defs.push(next_i);
        bi_defs.push(next_acc);
        let is_bi_def = |v: Val| bi_defs.contains(&v);

        // Normalize loop continuation condition to `next_i <u limit`.
        let limit_spec: Result<Val, i32> = match kind {
            CmpBranchKind::I32(CmpOp::LtU) if a == next_i && !is_bi_def(b) => Ok(b),
            CmpBranchKind::I32(CmpOp::GtU) if b == next_i && !is_bi_def(a) => Ok(a),
            CmpBranchKind::I32Imm(CmpOp::LtU, imm) if a == next_i && imm > 0 => Err(imm),
            _ => continue,
        };

        // Ensure no outside block directly uses any value defined in `bi` and no `BrTable` targets `bi`.
        let mut outside_preds = 0usize;
        let mut safe_cfg = true;
        for (ob, obb) in ir.blocks.iter().enumerate() {
            if ob == bi {
                continue;
            }
            for inst in &obb.insts {
                inst.op.for_each_use(&ir.val_pool, |v| {
                    if is_bi_def(v) {
                        safe_cfg = false;
                    }
                });
            }
            obb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
                if is_bi_def(v) {
                    safe_cfg = false;
                }
            });
            match obb.term {
                Term::Jump { target, args } if target == bi_id => {
                    if args.len as usize != ir.blocks[bi].params.len() {
                        safe_cfg = false;
                    }
                    outside_preds += 1;
                }
                Term::BranchIf {
                    then_bb: tb,
                    then_args: ta,
                    else_bb: eb,
                    else_args: ea,
                    ..
                }
                | Term::CmpBranch {
                    then_bb: tb,
                    then_args: ta,
                    else_bb: eb,
                    else_args: ea,
                    ..
                } => {
                    if tb == bi_id {
                        if ta.len as usize != ir.blocks[bi].params.len() {
                            safe_cfg = false;
                        }
                        outside_preds += 1;
                    }
                    if eb == bi_id {
                        if ea.len as usize != ir.blocks[bi].params.len() {
                            safe_cfg = false;
                        }
                        outside_preds += 1;
                    }
                }
                Term::BrTable { table_idx, .. }
                    if ir.br_tables[table_idx as usize]
                        .unique_edges
                        .iter()
                        .any(|&(t, _)| t == bi_id) =>
                {
                    safe_cfg = false;
                }
                _ => {}
            }
            if !safe_cfg {
                break;
            }
        }
        if !safe_cfg || outside_preds == 0 {
            continue;
        }

        // Verify every `else_args` element can be materialized cleanly after the vector loop.
        let orig_else_args: Vec<Val> = ir.pool_slice(else_args).to_vec();
        let orig_params: Vec<Val> = ir.blocks[bi].params.clone();
        let else_ok = orig_else_args.iter().all(|&ev| {
            !is_bi_def(ev)
                || ev == next_acc
                || ev == next_i
                || orig_params
                    .iter()
                    .enumerate()
                    .any(|(j, &p)| j != iv_idx && j != acc_idx && p == ev)
        });
        if !else_ok {
            continue;
        }

        // Build `pre_vec` -> `pre_vec2` -> `vec_run` -> (`bi` | `else_bb`) using `Op::MemByteSum`
        // (lowered to 64B unrolled AVX2 `vpsadbw ymm`, 16B SSE2 `psadbw`, or 16B NEON on AArch64).
        let pre_vec_id = BlockId(ir.blocks.len() as u32);
        let pre_vec2_id = BlockId(pre_vec_id.0 + 1);
        let vec_run_id = BlockId(pre_vec_id.0 + 2);

        let mut pv: Vec<Val> = Vec::with_capacity(orig_params.len());
        for &op in &orig_params {
            pv.push(ir.push_val(ir.val_tys[op.0 as usize]));
        }
        let mut pre_vec_insts = Vec::new();
        let limit_v = match limit_spec {
            Ok(v) => v,
            Err(imm) => {
                let lv = ir.push_val(Ty::I32);
                pre_vec_insts.push(Inst {
                    res: Some(lv),
                    op: Op::ConstI32(imm),
                });
                lv
            }
        };
        let Ok(pv_range) = ir.push_pool(&pv) else {
            continue;
        };

        let rem_v = ir.push_val(Ty::I32);
        let vec_cnt_v = ir.push_val(Ty::I32);
        let Ok(pv_range2) = ir.push_pool(&pv) else {
            continue;
        };

        let vsum_v = ir.push_val(Ty::I32);
        let vi_next = ir.push_val(Ty::I32);
        let acc_after_v = ir.push_val(Ty::I32);
        let mut tail_args_vec = pv.clone();
        tail_args_vec[iv_idx] = vi_next;
        tail_args_vec[acc_idx] = acc_after_v;
        let Ok(tail_args_range) = ir.push_pool(&tail_args_vec) else {
            continue;
        };
        let mut done_exit_vec = Vec::with_capacity(orig_else_args.len());
        for &ev in &orig_else_args {
            if ev == next_acc {
                done_exit_vec.push(acc_after_v);
            } else if ev == next_i {
                done_exit_vec.push(vi_next);
            } else if let Some(j) = orig_params.iter().position(|&p| p == ev) {
                done_exit_vec.push(pv[j]);
            } else {
                done_exit_vec.push(ev);
            }
        }
        let Ok(done_exit_range) = ir.push_pool(&done_exit_vec) else {
            continue;
        };

        // Retarget outside edges currently entering `bi` -> `pre_vec_id`.
        for (ob, obb) in ir.blocks.iter_mut().enumerate() {
            if ob == bi {
                continue;
            }
            match &mut obb.term {
                Term::Jump { target, .. } => {
                    if *target == bi_id {
                        *target = pre_vec_id;
                    }
                }
                Term::BranchIf {
                    then_bb: tb,
                    else_bb: eb,
                    ..
                }
                | Term::CmpBranch {
                    then_bb: tb,
                    else_bb: eb,
                    ..
                } => {
                    if *tb == bi_id {
                        *tb = pre_vec_id;
                    }
                    if *eb == bi_id {
                        *eb = pre_vec_id;
                    }
                }
                _ => {}
            }
        }

        ir.blocks.push(Block {
            params: pv.clone(),
            insts: pre_vec_insts,
            term: Term::CmpBranch {
                kind: CmpBranchKind::I32(CmpOp::LtU),
                a: pv[iv_idx],
                b: limit_v,
                then_bb: pre_vec2_id,
                then_args: ValRange::EMPTY,
                else_bb: bi_id,
                else_args: pv_range,
            },
            is_loop_header: false,
            preds: outside_preds as u32,
        });
        ir.blocks.push(Block {
            params: Vec::new(),
            insts: vec![
                Inst {
                    res: Some(rem_v),
                    op: Op::I32Bin(IntBinOp::Sub, limit_v, pv[iv_idx]),
                },
                Inst {
                    res: Some(vec_cnt_v),
                    op: Op::I32BinImm(IntBinOp::And, rem_v, -16),
                },
            ],
            term: Term::CmpBranch {
                kind: CmpBranchKind::I32Imm(CmpOp::Ne, 0),
                a: vec_cnt_v,
                b: vec_cnt_v,
                then_bb: vec_run_id,
                then_args: ValRange::EMPTY,
                else_bb: bi_id,
                else_args: pv_range2,
            },
            is_loop_header: false,
            preds: 1,
        });
        ir.blocks.push(Block {
            params: Vec::new(),
            insts: vec![
                Inst {
                    res: Some(vsum_v),
                    op: Op::MemByteSum {
                        base: pv[iv_idx],
                        count: vec_cnt_v,
                        offset,
                    },
                },
                Inst {
                    res: Some(vi_next),
                    op: Op::I32Bin(IntBinOp::Add, pv[iv_idx], vec_cnt_v),
                },
                Inst {
                    res: Some(acc_after_v),
                    op: Op::I32Bin(IntBinOp::Add, pv[acc_idx], vsum_v),
                },
            ],
            term: Term::CmpBranch {
                kind: CmpBranchKind::I32(CmpOp::LtU),
                a: vi_next,
                b: limit_v,
                then_bb: bi_id,
                then_args: tail_args_range,
                else_bb,
                else_args: done_exit_range,
            },
            is_loop_header: false,
            preds: 1,
        });
        changed = true;
    }

    changed
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum SlpKind {
    Bin(u16),
    Un(u16),
}

#[derive(Clone, Copy)]
enum SlpOperandPlan {
    Existing(Val),
    Splat(Val),
    ConstPair(u64, u64),
}

#[inline]
fn is_nz_non_nan_f64_bits(bits: u64) -> bool {
    (bits & 0x7fff_ffff_ffff_ffff) != 0 && !f64::from_bits(bits).is_nan()
}

fn classify_slp_f64_op(op: Op, const_f64: &[Option<u64>]) -> Option<(SlpKind, Val, Option<Val>)> {
    let is_nz = |v: Val| {
        const_f64
            .get(v.0 as usize)
            .copied()
            .flatten()
            .is_some_and(is_nz_non_nan_f64_bits)
    };
    match op {
        Op::F64Bin(FloatBinOp::Add, a, b) => Some((SlpKind::Bin(0xfdf0), a, Some(b))),
        Op::F64Bin(FloatBinOp::Sub, a, b) => Some((SlpKind::Bin(0xfdf1), a, Some(b))),
        Op::F64Bin(FloatBinOp::Mul, a, b) => Some((SlpKind::Bin(0xfdf2), a, Some(b))),
        Op::F64Bin(FloatBinOp::Div, a, b) => Some((SlpKind::Bin(0xfdf3), a, Some(b))),
        Op::F64Un(FloatUnOp::Abs, a) => Some((SlpKind::Un(0xfdec), a, None)),
        Op::F64Un(FloatUnOp::Neg, a) => Some((SlpKind::Un(0xfded), a, None)),
        Op::F64Un(FloatUnOp::Sqrt, a) => Some((SlpKind::Un(0xfdef), a, None)),
        Op::F64Un(FloatUnOp::Ceil, a) => Some((SlpKind::Un(0xfd74), a, None)),
        Op::F64Un(FloatUnOp::Floor, a) => Some((SlpKind::Un(0xfd75), a, None)),
        Op::F64Un(FloatUnOp::Trunc, a) => Some((SlpKind::Un(0xfd7a), a, None)),
        Op::F64Un(FloatUnOp::Nearest, a) => Some((SlpKind::Un(0xfd94), a, None)),
        // Bit-exact IEEE-754 `pmin` (`0xfdf6`: `b < a ? b : a`) and `pmax` (`0xfdf7`: `b > a ? b : a`).
        Op::CmpSelect {
            kind: CmpBranchKind::F64(cop),
            a,
            b,
            tv,
            fv,
        } => match cop {
            CmpOp::LtS if tv == a && fv == b => Some((SlpKind::Bin(0xfdf6), b, Some(a))),
            CmpOp::GtS if tv == a && fv == b => Some((SlpKind::Bin(0xfdf7), b, Some(a))),
            CmpOp::LeS if tv == a && fv == b && is_nz(b) => {
                Some((SlpKind::Bin(0xfdf6), b, Some(a)))
            }
            CmpOp::GeS if tv == a && fv == b && is_nz(b) => {
                Some((SlpKind::Bin(0xfdf7), b, Some(a)))
            }
            CmpOp::LtS if tv == b && fv == a => Some((SlpKind::Bin(0xfdf7), a, Some(b))),
            CmpOp::GtS if tv == b && fv == a => Some((SlpKind::Bin(0xfdf6), a, Some(b))),
            CmpOp::LeS if tv == b && fv == a && is_nz(a) => {
                Some((SlpKind::Bin(0xfdf7), a, Some(b)))
            }
            CmpOp::GeS if tv == b && fv == a && is_nz(a) => {
                Some((SlpKind::Bin(0xfdf6), a, Some(b)))
            }
            _ => None,
        },
        _ => None,
    }
}

#[inline]
fn op_touches_linear_mem(op: &Op) -> bool {
    matches!(
        op,
        Op::BoundsCheck { .. }
            | Op::Load { .. }
            | Op::Store { .. }
            | Op::V128LoadLane { .. }
            | Op::V128StoreLane { .. }
            | Op::MemGrow(_)
            | Op::MemCopy { .. }
            | Op::MemFill { .. }
            | Op::MemCopyConst { .. }
            | Op::MemFillConst { .. }
            | Op::MemByteSum { .. }
            | Op::Call { .. }
            | Op::CallIndirect { .. }
            | Op::Trap(_)
    )
}

/// Bit-exact 128-bit SSE2 `f64x2` Superword-Level Parallelism (SLP) vectorizer:
/// 1. Packs adjacent checked `F64` load pairs `(addr, off)` + `(addr, off+8)` into `Load(V128)`.
/// 2. Propagates packed `f64x2` lanes through isomorphic IEEE-754 `Add/Sub/Mul/Div/Un/pmin/pmax`
///    ops (hoisting loop-invariant constant pairs and scalar splats into the immediate dominator).
/// 3. Packs paired loop-carried `F64` sum accumulators `(acc0 += x0, acc1 += x1)` in rotated
///    single-block loops into a single `V128` `addpd` accumulator (preserving exact iteration order!).
/// 4. Combines adjacent checked `F64` store pairs into a single `Store(V128)`.
fn slp_vectorize_f64x2(ir: &mut IrFunc, dom: &CfgDom) -> bool {
    // Fast-path exit: skip immediately on functions without >= 2 checked F64 loads in any block.
    let mut candidate_blocks: Vec<usize> = Vec::new();
    for (bi, bb) in ir.blocks.iter().enumerate() {
        let mut f64_loads = 0usize;
        for inst in &bb.insts {
            if matches!(
                inst.op,
                Op::Load {
                    kind: LoadKind::F64,
                    checked: true,
                    ..
                }
            ) {
                f64_loads += 1;
                if f64_loads >= 2 {
                    candidate_blocks.push(bi);
                    break;
                }
            }
        }
    }
    if candidate_blocks.is_empty() {
        return false;
    }

    // Collect global `ConstF64` bits and `def_block` for safe hoisting & constant-pair packing.
    let mut const_f64: Vec<Option<u64>> = vec![None; ir.val_tys.len()];
    let mut def_block: Vec<u32> = vec![u32::MAX; ir.val_tys.len()];
    for (bi, bb) in ir.blocks.iter().enumerate() {
        let bu = bi as u32;
        for &p in &bb.params {
            def_block[p.0 as usize] = bu;
        }
        for inst in &bb.insts {
            if let Some(r) = inst.res {
                def_block[r.0 as usize] = bu;
                if let Op::ConstF64(bits) = inst.op {
                    const_f64[r.0 as usize] = Some(bits);
                }
            }
            match inst.op {
                Op::Call { rets, .. } | Op::CallIndirect { rets, .. } => {
                    for &rv in ir.pool_slice(rets) {
                        def_block[rv.0 as usize] = bu;
                    }
                }
                _ => {}
            }
        }
    }

    let mut changed_any = false;

    for bi in candidate_blocks {
        if ir.val_tys.len() + 128 > MAX_IR_VALS_PER_FN
            || ir.val_pool.len() + 128 > MAX_VAL_POOL_PER_FN
            || ir.blocks.len() + 4 > MAX_BLOCKS_PER_FN
        {
            break;
        }

        // Optional loop-invariant hoist target: immediate dominator of a loop header.
        let hoist_bb: Option<u32> = if bi != 0 && ir.blocks[bi].is_loop_header {
            let id = dom.idom[bi];
            if id != u32::MAX && id as usize != bi && dom.dominates(id, bi as u32) {
                Some(id)
            } else {
                None
            }
        } else {
            None
        };

        let mut pair_of: HashMap<(Val, Val), Val> = fx_map();
        let mut splat_of: HashMap<Val, Val> = fx_map();
        let mut const_vec_of: HashMap<(u64, u64), Val> = fx_map();

        // Phase 1: Pack adjacent checked `F64` load pairs in the same memory-write epoch.
        let mut load_pairs: Vec<(usize, usize, Val, u64, Val, Val)> = Vec::new();
        {
            let insts = &ir.blocks[bi].insts;
            let mut mem_ep = 0u32;
            let mut open: Vec<(usize, u32, Val, u64, Val)> = Vec::new();
            let mut used = vec![false; insts.len()];
            for (idx, inst) in insts.iter().enumerate() {
                if let Op::Load {
                    kind: LoadKind::F64,
                    addr,
                    offset,
                    checked: true,
                } = inst.op
                    && let Some(res) = inst.res
                {
                    let mut matched = None;
                    for (oi, &(pidx, pep, paddr, poff, pres)) in open.iter().enumerate() {
                        if used[pidx] || pep != mem_ep || paddr != addr {
                            continue;
                        }
                        if poff.checked_add(8) == Some(offset) && poff.checked_add(16).is_some() {
                            matched = Some((oi, pidx, poff, pres, res));
                            break;
                        }
                        if offset.checked_add(8) == Some(poff) && offset.checked_add(16).is_some() {
                            matched = Some((oi, pidx, offset, res, pres));
                            break;
                        }
                    }
                    if let Some((oi, pidx, off_lo, vlo, vhi)) = matched {
                        used[pidx] = true;
                        used[idx] = true;
                        open.swap_remove(oi);
                        load_pairs.push((pidx, idx, addr, off_lo, vlo, vhi));
                    } else {
                        open.push((idx, mem_ep, addr, offset, res));
                    }
                } else if inst.op.has_side_effect() {
                    mem_ep = mem_ep.wrapping_add(1);
                }
            }
        }
        if load_pairs.is_empty() {
            continue;
        }

        // Apply Phase 1 load replacements.
        let mut replace_at: HashMap<usize, Vec<Inst>> = fx_map_cap(load_pairs.len());
        let mut remove_idx: HashSet<usize> = fx_set();
        for (first_idx, second_idx, addr, off_lo, vlo, vhi) in load_pairs {
            let vec_v = ir.push_val(Ty::V128);
            def_block.push(bi as u32);
            const_f64.push(None);
            pair_of.insert((vlo, vhi), vec_v);
            replace_at.insert(
                first_idx,
                vec![
                    Inst {
                        res: Some(vec_v),
                        op: Op::Load {
                            kind: LoadKind::V128,
                            addr,
                            offset: off_lo,
                            checked: true,
                        },
                    },
                    Inst {
                        res: Some(vlo),
                        op: Op::V128ExtractLane {
                            kind: SimdLaneKind::F64x2,
                            v: vec_v,
                            lane: 0,
                        },
                    },
                    Inst {
                        res: Some(vhi),
                        op: Op::V128ExtractLane {
                            kind: SimdLaneKind::F64x2,
                            v: vec_v,
                            lane: 1,
                        },
                    },
                ],
            );
            remove_idx.insert(second_idx);
        }
        {
            let old = std::mem::take(&mut ir.blocks[bi].insts);
            let mut rebuilt = Vec::with_capacity(old.len() + replace_at.len() * 2);
            for (idx, inst) in old.into_iter().enumerate() {
                if remove_idx.contains(&idx) {
                    continue;
                }
                if let Some(rep) = replace_at.remove(&idx) {
                    rebuilt.extend(rep);
                } else {
                    rebuilt.push(inst);
                }
            }
            ir.blocks[bi].insts = rebuilt;
        }
        changed_any = true;

        // Phase 2: Iteratively pair isomorphic scalar `F64` ops fed by packed pairs.
        for _sweep in 0..8 {
            if ir.val_tys.len() + 64 > MAX_IR_VALS_PER_FN {
                break;
            }
            let insts_len = ir.blocks[bi].insts.len();
            let mut pos_in_bb: HashMap<Val, usize> = fx_map_cap(insts_len);
            for (idx, inst) in ir.blocks[bi].insts.iter().enumerate() {
                if let Some(r) = inst.res {
                    pos_in_bb.insert(r, idx);
                }
            }
            struct PendingNode {
                idx: usize,
                res: Val,
                kind: SlpKind,
                op0: Val,
                op1: Option<Val>,
            }
            let mut pending: Vec<PendingNode> = Vec::new();
            let mut paired = vec![false; insts_len];
            struct MatchedSlpOp {
                i0: usize,
                i1: usize,
                place_at_i0: bool,
                kind: SlpKind,
                r0: Val,
                r1: Val,
                plan_a: SlpOperandPlan,
                plan_b: Option<SlpOperandPlan>,
            }
            let mut matched_ops: Vec<MatchedSlpOp> = Vec::new();

            let resolve_plan = |x0: Val, x1: Val, pairs: &HashMap<(Val, Val), Val>| {
                if let Some(&v) = pairs.get(&(x0, x1)) {
                    Some(SlpOperandPlan::Existing(v))
                } else if x0 == x1 {
                    Some(SlpOperandPlan::Splat(x0))
                } else if let (Some(b0), Some(b1)) = (
                    const_f64.get(x0.0 as usize).copied().flatten(),
                    const_f64.get(x1.0 as usize).copied().flatten(),
                ) {
                    Some(SlpOperandPlan::ConstPair(b0, b1))
                } else {
                    None
                }
            };

            let plan_def_pos =
                |plan: SlpOperandPlan, pos_map: &HashMap<Val, usize>| -> Option<usize> {
                    match plan {
                        SlpOperandPlan::Existing(v) => pos_map.get(&v).copied(),
                        SlpOperandPlan::Splat(s) => pos_map.get(&s).copied(),
                        SlpOperandPlan::ConstPair(_, _) => None,
                    }
                };

            for (idx, inst) in ir.blocks[bi].insts.iter().enumerate() {
                let Some(res) = inst.res else {
                    continue;
                };
                let Some((kind, op0, op1)) = classify_slp_f64_op(inst.op, &const_f64) else {
                    continue;
                };
                let mut found = None;
                for (pi, p) in pending.iter().enumerate() {
                    if paired[p.idx] || p.kind != kind || p.res == res {
                        continue;
                    }
                    let plans: Option<(SlpOperandPlan, Option<SlpOperandPlan>)> = match (p.op1, op1)
                    {
                        (None, None) => {
                            if let Some(&v) = pair_of.get(&(p.op0, op0)) {
                                Some((SlpOperandPlan::Existing(v), None))
                            } else {
                                None
                            }
                        }
                        (Some(pb), Some(cb)) => {
                            let direct = resolve_plan(p.op0, op0, &pair_of)
                                .zip(resolve_plan(pb, cb, &pair_of))
                                .filter(|(pa, pb_p)| {
                                    matches!(pa, SlpOperandPlan::Existing(_))
                                        || matches!(pb_p, SlpOperandPlan::Existing(_))
                                });
                            if let Some((pa, pb_p)) = direct {
                                Some((pa, Some(pb_p)))
                            } else if matches!(kind, SlpKind::Bin(0xfdf0 | 0xfdf2)) {
                                // Commutative `add`/`mul`: also try `(p.op0, cb)` and `(pb, op0)`.
                                resolve_plan(p.op0, cb, &pair_of)
                                    .zip(resolve_plan(pb, op0, &pair_of))
                                    .filter(|(pa, pb_p)| {
                                        matches!(pa, SlpOperandPlan::Existing(_))
                                            || matches!(pb_p, SlpOperandPlan::Existing(_))
                                    })
                                    .map(|(pa, pb_p)| (pa, Some(pb_p)))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    let Some((plan_a, plan_b)) = plans else {
                        continue;
                    };

                    // Verify safe SSA placement at `p.idx` (preferred) or `idx`.
                    let max_dep = [
                        plan_def_pos(plan_a, &pos_in_bb),
                        plan_b.and_then(|pb_p| plan_def_pos(pb_p, &pos_in_bb)),
                    ]
                    .into_iter()
                    .flatten()
                    .max();
                    let can_place_at_i0 = max_dep.is_none_or(|d| d < p.idx);
                    if can_place_at_i0 {
                        found = Some((pi, p.idx, true, p.res, plan_a, plan_b));
                        break;
                    }
                    // Check if `p.res` has zero uses in `(p.idx + 1)..=idx`.
                    let mut p_used_between = false;
                    for mid in &ir.blocks[bi].insts[(p.idx + 1)..=idx] {
                        mid.op.for_each_use(&ir.val_pool, |u| {
                            if u == p.res {
                                p_used_between = true;
                            }
                        });
                        if p_used_between {
                            break;
                        }
                    }
                    if !p_used_between && max_dep.is_none_or(|d| d < idx) {
                        found = Some((pi, p.idx, false, p.res, plan_a, plan_b));
                        break;
                    }
                }
                if let Some((pi, i0, place_at_i0, r0, plan_a, plan_b)) = found {
                    paired[i0] = true;
                    paired[idx] = true;
                    pending.swap_remove(pi);
                    matched_ops.push(MatchedSlpOp {
                        i0,
                        i1: idx,
                        place_at_i0,
                        kind,
                        r0,
                        r1: res,
                        plan_a,
                        plan_b,
                    });
                } else {
                    pending.push(PendingNode {
                        idx,
                        res,
                        kind,
                        op0,
                        op1,
                    });
                }
            }
            if matched_ops.is_empty() {
                break;
            }

            let mut rep_map: HashMap<usize, Vec<Inst>> = fx_map_cap(matched_ops.len());
            let mut del_set: HashSet<usize> = fx_set();
            for MatchedSlpOp {
                i0,
                i1,
                place_at_i0,
                kind,
                r0,
                r1,
                plan_a,
                plan_b,
            } in matched_ops
            {
                let mut local_prefix: Vec<Inst> = Vec::new();
                let mut materialize = |plan: SlpOperandPlan, prefix: &mut Vec<Inst>| -> Val {
                    match plan {
                        SlpOperandPlan::Existing(v) => v,
                        SlpOperandPlan::ConstPair(b0, b1) => {
                            if let Some(&cv) = const_vec_of.get(&(b0, b1)) {
                                return cv;
                            }
                            let mut bytes = [0u8; 16];
                            bytes[0..8].copy_from_slice(&b0.to_le_bytes());
                            bytes[8..16].copy_from_slice(&b1.to_le_bytes());
                            let cidx = intern_v128_const(ir, bytes);
                            let cv = ir.push_val(Ty::V128);
                            const_f64.push(None);
                            if let Some(hb) = hoist_bb {
                                def_block.push(hb);
                                ir.blocks[hb as usize].insts.push(Inst {
                                    res: Some(cv),
                                    op: Op::ConstV128(cidx),
                                });
                                const_vec_of.insert((b0, b1), cv);
                            } else {
                                def_block.push(bi as u32);
                                prefix.push(Inst {
                                    res: Some(cv),
                                    op: Op::ConstV128(cidx),
                                });
                            }
                            cv
                        }
                        SlpOperandPlan::Splat(s) => {
                            if let Some(bits) = const_f64.get(s.0 as usize).copied().flatten() {
                                if let Some(&cv) = const_vec_of.get(&(bits, bits)) {
                                    return cv;
                                }
                                let mut bytes = [0u8; 16];
                                bytes[0..8].copy_from_slice(&bits.to_le_bytes());
                                bytes[8..16].copy_from_slice(&bits.to_le_bytes());
                                let cidx = intern_v128_const(ir, bytes);
                                let cv = ir.push_val(Ty::V128);
                                const_f64.push(None);
                                if let Some(hb) = hoist_bb {
                                    def_block.push(hb);
                                    ir.blocks[hb as usize].insts.push(Inst {
                                        res: Some(cv),
                                        op: Op::ConstV128(cidx),
                                    });
                                    const_vec_of.insert((bits, bits), cv);
                                } else {
                                    def_block.push(bi as u32);
                                    prefix.push(Inst {
                                        res: Some(cv),
                                        op: Op::ConstV128(cidx),
                                    });
                                }
                                return cv;
                            }
                            if let Some(&sv) = splat_of.get(&s) {
                                return sv;
                            }
                            let sv = ir.push_val(Ty::V128);
                            const_f64.push(None);
                            let s_db = def_block[s.0 as usize];
                            let can_hoist = hoist_bb.is_some_and(|hb| {
                                s_db != u32::MAX && s_db != bi as u32 && dom.dominates(s_db, hb)
                            });
                            if can_hoist {
                                let hb = hoist_bb.unwrap();
                                def_block.push(hb);
                                ir.blocks[hb as usize].insts.push(Inst {
                                    res: Some(sv),
                                    op: Op::V128Splat {
                                        kind: SimdSplatKind::F64x2,
                                        val: s,
                                    },
                                });
                                splat_of.insert(s, sv);
                            } else {
                                def_block.push(bi as u32);
                                prefix.push(Inst {
                                    res: Some(sv),
                                    op: Op::V128Splat {
                                        kind: SimdSplatKind::F64x2,
                                        val: s,
                                    },
                                });
                            }
                            sv
                        }
                    }
                };
                let va = materialize(plan_a, &mut local_prefix);
                let vb_opt = plan_b.map(|pb_p| materialize(pb_p, &mut local_prefix));
                let vec_res = ir.push_val(Ty::V128);
                def_block.push(bi as u32);
                const_f64.push(None);
                let vec_op = match (kind, vb_opt) {
                    (SlpKind::Bin(op), Some(vb)) => Op::V128Bin { op, a: va, b: vb },
                    (SlpKind::Un(op), None) => Op::V128Un { op, a: va },
                    _ => unreachable!(),
                };
                local_prefix.push(Inst {
                    res: Some(vec_res),
                    op: vec_op,
                });
                local_prefix.push(Inst {
                    res: Some(r0),
                    op: Op::V128ExtractLane {
                        kind: SimdLaneKind::F64x2,
                        v: vec_res,
                        lane: 0,
                    },
                });
                local_prefix.push(Inst {
                    res: Some(r1),
                    op: Op::V128ExtractLane {
                        kind: SimdLaneKind::F64x2,
                        v: vec_res,
                        lane: 1,
                    },
                });
                pair_of.insert((r0, r1), vec_res);
                if place_at_i0 {
                    rep_map.insert(i0, local_prefix);
                    del_set.insert(i1);
                } else {
                    del_set.insert(i0);
                    rep_map.insert(i1, local_prefix);
                }
            }
            let old = std::mem::take(&mut ir.blocks[bi].insts);
            let mut rebuilt = Vec::with_capacity(old.len() + rep_map.len() * 3);
            for (idx, inst) in old.into_iter().enumerate() {
                if del_set.contains(&idx) {
                    continue;
                }
                if let Some(rep) = rep_map.remove(&idx) {
                    rebuilt.extend(rep);
                } else {
                    rebuilt.push(inst);
                }
            }
            ir.blocks[bi].insts = rebuilt;
        }

        // Phase 3: Pack paired loop-carried `F64` sum accumulators in rotated single-block loops.
        if bi != 0
            && ir.blocks[bi].is_loop_header
            && ir.blocks[bi].params.len() >= 2
            && ir.blocks.len() < MAX_BLOCKS_PER_FN
            && let Term::CmpBranch {
                kind,
                a,
                b,
                then_bb,
                then_args,
                else_bb,
                else_args,
            } = ir.blocks[bi].term
            && then_bb.0 as usize == bi
            && else_bb.0 as usize != bi
        {
            let bi_id = BlockId(bi as u32);
            let has_br_table_pred = ir.blocks.iter().enumerate().any(|(ob, obb)| {
                ob != bi
                    && matches!(obb.term, Term::BrTable { table_idx, .. }
                        if ir.br_tables[table_idx as usize].unique_edges.iter().any(|&(t, _)| t == bi_id))
            });
            if !has_br_table_pred {
                let params_snap: Vec<Val> = ir.blocks[bi].params.clone();
                let then_snap: Vec<Val> = ir.pool_slice(then_args).to_vec();
                let else_snap: Vec<Val> = ir.pool_slice(else_args).to_vec();
                // Find candidate scalar accumulators: `param[k]` (`F64`) updated by `next = param[k] + d`.
                let mut acc_cands: Vec<(usize, Val, Val, Val)> = Vec::new(); // (k, p, next, delta)
                for (k, (&p, &nxt)) in params_snap.iter().zip(&then_snap).enumerate() {
                    if ir.val_tys[p.0 as usize] != Ty::F64 || nxt == p {
                        continue;
                    }
                    let Some(delta) = ir.blocks[bi].insts.iter().find_map(|ins| {
                        if ins.res != Some(nxt) {
                            return None;
                        }
                        match ins.op {
                            Op::F64Bin(FloatBinOp::Add, x, y) if x == p => Some(y),
                            Op::F64Bin(FloatBinOp::Add, x, y) if y == p => Some(x),
                            _ => None,
                        }
                    }) else {
                        continue;
                    };
                    // Verify `p` is used ONLY by `nxt` inside `bi` (and optionally `else_args`),
                    // `nxt` is used ONLY by `then_args[k]` (and optionally `else_args`),
                    // and neither `p` nor `nxt` is used by any outside block.
                    let mut p_ok = true;
                    let mut nxt_ok = true;
                    for ins in &ir.blocks[bi].insts {
                        let is_nxt = ins.res == Some(nxt);
                        ins.op.for_each_use(&ir.val_pool, |u| {
                            if u == p && !is_nxt {
                                p_ok = false;
                            }
                            if u == nxt {
                                nxt_ok = false;
                            }
                        });
                    }
                    if !p_ok || !nxt_ok {
                        continue;
                    }
                    // Check terminator comparison operands & other `then_args` slots.
                    if a == p || a == nxt || b == p || b == nxt {
                        continue;
                    }
                    if then_snap
                        .iter()
                        .enumerate()
                        .any(|(j, &v)| j != k && (v == p || v == nxt))
                    {
                        continue;
                    }
                    let mut outside_used = false;
                    for (ob, obb) in ir.blocks.iter().enumerate() {
                        if ob == bi {
                            continue;
                        }
                        for ins in &obb.insts {
                            ins.op.for_each_use(&ir.val_pool, |u| {
                                if u == p || u == nxt {
                                    outside_used = true;
                                }
                            });
                        }
                        obb.term.for_each_use(&ir.val_pool, &ir.br_tables, |u| {
                            if u == p || u == nxt {
                                outside_used = true;
                            }
                        });
                        if outside_used {
                            break;
                        }
                    }
                    if !outside_used {
                        acc_cands.push((k, p, nxt, delta));
                    }
                }

                // Pair accumulators whose deltas `(d0, d1)` are already packed in `pair_of`.
                let mut used_cand = vec![false; acc_cands.len()];
                let mut acc_pairs: Vec<(usize, usize, Val, Val, Val, Val, Val)> = Vec::new();
                for i in 0..acc_cands.len() {
                    if used_cand[i] {
                        continue;
                    }
                    let (k0, p0, n0, d0) = acc_cands[i];
                    for j in (i + 1)..acc_cands.len() {
                        if used_cand[j] {
                            continue;
                        }
                        let (k1, p1, n1, d1) = acc_cands[j];
                        if let Some(&d_vec) = pair_of.get(&(d0, d1)) {
                            used_cand[i] = true;
                            used_cand[j] = true;
                            acc_pairs.push((k0, k1, p0, p1, n0, n1, d_vec));
                            break;
                        } else if let Some(&d_vec) = pair_of.get(&(d1, d0)) {
                            used_cand[i] = true;
                            used_cand[j] = true;
                            acc_pairs.push((k1, k0, p1, p0, n1, n0, d_vec));
                            break;
                        }
                    }
                }

                if !acc_pairs.is_empty()
                    && ir.val_tys.len() + acc_pairs.len() * 12 <= MAX_IR_VALS_PER_FN
                    && ir.val_pool.len()
                        + (params_snap.len() + acc_pairs.len()) * (ir.blocks.len().min(16) + 2)
                        + else_snap.len()
                        <= MAX_VAL_POOL_PER_FN
                {
                    let mut new_then = then_snap.clone();
                    let mut exit_insts: Vec<Inst> = Vec::new();
                    let mut exit_subst: HashMap<Val, Val> = fx_map();
                    let mut packed_specs: Vec<(usize, usize, Val, Val, Val)> = Vec::new(); // (k0, k1, p_vec, next_vec, d_vec)

                    for &(k0, k1, p0, p1, n0, n1, d_vec) in &acc_pairs {
                        let p_vec = ir.push_val(Ty::V128);
                        def_block.push(bi as u32);
                        const_f64.push(None);
                        let next_vec = ir.push_val(Ty::V128);
                        def_block.push(bi as u32);
                        const_f64.push(None);

                        // Turn dead scalar loop carries into self-copies so `prune_block_params` drops them!
                        new_then[k0] = p0;
                        new_then[k1] = p1;
                        new_then.push(next_vec);
                        packed_specs.push((k0, k1, p_vec, next_vec, d_vec));

                        // Extract lanes lazily in `loop_exit_tramp` ONLY if `else_args` uses them.
                        if else_snap.contains(&n0) {
                            let e0 = ir.push_val(Ty::F64);
                            def_block.push(u32::MAX);
                            const_f64.push(None);
                            exit_insts.push(Inst {
                                res: Some(e0),
                                op: Op::V128ExtractLane {
                                    kind: SimdLaneKind::F64x2,
                                    v: next_vec,
                                    lane: 0,
                                },
                            });
                            exit_subst.insert(n0, e0);
                        }
                        if else_snap.contains(&n1) {
                            let e1 = ir.push_val(Ty::F64);
                            def_block.push(u32::MAX);
                            const_f64.push(None);
                            exit_insts.push(Inst {
                                res: Some(e1),
                                op: Op::V128ExtractLane {
                                    kind: SimdLaneKind::F64x2,
                                    v: next_vec,
                                    lane: 1,
                                },
                            });
                            exit_subst.insert(n1, e1);
                        }
                        if else_snap.contains(&p0) {
                            let e0 = ir.push_val(Ty::F64);
                            def_block.push(u32::MAX);
                            const_f64.push(None);
                            exit_insts.push(Inst {
                                res: Some(e0),
                                op: Op::V128ExtractLane {
                                    kind: SimdLaneKind::F64x2,
                                    v: p_vec,
                                    lane: 0,
                                },
                            });
                            exit_subst.insert(p0, e0);
                        }
                        if else_snap.contains(&p1) {
                            let e1 = ir.push_val(Ty::F64);
                            def_block.push(u32::MAX);
                            const_f64.push(None);
                            exit_insts.push(Inst {
                                res: Some(e1),
                                op: Op::V128ExtractLane {
                                    kind: SimdLaneKind::F64x2,
                                    v: p_vec,
                                    lane: 1,
                                },
                            });
                            exit_subst.insert(p1, e1);
                        }
                    }

                    // Prepare outside predecessor edge updates BEFORE mutating `ir.blocks[bi].params`.
                    let n_existing = ir.blocks.len();
                    let mut edge_updates: Vec<(usize, bool, ValRange, Vec<Inst>)> = Vec::new();
                    let mut edges_ok = true;
                    for ob in 0..n_existing {
                        if ob == bi {
                            continue;
                        }
                        let mut targets: Vec<(bool, ValRange)> = Vec::new();
                        match ir.blocks[ob].term {
                            Term::Jump { target, args } if target == bi_id => {
                                targets.push((true, args));
                            }
                            Term::BranchIf {
                                then_bb: tb,
                                then_args: ta,
                                else_bb: eb,
                                else_args: ea,
                                ..
                            }
                            | Term::CmpBranch {
                                then_bb: tb,
                                then_args: ta,
                                else_bb: eb,
                                else_args: ea,
                                ..
                            } => {
                                if tb == bi_id {
                                    targets.push((true, ta));
                                }
                                if eb == bi_id {
                                    targets.push((false, ea));
                                }
                            }
                            _ => {}
                        }
                        for (is_then, old_r) in targets {
                            let mut new_args: Vec<Val> = ir.pool_slice(old_r).to_vec();
                            let mut ob_insts: Vec<Inst> = Vec::new();
                            for &(k0, k1, _, _, _) in &packed_specs {
                                let i0 = new_args[k0];
                                let i1 = new_args[k1];
                                let init_v = if let (Some(b0), Some(b1)) = (
                                    const_f64.get(i0.0 as usize).copied().flatten(),
                                    const_f64.get(i1.0 as usize).copied().flatten(),
                                ) {
                                    let mut bytes = [0u8; 16];
                                    bytes[0..8].copy_from_slice(&b0.to_le_bytes());
                                    bytes[8..16].copy_from_slice(&b1.to_le_bytes());
                                    let cidx = intern_v128_const(ir, bytes);
                                    let v = ir.push_val(Ty::V128);
                                    def_block.push(ob as u32);
                                    const_f64.push(None);
                                    ob_insts.push(Inst {
                                        res: Some(v),
                                        op: Op::ConstV128(cidx),
                                    });
                                    v
                                } else if i0 == i1 {
                                    let v = ir.push_val(Ty::V128);
                                    def_block.push(ob as u32);
                                    const_f64.push(None);
                                    ob_insts.push(Inst {
                                        res: Some(v),
                                        op: Op::V128Splat {
                                            kind: SimdSplatKind::F64x2,
                                            val: i0,
                                        },
                                    });
                                    v
                                } else {
                                    let zidx = intern_v128_const(ir, [0u8; 16]);
                                    let z = ir.push_val(Ty::V128);
                                    let t = ir.push_val(Ty::V128);
                                    let v = ir.push_val(Ty::V128);
                                    def_block.extend_from_slice(&[ob as u32; 3]);
                                    const_f64.extend_from_slice(&[None; 3]);
                                    ob_insts.push(Inst {
                                        res: Some(z),
                                        op: Op::ConstV128(zidx),
                                    });
                                    ob_insts.push(Inst {
                                        res: Some(t),
                                        op: Op::V128ReplaceLane {
                                            kind: SimdLaneKind::F64x2,
                                            v: z,
                                            x: i0,
                                            lane: 0,
                                        },
                                    });
                                    ob_insts.push(Inst {
                                        res: Some(v),
                                        op: Op::V128ReplaceLane {
                                            kind: SimdLaneKind::F64x2,
                                            v: t,
                                            x: i1,
                                            lane: 1,
                                        },
                                    });
                                    v
                                };
                                new_args.push(init_v);
                            }
                            if let Ok(nr) = ir.push_pool(&new_args) {
                                edge_updates.push((ob, is_then, nr, ob_insts));
                            } else {
                                edges_ok = false;
                                break;
                            }
                        }
                        if !edges_ok {
                            break;
                        }
                    }

                    let rew_else_range_opt = if exit_insts.is_empty() {
                        Some(ValRange::EMPTY)
                    } else {
                        let rewritten_else: Vec<Val> = else_snap
                            .iter()
                            .map(|&v| exit_subst.get(&v).copied().unwrap_or(v))
                            .collect();
                        ir.push_pool(&rewritten_else).ok()
                    };

                    if edges_ok
                        && let Some(rew_else_range) = rew_else_range_opt
                        && let Ok(new_then_range) = ir.push_pool(&new_then)
                    {
                        for &(_, _, p_vec, next_vec, d_vec) in &packed_specs {
                            ir.blocks[bi].params.push(p_vec);
                            ir.blocks[bi].insts.push(Inst {
                                res: Some(next_vec),
                                op: Op::V128Bin {
                                    op: 0xfdf0,
                                    a: p_vec,
                                    b: d_vec,
                                },
                            });
                        }
                        for (ob, is_then, nr, ob_insts) in edge_updates {
                            ir.blocks[ob].insts.extend(ob_insts);
                            match &mut ir.blocks[ob].term {
                                Term::Jump { args, .. } => *args = nr,
                                Term::BranchIf {
                                    then_args: ta,
                                    else_args: ea,
                                    ..
                                }
                                | Term::CmpBranch {
                                    then_args: ta,
                                    else_args: ea,
                                    ..
                                } => {
                                    if is_then {
                                        *ta = nr;
                                    } else {
                                        *ea = nr;
                                    }
                                }
                                _ => {}
                            }
                        }
                        if exit_insts.is_empty() {
                            ir.blocks[bi].term = Term::CmpBranch {
                                kind,
                                a,
                                b,
                                then_bb,
                                then_args: new_then_range,
                                else_bb,
                                else_args,
                            };
                        } else {
                            let tramp_id = BlockId(ir.blocks.len() as u32);
                            ir.blocks.push(Block {
                                params: Vec::new(),
                                insts: exit_insts,
                                term: Term::Jump {
                                    target: else_bb,
                                    args: rew_else_range,
                                },
                                is_loop_header: false,
                                preds: 1,
                            });
                            ir.blocks[bi].term = Term::CmpBranch {
                                kind,
                                a,
                                b,
                                then_bb,
                                then_args: new_then_range,
                                else_bb: tramp_id,
                                else_args: ValRange::EMPTY,
                            };
                        }
                    }
                }
            }
        }

        // Phase 4: Combine adjacent checked `F64` store pairs into `Store(V128)`.
        {
            let insts = &ir.blocks[bi].insts;
            let mut pos_in_bb: HashMap<Val, usize> = fx_map_cap(insts.len());
            for (idx, inst) in insts.iter().enumerate() {
                if let Some(r) = inst.res {
                    pos_in_bb.insert(r, idx);
                }
            }
            let mut store_pairs: Vec<(usize, usize, Val, Val, u64)> = Vec::new();
            let mut used = vec![false; insts.len()];
            let mut i = 0usize;
            while i < insts.len() {
                if !used[i]
                    && let Op::Store {
                        kind: StoreKind::F64,
                        addr: a0,
                        val: v0,
                        offset: off0,
                        checked: true,
                    } = insts[i].op
                {
                    // Find the next linear-memory/side-effect op; pair if it's an adjacent F64 store.
                    let mut j = i + 1;
                    while j < insts.len() {
                        if let Op::Store {
                            kind: StoreKind::F64,
                            addr: a1,
                            val: v1,
                            offset: off1,
                            checked: true,
                        } = insts[j].op
                        {
                            if !used[j] && a0 == a1 {
                                let pair = if off0.checked_add(8) == Some(off1)
                                    && off0.checked_add(16).is_some()
                                {
                                    pair_of.get(&(v0, v1)).copied().map(|vv| (vv, off0))
                                } else if off1.checked_add(8) == Some(off0)
                                    && off1.checked_add(16).is_some()
                                {
                                    pair_of.get(&(v1, v0)).copied().map(|vv| (vv, off1))
                                } else {
                                    None
                                };
                                if let Some((vec_v, off_lo)) = pair
                                    && pos_in_bb.get(&vec_v).copied().is_none_or(|p| p < j)
                                {
                                    used[i] = true;
                                    used[j] = true;
                                    store_pairs.push((i, j, a0, vec_v, off_lo));
                                }
                            }
                            break;
                        }
                        if op_touches_linear_mem(&insts[j].op) || insts[j].op.has_side_effect() {
                            break;
                        }
                        j += 1;
                    }
                }
                i += 1;
            }
            if !store_pairs.is_empty() {
                let mut del_s: HashSet<usize> = fx_set();
                let mut rep_s: HashMap<usize, Inst> = fx_map_cap(store_pairs.len());
                for (s0, s1, addr, vec_v, off_lo) in store_pairs {
                    del_s.insert(s0);
                    rep_s.insert(
                        s1,
                        Inst {
                            res: None,
                            op: Op::Store {
                                kind: StoreKind::V128,
                                addr,
                                val: vec_v,
                                offset: off_lo,
                                checked: true,
                            },
                        },
                    );
                }
                let old = std::mem::take(&mut ir.blocks[bi].insts);
                let mut rebuilt = Vec::with_capacity(old.len());
                for (idx, inst) in old.into_iter().enumerate() {
                    if del_s.contains(&idx) {
                        continue;
                    }
                    if let Some(rep) = rep_s.remove(&idx) {
                        rebuilt.push(rep);
                    } else {
                        rebuilt.push(inst);
                    }
                }
                ir.blocks[bi].insts = rebuilt;
            }
        }
    }

    changed_any
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
        | Op::V128LoadLane { checked: false, .. }
        | Op::V128StoreLane { checked: false, .. }
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
#[derive(Clone, PartialEq, Eq)]
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
    try_run_stack_and_trap_elision_from_lookup(inputs.len(), |i| &inputs[i], module, budget)
}

/// Zero-clone Step 8a + Step 8b stack & trap elision from indexed `FnElideInput` lookup.
pub fn try_run_stack_and_trap_elision_from_lookup<'a, F>(
    n_defs: usize,
    get: F,
    module: &X64Module,
    budget: Option<&CompileBudget>,
) -> Res<Option<ElideOutput>>
where
    F: Fn(usize) -> &'a FnElideInput,
{
    if let Some(bg) = budget {
        bg.check_deadline()?;
    }
    let n_imports = module.imports.len();
    let n_total = n_imports + n_defs;

    let setup = (|| -> Option<_> {
        let mut edges: Vec<Vec<usize>> = Vec::new();
        edges.try_reserve_exact(n_defs).ok()?;
        for bi in 0..n_defs {
            let inp = get(bi);
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
    for bi in 0..n_defs {
        let inp = get(bi);
        if in_cycle[bi] || inp.calls_import_or_indirect {
            cannot_elide[bi] = true;
        }
    }

    for &bi in &topo_bottom_up {
        if cannot_elide[bi] {
            continue;
        }
        let inp = get(bi);
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
            let reserve = get(bi)
                .direct_def_callees
                .iter()
                .filter(|&&c| elided[c as usize])
                .map(|&c| subtree_bytes[c as usize])
                .max()
                .unwrap_or(0);
            elided_callee_stack_bytes[bi] = reserve;
        }
    }

    for (bi, &need_sc) in needs_stack_check.iter().enumerate() {
        let inp = get(bi);
        let fi = n_imports + bi;
        fn_can_trap[fi] = need_sc || inp.local_may_trap || inp.calls_import_or_indirect;
    }
    let mut changed = true;
    let mut fix_steps = 0usize;
    while changed {
        changed = false;
        for &bi in &topo_bottom_up {
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
            if get(bi)
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
