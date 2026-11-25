//! Lazy untyped-`u64` 3-address register execution tier with superinstructions,
//! direct register-to-register calls, and basic-block-amortized fuel/cancellation checks.
//!
//! Enabled only when `register-executor` is active and diagnostic/reference single-step
//! modes (`execution-reference`, `performance-counters`, `opcode-coverage`) are off.
//! Admits functions lazily per body with no module-wide instruction cap, so large
//! multi-thousand-function modules execute on the register tier without upfront cost.
use super::{float, integer};
use crate::execution::{Frame, Label};
use crate::{
    ErrorKind, Immediate, ImportKind, IndexType, MemoryType, Module, RefValue, Result, Store,
    TableType, V128, ValType, Value, allocation_failed, err, range, trap,
};
use std::ops::Range;
use std::sync::{Arc, atomic::Ordering};

const MAX_FN_INSNS: usize = 262_144;
const MAX_FN_SLOTS: usize = 32_768;
const MAX_BLOCK_OPS: usize = 64;
const BLOCK_META_NONE: u32 = u32::MAX;
const PAGE_BITS: u32 = 16;

#[derive(Clone, Copy, Debug)]
struct StaticLabel {
    start: u32,
    end: u32,
    /// Operand stack height relative to `frame.base` before `params`.
    height: u16,
    params: u16,
    results: u16,
    loop_: bool,
}

#[derive(Clone, Copy, Debug)]
struct BlockMeta {
    source_pc: u32,
    fuel: u16,
    stack_start: u32,
    stack_len: u16,
    labels_start: u32,
    labels_len: u16,
}

#[derive(Clone, Copy, Debug)]
struct MemGenArg {
    memory: u32,
    offset: u64,
}

#[derive(Clone, Copy, Debug)]
struct BulkMemArg {
    dst_mem: u32,
    src_mem: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FastType {
    I32,
    I64,
    F32,
    F64,
    V128,
}

impl FastType {
    #[inline(always)]
    fn from_val(v: ValType) -> Option<Self> {
        Some(match v {
            ValType::I32 => Self::I32,
            ValType::I64 => Self::I64,
            ValType::F32 => Self::F32,
            ValType::F64 => Self::F64,
            ValType::V128 => Self::V128,
            _ => return None,
        })
    }

    #[inline(always)]
    fn slots(self) -> u16 {
        if matches!(self, Self::V128) { 2 } else { 1 }
    }
}

#[derive(Clone, Copy, Debug)]
struct SimdMemArg {
    memory: u32,
    offset: u64,
    src_vec: u16,
    lane: u8,
}

#[derive(Clone, Copy, Debug)]
struct CallSite {
    func_idx: u32,
    arg_base: u16,
    arg_slots: u16,
    logical_arg_start: u16,
    meta_at_call: u32,
    meta_after_call: u32,
}

#[derive(Clone, Copy, Debug)]
struct CallIndirectSite {
    type_idx: u32,
    table_idx: u32,
    index_reg: u16,
    arg_base: u16,
    arg_slots: u16,
    logical_arg_start: u16,
    table_is_64: bool,
    meta_at_call: u32,
    meta_after_call: u32,
}

/// Compact 12-byte 3-address register instruction over raw untyped `u64` slots.
///
/// Representation invariant:
/// - `i32` / `f32`: payload in low 32 bits, zero-extended to `u64`.
/// - `i64` / `f64`: full 64-bit payload.
#[derive(Clone, Copy, Debug)]
enum RegOp {
    /// Basic-block header on forward control boundaries: charges `fuel` when fuel is active.
    Block {
        meta: u32,
        fuel: u16,
    },
    /// Basic-block header on loop headers, function entries, and MAX_BLOCK_OPS splits:
    /// checks cancellation and charges `fuel` (bails at `meta` if short).
    CheckBlock {
        meta: u32,
        fuel: u16,
    },
    /// Register move: `reg[dst] = reg[src]`.
    Mov {
        dst: u16,
        src: u16,
    },
    /// Load 32-bit constant (zero-extended into `u64`).
    Const32 {
        dst: u16,
        val: u32,
    },
    /// Load 64-bit constant from `consts[idx]`.
    Const64 {
        dst: u16,
        idx: u32,
    },

    // ---- Non-trapping 3-address i32 ALU ----
    I32Eqz {
        dst: u16,
        src: u16,
    },
    I32Add {
        dst: u16,
        a: u16,
        b: u16,
    },
    I32Sub {
        dst: u16,
        a: u16,
        b: u16,
    },
    I32Mul {
        dst: u16,
        a: u16,
        b: u16,
    },
    I32DivS {
        dst: u16,
        a: u16,
        b: u16,
        src_off: u8,
    },
    I32DivU {
        dst: u16,
        a: u16,
        b: u16,
        src_off: u8,
    },
    I32RemS {
        dst: u16,
        a: u16,
        b: u16,
        src_off: u8,
    },
    I32RemU {
        dst: u16,
        a: u16,
        b: u16,
        src_off: u8,
    },
    I32And {
        dst: u16,
        a: u16,
        b: u16,
    },
    I32Or {
        dst: u16,
        a: u16,
        b: u16,
    },
    I32Xor {
        dst: u16,
        a: u16,
        b: u16,
    },
    I32Shl {
        dst: u16,
        a: u16,
        b: u16,
    },
    I32ShrS {
        dst: u16,
        a: u16,
        b: u16,
    },
    I32ShrU {
        dst: u16,
        a: u16,
        b: u16,
    },
    I32Rotl {
        dst: u16,
        a: u16,
        b: u16,
    },
    I32Rotr {
        dst: u16,
        a: u16,
        b: u16,
    },
    I32Cmp {
        cmp: u8,
        dst: u16,
        a: u16,
        b: u16,
    },
    I32CmpImm {
        cmp: u8,
        dst: u16,
        a: u16,
        imm: i32,
    },

    // ---- Superinstructions: immediate i32 ALU ----
    I32AddImm {
        dst: u16,
        src: u16,
        imm: i32,
    },
    I32SubImm {
        dst: u16,
        src: u16,
        imm: i32,
    },
    I32MulImm {
        dst: u16,
        src: u16,
        imm: i32,
    },
    /// Non-zero non-power-of-two constant unsigned divide via Lemire 64-bit reciprocal `consts[cidx]`.
    I32DivUConst {
        dst: u16,
        src: u16,
        cidx: u32,
    },
    /// Non-zero divisor constant unsigned remainder (cannot trap).
    I32RemUImm {
        dst: u16,
        src: u16,
        imm: u32,
    },
    I32AndImm {
        dst: u16,
        src: u16,
        imm: u32,
    },
    I32OrImm {
        dst: u16,
        src: u16,
        imm: u32,
    },
    I32XorImm {
        dst: u16,
        src: u16,
        imm: u32,
    },
    I32ShlImm {
        dst: u16,
        src: u16,
        shift: u32,
    },
    I32ShrSImm {
        dst: u16,
        src: u16,
        shift: u32,
    },
    I32ShrUImm {
        dst: u16,
        src: u16,
        shift: u32,
    },
    I32RotlImm {
        dst: u16,
        src: u16,
        shift: u32,
    },
    I32RotrImm {
        dst: u16,
        src: u16,
        shift: u32,
    },

    // ---- Non-trapping 3-address i64 ALU & conversions ----
    I64Add {
        dst: u16,
        a: u16,
        b: u16,
    },
    I64Sub {
        dst: u16,
        a: u16,
        b: u16,
    },
    I64Mul {
        dst: u16,
        a: u16,
        b: u16,
    },
    I64And {
        dst: u16,
        a: u16,
        b: u16,
    },
    I64Or {
        dst: u16,
        a: u16,
        b: u16,
    },
    I64Xor {
        dst: u16,
        a: u16,
        b: u16,
    },
    I64Shl {
        dst: u16,
        a: u16,
        b: u16,
    },
    I64ShrS {
        dst: u16,
        a: u16,
        b: u16,
    },
    I64ShrU {
        dst: u16,
        a: u16,
        b: u16,
    },
    I64AddImm {
        dst: u16,
        src: u16,
        imm: i32,
    },
    I64Cmp {
        cmp: u8,
        dst: u16,
        a: u16,
        b: u16,
    },
    /// `i32.wrap_i64` and `i64.extend_i32_u` (both zero-extend low 32 bits).
    I32WrapI64 {
        dst: u16,
        src: u16,
    },
    I64ExtendI32S {
        dst: u16,
        src: u16,
    },

    // ---- Non-trapping 3-address f32/f64 ALU ----
    F32Add {
        dst: u16,
        a: u16,
        b: u16,
    },
    F32Sub {
        dst: u16,
        a: u16,
        b: u16,
    },
    F32Mul {
        dst: u16,
        a: u16,
        b: u16,
    },
    F32Div {
        dst: u16,
        a: u16,
        b: u16,
    },
    F64Add {
        dst: u16,
        a: u16,
        b: u16,
    },
    F64Sub {
        dst: u16,
        a: u16,
        b: u16,
    },
    F64Mul {
        dst: u16,
        a: u16,
        b: u16,
    },
    F64Div {
        dst: u16,
        a: u16,
        b: u16,
    },
    F64Min {
        dst: u16,
        a: u16,
        b: u16,
    },
    F64Max {
        dst: u16,
        a: u16,
        b: u16,
    },
    F64Cmp {
        cmp: u8,
        dst: u16,
        a: u16,
        b: u16,
    },
    F64AddConst {
        dst: u16,
        src: u16,
        cidx: u32,
    },
    F64SubConst {
        dst: u16,
        src: u16,
        cidx: u32,
    },
    F64MulConst {
        dst: u16,
        src: u16,
        cidx: u32,
    },
    F64MinConst {
        dst: u16,
        src: u16,
        cidx: u32,
    },
    F64MaxConst {
        dst: u16,
        src: u16,
        cidx: u32,
    },

    // ---- Generic scalar unary/binary (including trapping div/rem/trunc) ----
    Unary {
        op: u16,
        dst: u16,
        src: u16,
        src_off: u8,
    },
    Binary {
        op: u16,
        dst: u16,
        a: u16,
        b: u16,
        src_off: u8,
    },

    Select {
        dst: u16,
        a: u16,
        b: u16,
        cond: u16,
    },

    // ---- Scalar globals ----
    GlobalGet {
        dst: u16,
        global: u32,
    },
    GlobalSet {
        src: u16,
        global: u32,
    },

    // ---- Fast memory-0 loads & stores (memory32 & memory64) ----
    /// 4-byte load (`i32.load` and `f32.load`).
    LoadI32 {
        dst: u16,
        addr: u16,
        offset: u32,
        src_off: u8,
    },
    /// 8-byte load (`i64.load` and `f64.load`).
    LoadI64 {
        dst: u16,
        addr: u16,
        offset: u32,
        src_off: u8,
    },
    /// Unsigned 8-bit load (`i32.load8_u`).
    LoadI32U8 {
        dst: u16,
        addr: u16,
        offset: u32,
        src_off: u8,
    },
    /// Unsigned 16-bit load (`i32.load16_u`).
    LoadI32U16 {
        dst: u16,
        addr: u16,
        offset: u32,
        src_off: u8,
    },
    /// Fused `acc = acc.wrapping_add(mem0[addr + offset] as u8)`.
    LoadU8Add {
        acc: u16,
        addr: u16,
        offset: u32,
        src_off: u8,
    },
    /// Fused `acc = f64(acc) + f64(mem0[addr + offset])`.
    LoadF64Add {
        acc: u16,
        addr: u16,
        offset: u32,
        src_off: u8,
    },
    /// Fused `dst = mem0[(a + b) + offset]` (i32/f32, memory32).
    LoadI32AddAddr {
        dst: u16,
        a: u16,
        b: u16,
        offset: u32,
        src_off: u8,
    },
    /// Fused `dst = mem0[(a + b) + offset] as u8` (memory32).
    LoadU8AddAddr {
        dst: u16,
        a: u16,
        b: u16,
        offset: u32,
        src_off: u8,
    },
    /// Fused `dst = mem0[(addr as u32).wrapping_add(imm)]` (i32/f32, offset=0, memory32).
    LoadI32AddImm {
        dst: u16,
        addr: u16,
        imm: u32,
        src_off: u8,
    },
    /// Fused `dst = mem0[(addr as u32).wrapping_add(imm)] as u8` (offset=0, memory32).
    LoadU8AddImm {
        dst: u16,
        addr: u16,
        imm: u32,
        src_off: u8,
    },
    /// Fused `acc += mem0[(addr as u32).wrapping_add(imm)] as u8` (offset=0, memory32).
    LoadU8AddImmAcc {
        acc: u16,
        addr: u16,
        imm: u32,
        src_off: u8,
    },
    /// 4-byte store (`i32.store` and `f32.store`).
    StoreI32 {
        addr: u16,
        val: u16,
        offset: u32,
        src_off: u8,
    },
    /// 8-byte store (`i64.store` and `f64.store`).
    StoreI64 {
        addr: u16,
        val: u16,
        offset: u32,
        src_off: u8,
    },
    /// 1-byte store (`i32.store8`).
    StoreI32_8 {
        addr: u16,
        val: u16,
        offset: u32,
        src_off: u8,
    },
    /// 2-byte store (`i32.store16`).
    StoreI32_16 {
        addr: u16,
        val: u16,
        offset: u32,
        src_off: u8,
    },
    /// Fused `mem0[(addr as u32).wrapping_add(imm)] = val as u32` (offset=0, memory32).
    StoreI32AddImm {
        addr: u16,
        val: u16,
        imm: u32,
        src_off: u8,
    },
    /// Fused `mem0[(addr as u32).wrapping_add(imm)] = val as u8` (offset=0, memory32).
    StoreI32_8AddImm {
        addr: u16,
        val: u16,
        imm: u32,
        src_off: u8,
    },
    /// General scalar load/store (any opcode `0x28..=0x3e`, any memory, full `u64` offset).
    MemGen {
        op: u16,
        addr: u16,
        val: u16,
        arg_idx: u32,
        src_off: u8,
    },
    MemSize {
        dst: u16,
        mem: u32,
    },
    MemGrow {
        dst: u16,
        delta: u16,
        mem: u32,
        src_off: u8,
    },
    /// Fast `memory.copy 0 0` on unshared memory 0.
    MemCopy0 {
        dst: u16,
        src: u16,
        len: u16,
        src_off: u8,
    },
    /// Fast `memory.fill 0` on unshared memory 0.
    MemFill0 {
        dst: u16,
        val: u16,
        len: u16,
        src_off: u8,
    },
    /// General bulk memory/table op:
    /// `0=memory.copy`, `1=memory.fill`, `2=memory.init`, `3=data.drop`,
    /// `4=table.init`, `5=elem.drop`, `6=table.copy`.
    MemBulkGen {
        kind: u8,
        src_off: u8,
        dst: u16,
        src_or_val: u16,
        len: u16,
        arg_idx: u16,
    },
    /// `table.size table` -> `reg[dst] = len`.
    TableSize {
        dst: u16,
        table: u32,
    },

    // ---- Superinstructions: fused compare-and-branch & counted loop step ----
    /// `if cmp(reg[a], reg[b]) { ip = target }`.
    I32CmpBrIf {
        cmp: u8,
        a: u16,
        b: u16,
        target: u32,
    },
    /// `if cmp(reg[a], imm) { ip = target }`.
    I32CmpImmBrIf {
        cmp: u8,
        a: u16,
        imm: i32,
        target: u32,
    },
    /// `if cmp_i64(reg[a], reg[b]) { ip = target }`.
    I64CmpBrIf {
        cmp: u8,
        a: u16,
        b: u16,
        target: u32,
    },
    /// `reg[r] += step; if (reg[r] as u32) < (reg[limit] as u32) { ip = target }`.
    I32IncLtUBrIf {
        reg: u16,
        limit: u16,
        step: i16,
        target: u32,
    },
    /// `reg[r] += step; if (reg[r] as i32) < (reg[limit] as i32) { ip = target }`.
    I32IncLtSBrIf {
        reg: u16,
        limit: u16,
        step: i16,
        target: u32,
    },
    /// `reg[reg] = (reg[reg] as u32).wrapping_add(imm as u32) as u64; ip = target`.
    I32AddImmJump {
        reg: u16,
        imm: i32,
        target: u32,
    },

    // ---- Control flow ----
    Jump {
        target: u32,
    },
    BrIf {
        cond: u16,
        target: u32,
    },
    BrIfZero {
        cond: u16,
        target: u32,
    },
    BrTable {
        selector: u16,
        table_idx: u32,
    },
    MoveRange {
        dst_base: u16,
        src_base: u16,
        count: u16,
    },
    Call {
        call_idx: u32,
        src_off: u8,
    },
    CallIndirect {
        call_idx: u32,
        src_off: u8,
    },
    ReturnCall {
        call_idx: u32,
        src_off: u8,
    },
    ReturnCallIndirect {
        call_idx: u32,
        src_off: u8,
    },
    /// Wide 128-bit integer arithmetic (`0=add128, 1=sub128, 2=mul_wide_s, 3=mul_wide_u`)
    /// operating in-place on contiguous slots `base..base+4` (or `+2`) and writing `(lo, hi)` to `base..base+2`.
    WideArith {
        kind: u8,
        base: u16,
    },
    // ---- Native SIMD & Relaxed SIMD (2 consecutive `u64` slots per `v128`) ----
    MovV128 {
        dst: u16,
        src: u16,
    },
    ConstV128 {
        dst: u16,
        idx: u32,
    },
    SelectV128 {
        dst: u16,
        a: u16,
        b: u16,
        cond: u16,
    },
    GlobalGetV128 {
        dst: u16,
        global: u32,
    },
    GlobalSetV128 {
        src: u16,
        global: u32,
    },
    LoadV128 {
        dst: u16,
        addr: u16,
        offset: u32,
        src_off: u8,
    },
    StoreV128 {
        addr: u16,
        val: u16,
        offset: u32,
        src_off: u8,
    },
    SimdMemGen {
        op: u16,
        addr: u16,
        val: u16,
        arg_idx: u32,
        src_off: u8,
    },
    SimdShuffle {
        dst: u16,
        a: u16,
        b: u16,
        mask_idx: u16,
    },
    SimdUnary {
        op: u16,
        dst: u16,
        src: u16,
        lane: u8,
    },
    SimdBinary {
        op: u16,
        dst: u16,
        a: u16,
        b: u16,
        lane: u8,
    },
    SimdTernary {
        op: u16,
        dst: u16,
        a: u16,
        b: u16,
        c: u16,
    },
    Return {
        base: u16,
        count: u16,
    },
    TrapUnreachable {
        src_off: u8,
    },
    /// Clean bail to the decoded interpreter at block boundary `meta`.
    Bail {
        meta: u32,
    },
}

/// Lazily compiled untyped-`u64` register function owned by a `Module`.
#[derive(Debug)]
pub(crate) struct FastFunction {
    ops: Box<[RegOp]>,
    consts: Box<[u64]>,
    v128_consts: Box<[[u64; 2]]>,
    simd_masks: Box<[[u8; 16]]>,
    mem_args: Box<[MemGenArg]>,
    simd_mem_args: Box<[SimdMemArg]>,
    bulk_args: Box<[BulkMemArg]>,
    call_sites: Box<[CallSite]>,
    call_indirect_sites: Box<[CallIndirectSite]>,
    table_ranges: Box<[Range<u32>]>,
    table_targets: Box<[u32]>,
    blocks: Box<[BlockMeta]>,
    stack_types: Box<[FastType]>,
    static_labels: Box<[StaticLabel]>,
    local_types: Box<[FastType]>,
    result_types: Box<[FastType]>,
    /// Logical Wasm local count (`params + locals`) for `active_local_slots` resource accounting.
    num_locals: u16,
    /// Physical `u64` register slots occupied by locals (`sum(ty.slots())`).
    local_reg_slots: u16,
    slots: u16,
    stack_peak: usize,
    max_nesting: usize,
    mem0_is_64: bool,
    fuel_blocks: bool,
}

/// Outcome of running a function on the fast register tier.
pub(crate) enum FastOutcome {
    /// Entry declined before running any instruction (e.g. cold threshold, unsupported body, or tight stack/fuel limit).
    Declined,
    /// Top-level entry frame completed and returned its results onto `stack`.
    Returned,
    /// Execution bailed cleanly at a source PC; `frames` and `stack` are materialized at `frame.pc` (NOT precharged).
    Bailed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VVal {
    Slot(u16),
    Local(u16),
    Imm32(u32),
    Imm64(u64),
    ImmV128(u32),
}

#[derive(Clone, Copy)]
struct VSlot {
    src: VVal,
    slot_off: u16,
    ty: FastType,
}

struct CompileLabel {
    start: usize,
    end: usize,
    other: Option<usize>,
    loop_: bool,
    base_height: usize,
    base_slot: u16,
    prefix: Vec<FastType>,
    params: Vec<FastType>,
    results: Vec<FastType>,
    param_slots: u16,
    result_slots: u16,
    parent_dead: bool,
}

fn declared_memory(module: &Module, index: u32) -> Option<MemoryType> {
    let decoded = module.data.inner.decoded();
    let mut rem = index as usize;
    for imp in &decoded.imports {
        if let ImportKind::Memory(ty) = imp.kind {
            if rem == 0 {
                return Some(ty);
            }
            rem -= 1;
        }
    }
    decoded.memories.get(rem).copied()
}

fn declared_table(module: &Module, index: u32) -> Option<TableType> {
    let decoded = module.data.inner.decoded();
    let mut rem = index as usize;
    for imp in &decoded.imports {
        if let ImportKind::Table(ty) = imp.kind {
            if rem == 0 {
                return Some(ty);
            }
            rem -= 1;
        }
    }
    decoded.tables.get(rem).copied()
}

fn declared_global_type(module: &Module, index: u32) -> Option<ValType> {
    let decoded = module.data.inner.decoded();
    let mut rem = index as usize;
    for imp in &decoded.imports {
        if let ImportKind::Global(gt) = imp.kind {
            if rem == 0 {
                return Some(gt.value);
            }
            rem -= 1;
        }
    }
    decoded.globals.get(rem).map(|g| g.ty.value)
}

fn declared_func_type(module: &Module, index: u32) -> Option<&crate::FuncType> {
    let decoded = module.data.inner.decoded();
    let mut rem = index as usize;
    for imp in &decoded.imports {
        if let ImportKind::Func(ti) = imp.kind {
            if rem == 0 {
                return decoded.types.get(ti.0 as usize);
            }
            rem -= 1;
        }
    }
    let ti = decoded.functions.get(rem)?.0 as usize;
    decoded.types.get(ti)
}

#[inline(always)]
fn fast_from_numeric(t: super::ir::NumericType) -> FastType {
    match t {
        super::ir::NumericType::I32 => FastType::I32,
        super::ir::NumericType::I64 => FastType::I64,
        super::ir::NumericType::F32 => FastType::F32,
        super::ir::NumericType::F64 => FastType::F64,
    }
}

fn block_sig(module: &Module, imm: &Immediate) -> Option<(Vec<FastType>, Vec<FastType>)> {
    match imm {
        Immediate::Block(-64) => Some((Vec::new(), Vec::new())),
        Immediate::BlockType(ty) => Some((Vec::new(), vec![FastType::from_val(*ty)?])),
        Immediate::Block(v) if *v < 0 => {
            let ty = match *v {
                -1 => FastType::I32,
                -2 => FastType::I64,
                -3 => FastType::F32,
                -4 => FastType::F64,
                -5 => FastType::V128,
                _ => return None,
            };
            Some((Vec::new(), vec![ty]))
        }
        Immediate::Block(idx) => {
            let ft = module.data.inner.decoded().types.get(*idx as usize)?;
            let mut params = Vec::with_capacity(ft.params.len());
            for &t in &ft.params {
                params.push(FastType::from_val(t)?);
            }
            let mut results = Vec::with_capacity(ft.results.len());
            for &t in &ft.results {
                results.push(FastType::from_val(t)?);
            }
            Some((params, results))
        }
        _ => None,
    }
}

fn mem_sig(op: u16) -> Option<(FastType, usize, bool)> {
    let ty = match op {
        0x28 | 0x2c..=0x2f | 0x36 | 0x3a..=0x3b => FastType::I32,
        0x29 | 0x30..=0x35 | 0x37 | 0x3c..=0x3e => FastType::I64,
        0x2a | 0x38 => FastType::F32,
        0x2b | 0x39 => FastType::F64,
        _ => return None,
    };
    let width = match op {
        0x28 | 0x2a | 0x34..=0x36 | 0x38 | 0x3e => 4,
        0x29 | 0x2b | 0x37 | 0x39 => 8,
        0x2c | 0x2d | 0x30 | 0x31 | 0x3a | 0x3c => 1,
        _ => 2,
    };
    Some((ty, width, op >= 0x36))
}

#[derive(Clone, Copy, Debug)]
enum SimdShape {
    Mem,
    Const,
    Shuffle,
    Unary { out_ty: FastType, has_lane: bool },
    Binary { has_lane: bool },
    Ternary,
}

fn simd_op_shape(op: u16) -> Option<SimdShape> {
    Some(match op {
        0xfd00..=0xfd0b | 0xfd54..=0xfd5d => SimdShape::Mem,
        0xfd0c => SimdShape::Const,
        0xfd0d => SimdShape::Shuffle,
        0xfd0f..=0xfd14 => SimdShape::Unary {
            out_ty: FastType::V128,
            has_lane: false,
        },
        0xfd15 | 0xfd16 | 0xfd18 | 0xfd19 | 0xfd1b => SimdShape::Unary {
            out_ty: FastType::I32,
            has_lane: true,
        },
        0xfd1d => SimdShape::Unary {
            out_ty: FastType::I64,
            has_lane: true,
        },
        0xfd1f => SimdShape::Unary {
            out_ty: FastType::F32,
            has_lane: true,
        },
        0xfd21 => SimdShape::Unary {
            out_ty: FastType::F64,
            has_lane: true,
        },
        0xfd17 | 0xfd1a | 0xfd1c | 0xfd1e | 0xfd20 | 0xfd22 => SimdShape::Binary { has_lane: true },
        0xfd0e
        | 0xfd23..=0xfd4c
        | 0xfd4e..=0xfd51
        | 0xfd65
        | 0xfd66
        | 0xfd6b..=0xfd73
        | 0xfd76..=0xfd79
        | 0xfd7b
        | 0xfd82
        | 0xfd85
        | 0xfd86
        | 0xfd8b..=0xfd93
        | 0xfd95..=0xfd99
        | 0xfd9b..=0xfd9f
        | 0xfdab..=0xfdae
        | 0xfdb1
        | 0xfdb5..=0xfdba
        | 0xfdbc..=0xfdbf
        | 0xfdcb..=0xfdce
        | 0xfdd1
        | 0xfdd5..=0xfddf
        | 0xfde4..=0xfdeb
        | 0xfdf0..=0xfdf7
        | 0xfa00
        | 0xfa0d..=0xfa12 => SimdShape::Binary { has_lane: false },
        0xfd4d
        | 0xfd5e..=0xfd62
        | 0xfd67..=0xfd6a
        | 0xfd74
        | 0xfd75
        | 0xfd7a
        | 0xfd7c..=0xfd81
        | 0xfd87..=0xfd8a
        | 0xfd94
        | 0xfda0
        | 0xfda1
        | 0xfda7..=0xfdaa
        | 0xfdc0
        | 0xfdc1
        | 0xfdc7..=0xfdca
        | 0xfde0
        | 0xfde1
        | 0xfde3
        | 0xfdec
        | 0xfded
        | 0xfdef
        | 0xfdf8..=0xfdff
        | 0xfa01..=0xfa04 => SimdShape::Unary {
            out_ty: FastType::V128,
            has_lane: false,
        },
        0xfd53 | 0xfd63 | 0xfd64 | 0xfd83 | 0xfd84 | 0xfda3 | 0xfda4 | 0xfdc3 | 0xfdc4 => {
            SimdShape::Unary {
                out_ty: FastType::I32,
                has_lane: false,
            }
        }
        0xfd52 | 0xfa05..=0xfa0c | 0xfa13 => SimdShape::Ternary,
        _ => return None,
    })
}

fn is_supported_op(module: &Module, insn: &crate::Instruction) -> bool {
    match insn.opcode {
        0x00..=0x05
        | 0x0b..=0x0f
        | 0x1a
        | 0x1b
        | 0x20..=0x22
        | 0x41..=0xc4
        | 0xfc00..=0xfc07
        | 0xfc13..=0xfc16 => true,
        0x1c => {
            matches!(&insn.immediate, Immediate::Types(ts) if ts.len() == 1 && FastType::from_val(ts[0]).is_some())
        }
        0x10 | 0x12 => {
            let Immediate::Index(fi) = insn.immediate else {
                return false;
            };
            declared_func_type(module, fi).is_some_and(|ft| {
                ft.params
                    .iter()
                    .chain(&ft.results)
                    .all(|&t| FastType::from_val(t).is_some())
            })
        }
        0x11 | 0x13 => {
            let Immediate::Pair(type_idx, table_idx) = insn.immediate else {
                return false;
            };
            declared_table(module, table_idx).is_some()
                && module
                    .data
                    .inner
                    .decoded()
                    .types
                    .get(type_idx as usize)
                    .is_some_and(|ft| {
                        ft.params
                            .iter()
                            .chain(&ft.results)
                            .all(|&t| FastType::from_val(t).is_some())
                    })
        }
        0x23 | 0x24 => {
            let Immediate::Index(gi) = insn.immediate else {
                return false;
            };
            declared_global_type(module, gi).is_some_and(|t| FastType::from_val(t).is_some())
        }
        0x28..=0x3e => {
            let Immediate::MemArg { memory, .. } = insn.immediate else {
                return false;
            };
            declared_memory(module, memory).is_some_and(|m| !m.shared)
        }
        0x3f | 0x40 => {
            let Immediate::Index(mem) = insn.immediate else {
                return false;
            };
            declared_memory(module, mem).is_some_and(|m| !m.shared)
        }
        0xfc08 => {
            let Immediate::Pair(seg, mem) = insn.immediate else {
                return false;
            };
            (seg as usize) < module.data.inner.decoded().data.len()
                && declared_memory(module, mem).is_some()
        }
        0xfc09 => {
            let Immediate::Index(seg) = insn.immediate else {
                return false;
            };
            (seg as usize) < module.data.inner.decoded().data.len()
        }
        0xfc0a => {
            let Immediate::Pair(dst_mem, src_mem) = insn.immediate else {
                return false;
            };
            declared_memory(module, dst_mem).is_some() && declared_memory(module, src_mem).is_some()
        }
        0xfc0b => {
            let Immediate::Index(mem) = insn.immediate else {
                return false;
            };
            declared_memory(module, mem).is_some()
        }
        0xfc0c => {
            let Immediate::Pair(elem, tbl) = insn.immediate else {
                return false;
            };
            (elem as usize) < module.data.inner.decoded().elements.len()
                && declared_table(module, tbl).is_some()
        }
        0xfc0d => {
            let Immediate::Index(elem) = insn.immediate else {
                return false;
            };
            (elem as usize) < module.data.inner.decoded().elements.len()
        }
        0xfc0e => {
            let Immediate::Pair(dst_tbl, src_tbl) = insn.immediate else {
                return false;
            };
            declared_table(module, dst_tbl).is_some() && declared_table(module, src_tbl).is_some()
        }
        0xfc10 => {
            let Immediate::Index(tbl) = insn.immediate else {
                return false;
            };
            declared_table(module, tbl).is_some()
        }
        0xfd00..=0xfd0b | 0xfd54..=0xfd5d => match insn.immediate {
            Immediate::MemArg { memory, .. } | Immediate::MemLane { memory, .. } => {
                declared_memory(module, memory).is_some()
            }
            _ => false,
        },
        0xfd0c..=0xfd53 | 0xfd5e..=0xfdff | 0xfa00..=0xfa13 => simd_op_shape(insn.opcode).is_some(),
        _ => false,
    }
}

fn invert_i32_cmp(cmp: u8) -> u8 {
    match cmp {
        0 => 1, // eq <-> ne
        1 => 0,
        2 => 8, // lt_s <-> ge_s
        8 => 2,
        3 => 9, // lt_u <-> ge_u
        9 => 3,
        4 => 6, // gt_s <-> le_s
        6 => 4,
        5 => 7, // gt_u <-> le_u
        7 => 5,
        _ => unreachable!(),
    }
}

#[inline(always)]
fn eval_i32_cmp(cmp: u8, a: u32, b: u32) -> bool {
    match cmp {
        0 => a == b,
        1 => a != b,
        2 => (a as i32) < (b as i32),
        3 => a < b,
        4 => (a as i32) > (b as i32),
        5 => a > b,
        6 => (a as i32) <= (b as i32),
        7 => a <= b,
        8 => (a as i32) >= (b as i32),
        _ => a >= b,
    }
}

#[inline(always)]
fn eval_i64_cmp(cmp: u8, a: u64, b: u64) -> bool {
    match cmp {
        0 => a == b,
        1 => a != b,
        2 => (a as i64) < (b as i64),
        3 => a < b,
        4 => (a as i64) > (b as i64),
        5 => a > b,
        6 => (a as i64) <= (b as i64),
        7 => a <= b,
        8 => (a as i64) >= (b as i64),
        _ => a >= b,
    }
}

#[inline(always)]
fn eval_f64_cmp(cmp: u8, a: f64, b: f64) -> bool {
    match cmp {
        0 => a == b,
        1 => a != b,
        2 => a < b,
        3 => a > b,
        4 => a <= b,
        _ => a >= b,
    }
}

#[inline(always)]
fn wasm_f64_min(fa: f64, fb: f64) -> u64 {
    let r = if fa.is_nan() || fb.is_nan() {
        f64::NAN
    } else if fa == fb {
        f64::from_bits(fa.to_bits() | fb.to_bits())
    } else {
        fa.min(fb)
    };
    r.to_bits()
}

#[inline(always)]
fn wasm_f64_max(fa: f64, fb: f64) -> u64 {
    let r = if fa.is_nan() || fb.is_nan() {
        f64::NAN
    } else if fa == fb {
        f64::from_bits(fa.to_bits() & fb.to_bits())
    } else {
        fa.max(fb)
    };
    r.to_bits()
}

#[inline]
fn push_const(consts: &mut Vec<u64>, val: u64) -> u32 {
    if consts.len() <= 32
        && let Some(idx) = consts.iter().position(|&c| c == val)
    {
        return idx as u32;
    }
    let idx = consts.len() as u32;
    consts.push(val);
    idx
}

fn fold_i32_bin(op: u16, a: u32, b: u32) -> Option<u32> {
    Some(match op {
        0x46 => u32::from(a == b),
        0x47 => u32::from(a != b),
        0x48 => u32::from((a as i32) < (b as i32)),
        0x49 => u32::from(a < b),
        0x4a => u32::from((a as i32) > (b as i32)),
        0x4b => u32::from(a > b),
        0x4c => u32::from((a as i32) <= (b as i32)),
        0x4d => u32::from(a <= b),
        0x4e => u32::from((a as i32) >= (b as i32)),
        0x4f => u32::from(a >= b),
        0x6a => a.wrapping_add(b),
        0x6b => a.wrapping_sub(b),
        0x6c => a.wrapping_mul(b),
        0x71 => a & b,
        0x72 => a | b,
        0x73 => a ^ b,
        0x74 => a.wrapping_shl(b),
        0x75 => ((a as i32).wrapping_shr(b)) as u32,
        0x76 => a.wrapping_shr(b),
        0x77 => a.rotate_left(b),
        0x78 => a.rotate_right(b),
        _ => return None,
    })
}

/// Lazily compiles `body` of `module` to an untyped-`u64` 3-address `FastFunction`.
/// Supports scalar (`i32`, `i64`, `f32`, `f64`) and native SIMD (`v128`, 2 `u64` slots per value).
/// Returns `None` if the function uses reference locals/params/results or exceeds per-body limits.
pub(crate) fn compile_fast(module: &Module, body: usize) -> Option<Arc<FastFunction>> {
    let decoded = module.data.inner.decoded();
    let code = decoded.code.get(body)?;
    let n = code.instructions.len();
    if n == 0 || n > MAX_FN_INSNS {
        return None;
    }
    let type_index = decoded.functions.get(body)?.0 as usize;
    let ft = decoded.types.get(type_index)?;
    let bounds = module.data.inner.function_bounds().get(body)?;
    let num_params = ft.params.len();
    let num_locals = num_params.checked_add(code.locals.len())?;

    let mut local_types = Vec::with_capacity(num_locals);
    let mut local_slots = Vec::with_capacity(num_locals);
    let mut local_reg_slots_usize = 0usize;
    for &t in ft.params.iter().chain(&code.locals) {
        let fty = FastType::from_val(t)?;
        local_slots.push(u16::try_from(local_reg_slots_usize).ok()?);
        local_reg_slots_usize = local_reg_slots_usize.checked_add(fty.slots() as usize)?;
        local_types.push(fty);
    }
    if local_reg_slots_usize.checked_add(bounds.stack_peak)? > MAX_FN_SLOTS {
        return None;
    }
    let mut result_types = Vec::with_capacity(ft.results.len());
    let mut result_reg_slots_usize = 0usize;
    for &t in &ft.results {
        let fty = FastType::from_val(t)?;
        result_reg_slots_usize = result_reg_slots_usize.checked_add(fty.slots() as usize)?;
        result_types.push(fty);
    }
    let result_reg_slots = u16::try_from(result_reg_slots_usize).ok()?;
    let local_reg_slots = u16::try_from(local_reg_slots_usize).ok()?;

    // Check block signatures and compute basic-block starts.
    let fuel_blocks = module.data.register_fast_fuel;
    let mut is_block_start = vec![false; n + 1];
    let mut is_check_block = vec![false; n + 1];
    let mut has_loops_or_calls = false;
    is_block_start[0] = true;
    is_check_block[0] = true;
    for (pc, insn) in code.instructions.iter().enumerate() {
        match insn.opcode {
            0x02..=0x04 => {
                // Unsupported block signatures decline the whole function cleanly.
                block_sig(module, &insn.immediate)?;
                let (end, other) = module.data.control.get(body, pc)?;
                if end >= n {
                    return None;
                }
                if insn.opcode == 0x03 {
                    // Loop header target is pc + 1.
                    is_block_start[pc + 1] = true;
                    is_check_block[pc + 1] = true;
                    has_loops_or_calls = true;
                } else if insn.opcode == 0x04 {
                    is_block_start[pc + 1] = true;
                    if let Some(el) = other {
                        is_block_start[el + 1] = true;
                    }
                    is_block_start[end] = true;
                }
                is_block_start[end + 1] = true;
            }
            0x05 => {
                is_block_start[pc + 1] = true;
            }
            0x00 | 0x0c..=0x0f => {
                is_block_start[pc + 1] = true;
            }
            0x10..=0x13 => {
                is_block_start[pc + 1] = true;
                has_loops_or_calls = true;
            }
            _ if !is_supported_op(module, insn) => {
                is_block_start[pc] = true;
                is_block_start[pc + 1] = true;
            }
            _ => {}
        }
    }
    if !fuel_blocks && !has_loops_or_calls && n <= MAX_BLOCK_OPS {
        // Pure bounded leaf function (no loops, no calls, <= 64 ops):
        // cancellation was already checked at caller loop header or invocation entry.
        is_check_block[0] = false;
    }

    let mem0_is_64 = declared_memory(module, 0).is_some_and(|m| m.index_type == IndexType::I64);
    let mem0_unshared = declared_memory(module, 0).is_some_and(|m| !m.shared);

    let mut ops: Vec<RegOp> = Vec::with_capacity(n);
    let mut consts: Vec<u64> = Vec::new();
    let mut v128_consts: Vec<[u64; 2]> = Vec::new();
    let mut simd_masks: Vec<[u8; 16]> = Vec::new();
    let mut mem_args: Vec<MemGenArg> = Vec::new();
    let mut simd_mem_args: Vec<SimdMemArg> = Vec::new();
    let mut bulk_args: Vec<BulkMemArg> = Vec::new();
    let mut call_sites: Vec<CallSite> = Vec::new();
    let mut call_indirect_sites: Vec<CallIndirectSite> = Vec::new();
    let mut table_ranges: Vec<Range<u32>> = Vec::new();
    let mut table_targets: Vec<u32> = Vec::new();
    let mut blocks: Vec<BlockMeta> = Vec::new();
    let mut stack_types: Vec<FastType> = Vec::new();
    let mut static_labels: Vec<StaticLabel> = Vec::new();

    // Map Wasm source_pc -> emitted RegOp ip (for jump targets).
    let mut pc_to_ip: Vec<u32> = vec![u32::MAX; n + 1];
    // (ip_of_branch_op, target_source_pc)
    let mut branch_patches: Vec<(usize, usize)> = Vec::new();
    // (table_targets_index, target_source_pc)
    let mut table_patches: Vec<(usize, usize)> = Vec::new();

    let mut max_slots: usize = local_reg_slots_usize + bounds.stack_peak;
    let mut vstack: Vec<VSlot> = Vec::with_capacity(bounds.stack_peak);
    let mut labels: Vec<CompileLabel> = Vec::new();
    let mut dead = false;

    let mut in_block = false;
    let mut cur_block_ip: Option<usize> = None;
    let mut cur_block_start_ip: usize = 0;
    let mut cur_block_meta: u32 = BLOCK_META_NONE;
    let mut cur_block_ops: usize = 0;

    let slot_top = |vstack: &[VSlot]| -> u16 {
        vstack
            .last()
            .map_or(local_reg_slots, |s| s.slot_off + s.ty.slots())
    };

    let push_meta = |source_pc: usize,
                     vstack: &[VSlot],
                     labels: &[CompileLabel],
                     blocks: &mut Vec<BlockMeta>,
                     stack_types: &mut Vec<FastType>,
                     static_labels: &mut Vec<StaticLabel>|
     -> u32 {
        let stack_start = stack_types.len() as u32;
        for s in vstack {
            stack_types.push(s.ty);
        }
        let labels_start = static_labels.len() as u32;
        for l in labels {
            static_labels.push(StaticLabel {
                start: l.start as u32,
                end: l.end as u32,
                height: l.base_height as u16,
                params: l.params.len() as u16,
                results: l.results.len() as u16,
                loop_: l.loop_,
            });
        }
        let idx = blocks.len() as u32;
        blocks.push(BlockMeta {
            source_pc: source_pc as u32,
            fuel: 0,
            stack_start,
            stack_len: vstack.len() as u16,
            labels_start,
            labels_len: labels.len() as u16,
        });
        idx
    };

    let close_block = |in_block: &mut bool,
                       cur_block_ip: &mut Option<usize>,
                       cur_block_meta: &mut u32,
                       cur_block_ops: &mut usize,
                       ops: &mut [RegOp],
                       blocks: &mut [BlockMeta]| {
        if let Some(bip) = cur_block_ip.take() {
            let f = *cur_block_ops as u16;
            match &mut ops[bip] {
                RegOp::Block { fuel, .. } | RegOp::CheckBlock { fuel, .. } => *fuel = f,
                _ => {}
            }
            if *cur_block_meta != BLOCK_META_NONE {
                blocks[*cur_block_meta as usize].fuel = f;
            }
        }
        *in_block = false;
        *cur_block_meta = BLOCK_META_NONE;
        *cur_block_ops = 0;
    };

    let flush_local = |local_slot: u16, vstack: &mut [VSlot], ops: &mut Vec<RegOp>| {
        for slot in vstack.iter_mut() {
            if slot.src == VVal::Local(local_slot) {
                let dst = slot.slot_off;
                if slot.ty == FastType::V128 {
                    ops.push(RegOp::MovV128 {
                        dst,
                        src: local_slot,
                    });
                } else {
                    ops.push(RegOp::Mov {
                        dst,
                        src: local_slot,
                    });
                }
                slot.src = VVal::Slot(dst);
            }
        }
    };

    let flush_all = |vstack: &mut [VSlot], ops: &mut Vec<RegOp>, consts: &mut Vec<u64>| {
        for slot in vstack.iter_mut() {
            let dst = slot.slot_off;
            match slot.src {
                VVal::Slot(s) if s == dst => {}
                VVal::Slot(s) | VVal::Local(s) => {
                    if slot.ty == FastType::V128 {
                        ops.push(RegOp::MovV128 { dst, src: s });
                    } else {
                        ops.push(RegOp::Mov { dst, src: s });
                    }
                    slot.src = VVal::Slot(dst);
                }
                VVal::Imm32(val) => {
                    ops.push(RegOp::Const32 { dst, val });
                    slot.src = VVal::Slot(dst);
                }
                VVal::Imm64(val) => {
                    let idx = push_const(consts, val);
                    ops.push(RegOp::Const64 { dst, idx });
                    slot.src = VVal::Slot(dst);
                }
                VVal::ImmV128(idx) => {
                    ops.push(RegOp::ConstV128 { dst, idx });
                    slot.src = VVal::Slot(dst);
                }
            }
        }
    };

    let mat = |v: VVal,
               tmp_slot: u16,
               max_slots: &mut usize,
               ops: &mut Vec<RegOp>,
               consts: &mut Vec<u64>|
     -> u16 {
        match v {
            VVal::Slot(s) | VVal::Local(s) => s,
            VVal::Imm32(val) => {
                *max_slots = (*max_slots).max(tmp_slot as usize + 1);
                ops.push(RegOp::Const32 { dst: tmp_slot, val });
                tmp_slot
            }
            VVal::Imm64(val) => {
                *max_slots = (*max_slots).max(tmp_slot as usize + 1);
                let idx = push_const(consts, val);
                ops.push(RegOp::Const64 { dst: tmp_slot, idx });
                tmp_slot
            }
            VVal::ImmV128(idx) => {
                *max_slots = (*max_slots).max(tmp_slot as usize + 2);
                ops.push(RegOp::ConstV128 { dst: tmp_slot, idx });
                tmp_slot
            }
        }
    };

    let mut pc = 0usize;
    while pc < n {
        if slot_top(&vstack) as usize + 8 > MAX_FN_SLOTS {
            return None;
        }
        let insn = &code.instructions[pc];

        // Handle dead (unreachable) regions up to matching `else` or `end`.
        if dead {
            match insn.opcode {
                0x02..=0x04 => {
                    let (params, results) = block_sig(module, &insn.immediate)?;
                    let param_slots = params.iter().map(|t| t.slots()).sum();
                    let result_slots = results.iter().map(|t| t.slots()).sum();
                    let (end, other) = module.data.control.get(body, pc)?;
                    labels.push(CompileLabel {
                        start: pc + 1,
                        end,
                        other,
                        loop_: insn.opcode == 0x03,
                        base_height: 0,
                        base_slot: local_reg_slots,
                        prefix: Vec::new(),
                        params,
                        results,
                        param_slots,
                        result_slots,
                        parent_dead: true,
                    });
                }
                0x05 => {
                    let top = labels.last()?;
                    if !top.parent_dead {
                        // Entering live `else` branch!
                        dead = false;
                        vstack.clear();
                        let mut off = local_reg_slots;
                        for &ty in top.prefix.iter().chain(&top.params) {
                            vstack.push(VSlot {
                                src: VVal::Slot(off),
                                slot_off: off,
                                ty,
                            });
                            off += ty.slots();
                        }
                        max_slots = max_slots.max(off as usize);
                    }
                }
                0x0b => {
                    if let Some(label) = labels.pop() {
                        if !label.parent_dead {
                            dead = false;
                            vstack.clear();
                            let mut off = local_reg_slots;
                            for &ty in label.prefix.iter().chain(&label.results) {
                                vstack.push(VSlot {
                                    src: VVal::Slot(off),
                                    slot_off: off,
                                    ty,
                                });
                                off += ty.slots();
                            }
                            max_slots = max_slots.max(off as usize);
                            if is_block_start[pc] {
                                pc_to_ip[pc] = ops.len() as u32;
                                if fuel_blocks {
                                    let meta = push_meta(
                                        pc,
                                        &vstack,
                                        &labels,
                                        &mut blocks,
                                        &mut stack_types,
                                        &mut static_labels,
                                    );
                                    blocks[meta as usize].labels_len += 1;
                                    static_labels.push(StaticLabel {
                                        start: label.start as u32,
                                        end: label.end as u32,
                                        height: label.base_height as u16,
                                        params: label.params.len() as u16,
                                        results: label.results.len() as u16,
                                        loop_: label.loop_,
                                    });
                                    cur_block_ip = Some(ops.len());
                                    cur_block_meta = meta;
                                    ops.push(RegOp::Block { meta, fuel: 0 });
                                    cur_block_ops = 1;
                                    close_block(
                                        &mut in_block,
                                        &mut cur_block_ip,
                                        &mut cur_block_meta,
                                        &mut cur_block_ops,
                                        &mut ops,
                                        &mut blocks,
                                    );
                                } else {
                                    cur_block_start_ip = ops.len();
                                }
                            }
                        }
                    } else {
                        dead = false;
                    }
                }
                _ => {}
            }
            pc += 1;
            continue;
        }

        // Start a new basic block if required or if the current block hit MAX_BLOCK_OPS.
        let hit_max_ops = cur_block_ops >= MAX_BLOCK_OPS;
        let need_new_block = !in_block || is_block_start[pc] || hit_max_ops;
        if need_new_block {
            if in_block {
                flush_all(&mut vstack, &mut ops, &mut consts);
                close_block(
                    &mut in_block,
                    &mut cur_block_ip,
                    &mut cur_block_meta,
                    &mut cur_block_ops,
                    &mut ops,
                    &mut blocks,
                );
            }
            pc_to_ip[pc] = ops.len() as u32;
            in_block = true;
            cur_block_ops = 0;
            if fuel_blocks || is_check_block[pc] || hit_max_ops {
                let meta = push_meta(
                    pc,
                    &vstack,
                    &labels,
                    &mut blocks,
                    &mut stack_types,
                    &mut static_labels,
                );
                cur_block_ip = Some(ops.len());
                cur_block_meta = meta;
                if is_check_block[pc] || hit_max_ops {
                    ops.push(RegOp::CheckBlock { meta, fuel: 0 });
                } else {
                    ops.push(RegOp::Block { meta, fuel: 0 });
                }
            } else {
                cur_block_ip = None;
                cur_block_meta = BLOCK_META_NONE;
            }
            cur_block_start_ip = ops.len();
        }

        // Unsupported opcode -> emit Bail at this block boundary and end block.
        if !is_supported_op(module, insn) {
            let meta = if cur_block_ip.take().is_some() {
                ops.pop();
                cur_block_meta
            } else {
                push_meta(
                    pc,
                    &vstack,
                    &labels,
                    &mut blocks,
                    &mut stack_types,
                    &mut static_labels,
                )
            };
            in_block = false;
            cur_block_meta = BLOCK_META_NONE;
            cur_block_ops = 0;
            ops.push(RegOp::Bail { meta });
            if pc + 1 < n {
                break;
            }
            pc += 1;
            continue;
        }

        let src_off = cur_block_ops as u8;
        cur_block_ops += 1;

        // Helper: peek if next instruction is `local.set` / `local.tee` in the same block.
        let try_direct_local_dst = |pc: usize,
                                    cur_block_ops: usize,
                                    vstack: &mut Vec<VSlot>,
                                    ops: &mut Vec<RegOp>,
                                    res_ty: FastType|
         -> (u16, usize, bool) {
            if pc + 1 < n && !is_block_start[pc + 1] && cur_block_ops < MAX_BLOCK_OPS {
                let next = &code.instructions[pc + 1];
                if let Immediate::Index(l) = next.immediate
                    && (next.opcode == 0x21 || next.opcode == 0x22)
                {
                    let l_slot = local_slots[l as usize];
                    flush_local(l_slot, vstack, ops);
                    return (l_slot, 1, next.opcode == 0x22);
                }
            }
            let _ = res_ty;
            (slot_top(vstack), 0, false)
        };

        let finish_dst = |dst: u16,
                          consumed_next: usize,
                          is_tee: bool,
                          res_ty: FastType,
                          vstack: &mut Vec<VSlot>,
                          max_slots: &mut usize,
                          cur_block_ops: &mut usize,
                          pc: &mut usize| {
            if consumed_next > 0 {
                *cur_block_ops += consumed_next;
                *pc += consumed_next;
                if is_tee {
                    let slot_off = slot_top(vstack);
                    vstack.push(VSlot {
                        src: VVal::Local(dst),
                        slot_off,
                        ty: res_ty,
                    });
                    *max_slots = (*max_slots).max(slot_off as usize + res_ty.slots() as usize);
                }
            } else {
                vstack.push(VSlot {
                    src: VVal::Slot(dst),
                    slot_off: dst,
                    ty: res_ty,
                });
                *max_slots = (*max_slots).max(dst as usize + res_ty.slots() as usize);
            }
        };

        match insn.opcode {
            0x00 => {
                // unreachable
                ops.push(RegOp::TrapUnreachable { src_off });
                close_block(
                    &mut in_block,
                    &mut cur_block_ip,
                    &mut cur_block_meta,
                    &mut cur_block_ops,
                    &mut ops,
                    &mut blocks,
                );
                dead = true;
            }
            0x01 => {
                // nop (fuel already counted in cur_block_ops)
            }
            0x02 | 0x03 => {
                // block / loop
                let (params, results) = block_sig(module, &insn.immediate)?;
                let param_slots = params.iter().map(|t| t.slots()).sum();
                let result_slots = results.iter().map(|t| t.slots()).sum();
                let (end, other) = module.data.control.get(body, pc)?;
                if insn.opcode == 0x03 {
                    flush_all(&mut vstack, &mut ops, &mut consts);
                }
                let base_height = vstack.len().checked_sub(params.len())?;
                let base_slot = if base_height < vstack.len() {
                    vstack[base_height].slot_off
                } else {
                    slot_top(&vstack)
                };
                let prefix = vstack[..base_height].iter().map(|s| s.ty).collect();
                labels.push(CompileLabel {
                    start: pc + 1,
                    end,
                    other,
                    loop_: insn.opcode == 0x03,
                    base_height,
                    base_slot,
                    prefix,
                    params,
                    results,
                    param_slots,
                    result_slots,
                    parent_dead: false,
                });
                if insn.opcode == 0x03 {
                    close_block(
                        &mut in_block,
                        &mut cur_block_ip,
                        &mut cur_block_meta,
                        &mut cur_block_ops,
                        &mut ops,
                        &mut blocks,
                    );
                }
            }
            0x04 => {
                // if
                let cond_slot = vstack.pop()?;
                let (params, results) = block_sig(module, &insn.immediate)?;
                let param_slots = params.iter().map(|t| t.slots()).sum();
                let result_slots = results.iter().map(|t| t.slots()).sum();
                let (end, other) = module.data.control.get(body, pc)?;
                flush_all(&mut vstack, &mut ops, &mut consts);
                let cond_reg = mat(
                    cond_slot.src,
                    slot_top(&vstack),
                    &mut max_slots,
                    &mut ops,
                    &mut consts,
                );
                let base_height = vstack.len().checked_sub(params.len())?;
                let base_slot = if base_height < vstack.len() {
                    vstack[base_height].slot_off
                } else {
                    slot_top(&vstack)
                };
                let prefix = vstack[..base_height].iter().map(|s| s.ty).collect();
                labels.push(CompileLabel {
                    start: pc + 1,
                    end,
                    other,
                    loop_: false,
                    base_height,
                    base_slot,
                    prefix,
                    params,
                    results,
                    param_slots,
                    result_slots,
                    parent_dead: false,
                });
                let false_pc = other.map_or(end, |e| e + 1);
                let br_ip = ops.len();
                ops.push(RegOp::BrIfZero {
                    cond: cond_reg,
                    target: 0,
                });
                branch_patches.push((br_ip, false_pc));
                close_block(
                    &mut in_block,
                    &mut cur_block_ip,
                    &mut cur_block_meta,
                    &mut cur_block_ops,
                    &mut ops,
                    &mut blocks,
                );
            }
            0x05 => {
                // else: end of then-branch -> jump to `end`, then start else-branch at `pc + 1`
                let top = labels.last()?;
                flush_all(&mut vstack, &mut ops, &mut consts);
                let jmp_ip = ops.len();
                ops.push(RegOp::Jump { target: 0 });
                branch_patches.push((jmp_ip, top.end));
                close_block(
                    &mut in_block,
                    &mut cur_block_ip,
                    &mut cur_block_meta,
                    &mut cur_block_ops,
                    &mut ops,
                    &mut blocks,
                );
                vstack.clear();
                let mut off = local_reg_slots;
                for &ty in top.prefix.iter().chain(&top.params) {
                    vstack.push(VSlot {
                        src: VVal::Slot(off),
                        slot_off: off,
                        ty,
                    });
                    off += ty.slots();
                }
                max_slots = max_slots.max(off as usize);
            }
            0x0b => {
                // end
                if let Some(_label) = labels.pop() {
                    // Block end: fallthrough.
                } else {
                    flush_all(&mut vstack, &mut ops, &mut consts);
                    let count = result_types.len();
                    let start = vstack.len().checked_sub(count)?;
                    let base = if count > 0 {
                        vstack[start].slot_off
                    } else {
                        local_reg_slots
                    };
                    ops.push(RegOp::Return {
                        base,
                        count: result_reg_slots,
                    });
                    close_block(
                        &mut in_block,
                        &mut cur_block_ip,
                        &mut cur_block_meta,
                        &mut cur_block_ops,
                        &mut ops,
                        &mut blocks,
                    );
                }
            }
            0x0c => {
                // br depth
                let Immediate::Index(depth) = insn.immediate else {
                    return None;
                };
                let depth = depth as usize;
                flush_all(&mut vstack, &mut ops, &mut consts);
                if depth == labels.len() {
                    let count = result_types.len();
                    let start = vstack.len().checked_sub(count)?;
                    let base = if count > 0 {
                        vstack[start].slot_off
                    } else {
                        local_reg_slots
                    };
                    ops.push(RegOp::Return {
                        base,
                        count: result_reg_slots,
                    });
                } else {
                    let idx = labels.len().checked_sub(depth + 1)?;
                    let label = &labels[idx];
                    let target_pc = if label.loop_ {
                        label.start
                    } else {
                        label.end + 1
                    };
                    let (arity, arity_slots) = if label.loop_ {
                        (label.params.len(), label.param_slots)
                    } else {
                        (label.results.len(), label.result_slots)
                    };
                    let src_start = vstack.len().checked_sub(arity)?;
                    let src_base = if arity > 0 {
                        vstack[src_start].slot_off
                    } else {
                        slot_top(&vstack)
                    };
                    let dst_base = label.base_slot;
                    if arity_slots > 0 && src_base != dst_base {
                        ops.push(RegOp::MoveRange {
                            dst_base,
                            src_base,
                            count: arity_slots,
                        });
                        let jmp_ip = ops.len();
                        ops.push(RegOp::Jump { target: 0 });
                        branch_patches.push((jmp_ip, target_pc));
                    } else if ops.len() > cur_block_start_ip
                        && let Some(RegOp::I32AddImm { dst, src, imm }) = ops.last().copied()
                        && dst == src
                    {
                        // Fuse `r = r + imm; br target` (common top-tested loop backedge).
                        ops.pop();
                        let jmp_ip = ops.len();
                        ops.push(RegOp::I32AddImmJump {
                            reg: dst,
                            imm,
                            target: 0,
                        });
                        branch_patches.push((jmp_ip, target_pc));
                    } else {
                        let jmp_ip = ops.len();
                        ops.push(RegOp::Jump { target: 0 });
                        branch_patches.push((jmp_ip, target_pc));
                    }
                }
                close_block(
                    &mut in_block,
                    &mut cur_block_ip,
                    &mut cur_block_meta,
                    &mut cur_block_ops,
                    &mut ops,
                    &mut blocks,
                );
                dead = true;
            }
            0x0d => {
                // br_if depth
                let Immediate::Index(depth) = insn.immediate else {
                    return None;
                };
                let depth = depth as usize;
                let cond_slot = vstack.pop()?;
                flush_all(&mut vstack, &mut ops, &mut consts);
                let cond_reg = mat(
                    cond_slot.src,
                    slot_top(&vstack),
                    &mut max_slots,
                    &mut ops,
                    &mut consts,
                );
                if depth == labels.len() {
                    let count = result_types.len();
                    let start = vstack.len().checked_sub(count)?;
                    let base = if count > 0 {
                        vstack[start].slot_off
                    } else {
                        local_reg_slots
                    };
                    let skip_ip = ops.len();
                    ops.push(RegOp::BrIfZero {
                        cond: cond_reg,
                        target: (skip_ip + 2) as u32,
                    });
                    ops.push(RegOp::Return {
                        base,
                        count: result_reg_slots,
                    });
                } else {
                    let idx = labels.len().checked_sub(depth + 1)?;
                    let label = &labels[idx];
                    let target_pc = if label.loop_ {
                        label.start
                    } else {
                        label.end + 1
                    };
                    let (arity, arity_slots) = if label.loop_ {
                        (label.params.len(), label.param_slots)
                    } else {
                        (label.results.len(), label.result_slots)
                    };
                    let src_start = vstack.len().checked_sub(arity)?;
                    let src_base = if arity > 0 {
                        vstack[src_start].slot_off
                    } else {
                        slot_top(&vstack)
                    };
                    let dst_base = label.base_slot;
                    if arity_slots > 0 && src_base != dst_base {
                        let skip_ip = ops.len();
                        ops.push(RegOp::BrIfZero {
                            cond: cond_reg,
                            target: (skip_ip + 3) as u32,
                        });
                        ops.push(RegOp::MoveRange {
                            dst_base,
                            src_base,
                            count: arity_slots,
                        });
                        let jmp_ip = ops.len();
                        ops.push(RegOp::Jump { target: 0 });
                        branch_patches.push((jmp_ip, target_pc));
                    } else {
                        let br_ip = ops.len();
                        ops.push(RegOp::BrIf {
                            cond: cond_reg,
                            target: 0,
                        });
                        branch_patches.push((br_ip, target_pc));
                    }
                }
                close_block(
                    &mut in_block,
                    &mut cur_block_ip,
                    &mut cur_block_meta,
                    &mut cur_block_ops,
                    &mut ops,
                    &mut blocks,
                );
            }
            0x0e => {
                // br_table
                let Immediate::BrTable(depths, default) = &insn.immediate else {
                    return None;
                };
                let sel_slot = vstack.pop()?;
                flush_all(&mut vstack, &mut ops, &mut consts);
                let sel_reg = mat(
                    sel_slot.src,
                    slot_top(&vstack),
                    &mut max_slots,
                    &mut ops,
                    &mut consts,
                );
                let table_idx = table_ranges.len() as u32;
                let t_start = table_targets.len() as u32;
                ops.push(RegOp::BrTable {
                    selector: sel_reg,
                    table_idx,
                });
                close_block(
                    &mut in_block,
                    &mut cur_block_ip,
                    &mut cur_block_meta,
                    &mut cur_block_ops,
                    &mut ops,
                    &mut blocks,
                );
                for &d in depths.iter().chain(std::iter::once(default)) {
                    let depth = d as usize;
                    if depth == labels.len() {
                        let count = result_types.len();
                        let start = vstack.len().checked_sub(count)?;
                        let base = if count > 0 {
                            vstack[start].slot_off
                        } else {
                            local_reg_slots
                        };
                        let tramp_ip = ops.len() as u32;
                        ops.push(RegOp::Return {
                            base,
                            count: result_reg_slots,
                        });
                        table_targets.push(tramp_ip);
                    } else {
                        let idx = labels.len().checked_sub(depth + 1)?;
                        let label = &labels[idx];
                        let target_pc = if label.loop_ {
                            label.start
                        } else {
                            label.end + 1
                        };
                        let (arity, arity_slots) = if label.loop_ {
                            (label.params.len(), label.param_slots)
                        } else {
                            (label.results.len(), label.result_slots)
                        };
                        let src_start = vstack.len().checked_sub(arity)?;
                        let src_base = if arity > 0 {
                            vstack[src_start].slot_off
                        } else {
                            slot_top(&vstack)
                        };
                        let dst_base = label.base_slot;
                        if arity_slots > 0 && src_base != dst_base {
                            let tramp_ip = ops.len() as u32;
                            ops.push(RegOp::MoveRange {
                                dst_base,
                                src_base,
                                count: arity_slots,
                            });
                            let jmp_ip = ops.len();
                            ops.push(RegOp::Jump { target: 0 });
                            branch_patches.push((jmp_ip, target_pc));
                            table_targets.push(tramp_ip);
                        } else {
                            let ti = table_targets.len();
                            table_targets.push(0);
                            table_patches.push((ti, target_pc));
                        }
                    }
                }
                let t_end = table_targets.len() as u32;
                table_ranges.push(t_start..t_end);
                dead = true;
            }
            0x0f => {
                // return
                flush_all(&mut vstack, &mut ops, &mut consts);
                let count = result_types.len();
                let start = vstack.len().checked_sub(count)?;
                let base = if count > 0 {
                    vstack[start].slot_off
                } else {
                    local_reg_slots
                };
                ops.push(RegOp::Return {
                    base,
                    count: result_reg_slots,
                });
                close_block(
                    &mut in_block,
                    &mut cur_block_ip,
                    &mut cur_block_meta,
                    &mut cur_block_ops,
                    &mut ops,
                    &mut blocks,
                );
                dead = true;
            }
            0x10 => {
                // direct call
                let Immediate::Index(fi) = insn.immediate else {
                    return None;
                };
                let callee_ft = declared_func_type(module, fi)?;
                let arg_count = callee_ft.params.len();
                let arg_slots: u16 = callee_ft
                    .params
                    .iter()
                    .map(|&t| FastType::from_val(t).map(FastType::slots))
                    .collect::<Option<Vec<_>>>()?
                    .into_iter()
                    .sum();
                flush_all(&mut vstack, &mut ops, &mut consts);
                let meta_at_call = push_meta(
                    pc,
                    &vstack,
                    &labels,
                    &mut blocks,
                    &mut stack_types,
                    &mut static_labels,
                );
                let arg_start = vstack.len().checked_sub(arg_count)?;
                let arg_base = if arg_count > 0 {
                    vstack[arg_start].slot_off
                } else {
                    slot_top(&vstack)
                };
                vstack.truncate(arg_start);
                let meta_after_call = push_meta(
                    pc + 1,
                    &vstack,
                    &labels,
                    &mut blocks,
                    &mut stack_types,
                    &mut static_labels,
                );
                let call_idx = call_sites.len() as u32;
                call_sites.push(CallSite {
                    func_idx: fi,
                    arg_base,
                    arg_slots,
                    logical_arg_start: arg_start as u16,
                    meta_at_call,
                    meta_after_call,
                });
                ops.push(RegOp::Call { call_idx, src_off });
                let mut off = arg_base;
                for &rt in &callee_ft.results {
                    let ty = FastType::from_val(rt)?;
                    vstack.push(VSlot {
                        src: VVal::Slot(off),
                        slot_off: off,
                        ty,
                    });
                    off += ty.slots();
                }
                max_slots = max_slots.max(off as usize);
                close_block(
                    &mut in_block,
                    &mut cur_block_ip,
                    &mut cur_block_meta,
                    &mut cur_block_ops,
                    &mut ops,
                    &mut blocks,
                );
            }
            0x11 => {
                // call_indirect (type_idx, table_idx)
                let Immediate::Pair(type_idx, table_idx) = insn.immediate else {
                    return None;
                };
                let callee_ft = decoded.types.get(type_idx as usize)?;
                let table_is_64 = declared_table(module, table_idx)?.index_type == IndexType::I64;
                let arg_count = callee_ft.params.len();
                let arg_slots: u16 = callee_ft
                    .params
                    .iter()
                    .map(|&t| FastType::from_val(t).map(FastType::slots))
                    .collect::<Option<Vec<_>>>()?
                    .into_iter()
                    .sum();
                // Flush before popping the table index so `meta_at_call` preserves
                // `[args..., table_index]` on the canonical stack if this call bails!
                flush_all(&mut vstack, &mut ops, &mut consts);
                let meta_at_call = push_meta(
                    pc,
                    &vstack,
                    &labels,
                    &mut blocks,
                    &mut stack_types,
                    &mut static_labels,
                );
                let idx_slot = vstack.pop()?;
                let index_reg = idx_slot.slot_off;
                let arg_start = vstack.len().checked_sub(arg_count)?;
                let arg_base = if arg_count > 0 {
                    vstack[arg_start].slot_off
                } else {
                    slot_top(&vstack)
                };
                vstack.truncate(arg_start);
                let meta_after_call = push_meta(
                    pc + 1,
                    &vstack,
                    &labels,
                    &mut blocks,
                    &mut stack_types,
                    &mut static_labels,
                );
                let call_idx = call_indirect_sites.len() as u32;
                call_indirect_sites.push(CallIndirectSite {
                    type_idx,
                    table_idx,
                    index_reg,
                    arg_base,
                    arg_slots,
                    logical_arg_start: arg_start as u16,
                    table_is_64,
                    meta_at_call,
                    meta_after_call,
                });
                ops.push(RegOp::CallIndirect { call_idx, src_off });
                let mut off = arg_base;
                for &rt in &callee_ft.results {
                    let ty = FastType::from_val(rt)?;
                    vstack.push(VSlot {
                        src: VVal::Slot(off),
                        slot_off: off,
                        ty,
                    });
                    off += ty.slots();
                }
                max_slots = max_slots.max(off as usize);
                close_block(
                    &mut in_block,
                    &mut cur_block_ip,
                    &mut cur_block_meta,
                    &mut cur_block_ops,
                    &mut ops,
                    &mut blocks,
                );
            }
            0x12 => {
                // return_call (O(1) tail call reusing current frame in-place)
                let Immediate::Index(fi) = insn.immediate else {
                    return None;
                };
                let callee_ft = declared_func_type(module, fi)?;
                let arg_count = callee_ft.params.len();
                let arg_slots: u16 = callee_ft
                    .params
                    .iter()
                    .map(|&t| FastType::from_val(t).map(FastType::slots))
                    .collect::<Option<Vec<_>>>()?
                    .into_iter()
                    .sum();
                flush_all(&mut vstack, &mut ops, &mut consts);
                let meta_at_call = push_meta(
                    pc,
                    &vstack,
                    &labels,
                    &mut blocks,
                    &mut stack_types,
                    &mut static_labels,
                );
                let arg_start = vstack.len().checked_sub(arg_count)?;
                let arg_base = if arg_count > 0 {
                    vstack[arg_start].slot_off
                } else {
                    slot_top(&vstack)
                };
                let call_idx = call_sites.len() as u32;
                call_sites.push(CallSite {
                    func_idx: fi,
                    arg_base,
                    arg_slots,
                    logical_arg_start: arg_start as u16,
                    meta_at_call,
                    meta_after_call: BLOCK_META_NONE,
                });
                ops.push(RegOp::ReturnCall { call_idx, src_off });
                close_block(
                    &mut in_block,
                    &mut cur_block_ip,
                    &mut cur_block_meta,
                    &mut cur_block_ops,
                    &mut ops,
                    &mut blocks,
                );
                dead = true;
            }
            0x13 => {
                // return_call_indirect (type_idx, table_idx)
                let Immediate::Pair(type_idx, table_idx) = insn.immediate else {
                    return None;
                };
                let callee_ft = decoded.types.get(type_idx as usize)?;
                let table_is_64 = declared_table(module, table_idx)?.index_type == IndexType::I64;
                let arg_count = callee_ft.params.len();
                let arg_slots: u16 = callee_ft
                    .params
                    .iter()
                    .map(|&t| FastType::from_val(t).map(FastType::slots))
                    .collect::<Option<Vec<_>>>()?
                    .into_iter()
                    .sum();
                flush_all(&mut vstack, &mut ops, &mut consts);
                let meta_at_call = push_meta(
                    pc,
                    &vstack,
                    &labels,
                    &mut blocks,
                    &mut stack_types,
                    &mut static_labels,
                );
                let idx_slot = vstack.pop()?;
                let index_reg = idx_slot.slot_off;
                let arg_start = vstack.len().checked_sub(arg_count)?;
                let arg_base = if arg_count > 0 {
                    vstack[arg_start].slot_off
                } else {
                    slot_top(&vstack)
                };
                let call_idx = call_indirect_sites.len() as u32;
                call_indirect_sites.push(CallIndirectSite {
                    type_idx,
                    table_idx,
                    index_reg,
                    arg_base,
                    arg_slots,
                    logical_arg_start: arg_start as u16,
                    table_is_64,
                    meta_at_call,
                    meta_after_call: BLOCK_META_NONE,
                });
                ops.push(RegOp::ReturnCallIndirect { call_idx, src_off });
                close_block(
                    &mut in_block,
                    &mut cur_block_ip,
                    &mut cur_block_meta,
                    &mut cur_block_ops,
                    &mut ops,
                    &mut blocks,
                );
                dead = true;
            }
            0x1a => {
                // drop
                vstack.pop()?;
            }
            0x1b | 0x1c => {
                // select (scalar or v128)
                let cond_v = vstack.pop()?;
                let b_v = vstack.pop()?;
                let a_v = vstack.pop()?;
                let base_tmp = slot_top(&vstack);
                let w = a_v.ty.slots();
                let ra = mat(a_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                let rb = mat(b_v.src, base_tmp + w, &mut max_slots, &mut ops, &mut consts);
                let rc = mat(
                    cond_v.src,
                    base_tmp + 2 * w,
                    &mut max_slots,
                    &mut ops,
                    &mut consts,
                );
                let (dst, consumed, is_tee) =
                    try_direct_local_dst(pc, cur_block_ops, &mut vstack, &mut ops, a_v.ty);
                if a_v.ty == FastType::V128 {
                    ops.push(RegOp::SelectV128 {
                        dst,
                        a: ra,
                        b: rb,
                        cond: rc,
                    });
                } else {
                    ops.push(RegOp::Select {
                        dst,
                        a: ra,
                        b: rb,
                        cond: rc,
                    });
                }
                finish_dst(
                    dst,
                    consumed,
                    is_tee,
                    a_v.ty,
                    &mut vstack,
                    &mut max_slots,
                    &mut cur_block_ops,
                    &mut pc,
                );
            }
            0x20 => {
                // local.get
                let Immediate::Index(l) = insn.immediate else {
                    return None;
                };
                let l = l as usize;
                let ty = *local_types.get(l)?;
                let l_slot = *local_slots.get(l)?;
                let slot_off = slot_top(&vstack);
                vstack.push(VSlot {
                    src: VVal::Local(l_slot),
                    slot_off,
                    ty,
                });
                max_slots = max_slots.max(slot_off as usize + ty.slots() as usize);
            }
            0x21 | 0x22 => {
                // local.set / local.tee
                let Immediate::Index(l) = insn.immediate else {
                    return None;
                };
                let l_slot = *local_slots.get(l as usize)?;
                let val = vstack.pop()?;
                flush_local(l_slot, &mut vstack, &mut ops);
                match val.src {
                    VVal::Local(s) if s == l_slot => {}
                    VVal::Slot(s) | VVal::Local(s) => {
                        if val.ty == FastType::V128 {
                            ops.push(RegOp::MovV128 {
                                dst: l_slot,
                                src: s,
                            });
                        } else {
                            ops.push(RegOp::Mov {
                                dst: l_slot,
                                src: s,
                            });
                        }
                    }
                    VVal::Imm32(v) => ops.push(RegOp::Const32 {
                        dst: l_slot,
                        val: v,
                    }),
                    VVal::Imm64(v) => {
                        let idx = push_const(&mut consts, v);
                        ops.push(RegOp::Const64 { dst: l_slot, idx });
                    }
                    VVal::ImmV128(idx) => {
                        ops.push(RegOp::ConstV128 { dst: l_slot, idx });
                    }
                }
                if insn.opcode == 0x22 {
                    let slot_off = slot_top(&vstack);
                    vstack.push(VSlot {
                        src: VVal::Local(l_slot),
                        slot_off,
                        ty: val.ty,
                    });
                    max_slots = max_slots.max(slot_off as usize + val.ty.slots() as usize);
                }
            }
            0x23 => {
                // global.get
                let Immediate::Index(gi) = insn.immediate else {
                    return None;
                };
                let ty = FastType::from_val(declared_global_type(module, gi)?)?;
                let (dst, consumed, is_tee) =
                    try_direct_local_dst(pc, cur_block_ops, &mut vstack, &mut ops, ty);
                if ty == FastType::V128 {
                    ops.push(RegOp::GlobalGetV128 { dst, global: gi });
                } else {
                    ops.push(RegOp::GlobalGet { dst, global: gi });
                }
                finish_dst(
                    dst,
                    consumed,
                    is_tee,
                    ty,
                    &mut vstack,
                    &mut max_slots,
                    &mut cur_block_ops,
                    &mut pc,
                );
            }
            0x24 => {
                // global.set
                let Immediate::Index(gi) = insn.immediate else {
                    return None;
                };
                let val = vstack.pop()?;
                let src = mat(
                    val.src,
                    slot_top(&vstack),
                    &mut max_slots,
                    &mut ops,
                    &mut consts,
                );
                if val.ty == FastType::V128 {
                    ops.push(RegOp::GlobalSetV128 { src, global: gi });
                } else {
                    ops.push(RegOp::GlobalSet { src, global: gi });
                }
            }
            0x28..=0x3e => {
                // scalar memory load / store
                let Immediate::MemArg { memory, offset, .. } = insn.immediate else {
                    return None;
                };
                let (ty, _, is_store) = mem_sig(insn.opcode)?;
                if is_store {
                    let val_v = vstack.pop()?;
                    let addr_v = vstack.pop()?;
                    let base_tmp = slot_top(&vstack);
                    // Peephole: fuse preceding `I32AddImm { dst: tmp, src: base, imm }` into
                    // `StoreI32AddImm` / `StoreI32_8AddImm` when `offset == 0` and memory32.
                    let fused_add_imm = if memory == 0
                        && mem0_unshared
                        && !mem0_is_64
                        && offset == 0
                        && matches!(insn.opcode, 0x36 | 0x38 | 0x3a)
                        && let VVal::Slot(r_addr) = addr_v.src
                        && r_addr >= local_reg_slots
                        && val_v.src != VVal::Slot(r_addr)
                        && ops.len() > cur_block_start_ip
                        && let Some(RegOp::I32AddImm { dst, src, imm }) = ops.last().copied()
                        && dst == r_addr
                    {
                        ops.pop();
                        Some((src, imm as u32))
                    } else {
                        None
                    };
                    if let Some((base_reg, imm)) = fused_add_imm {
                        let r_val = mat(
                            val_v.src,
                            base_tmp + 1,
                            &mut max_slots,
                            &mut ops,
                            &mut consts,
                        );
                        match insn.opcode {
                            0x36 | 0x38 => ops.push(RegOp::StoreI32AddImm {
                                addr: base_reg,
                                val: r_val,
                                imm,
                                src_off,
                            }),
                            _ => ops.push(RegOp::StoreI32_8AddImm {
                                addr: base_reg,
                                val: r_val,
                                imm,
                                src_off,
                            }),
                        }
                    } else {
                        let r_addr =
                            mat(addr_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                        let r_val = mat(
                            val_v.src,
                            base_tmp + 1,
                            &mut max_slots,
                            &mut ops,
                            &mut consts,
                        );
                        if memory == 0
                            && mem0_unshared
                            && let Ok(off32) = u32::try_from(offset)
                        {
                            match insn.opcode {
                                0x36 | 0x38 => ops.push(RegOp::StoreI32 {
                                    addr: r_addr,
                                    val: r_val,
                                    offset: off32,
                                    src_off,
                                }),
                                0x37 | 0x39 => ops.push(RegOp::StoreI64 {
                                    addr: r_addr,
                                    val: r_val,
                                    offset: off32,
                                    src_off,
                                }),
                                0x3a => ops.push(RegOp::StoreI32_8 {
                                    addr: r_addr,
                                    val: r_val,
                                    offset: off32,
                                    src_off,
                                }),
                                0x3b => ops.push(RegOp::StoreI32_16 {
                                    addr: r_addr,
                                    val: r_val,
                                    offset: off32,
                                    src_off,
                                }),
                                _ => {
                                    let arg_idx = mem_args.len() as u32;
                                    mem_args.push(MemGenArg { memory, offset });
                                    ops.push(RegOp::MemGen {
                                        op: insn.opcode,
                                        addr: r_addr,
                                        val: r_val,
                                        arg_idx,
                                        src_off,
                                    });
                                }
                            }
                        } else {
                            let arg_idx = mem_args.len() as u32;
                            mem_args.push(MemGenArg { memory, offset });
                            ops.push(RegOp::MemGen {
                                op: insn.opcode,
                                addr: r_addr,
                                val: r_val,
                                arg_idx,
                                src_off,
                            });
                        }
                    }
                } else {
                    let addr_v = vstack.pop()?;
                    let base_tmp = slot_top(&vstack);
                    let r_addr = mat(addr_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                    let (dst, consumed, is_tee) =
                        try_direct_local_dst(pc, cur_block_ops, &mut vstack, &mut ops, ty);
                    if memory == 0
                        && mem0_unshared
                        && let Ok(off32) = u32::try_from(offset)
                    {
                        let can_fuse_addr = !mem0_is_64
                            && matches!(insn.opcode, 0x28 | 0x2a | 0x2d)
                            && r_addr >= local_reg_slots
                            && ops.len() > cur_block_start_ip;
                        let fused_addr = if can_fuse_addr
                            && let Some(RegOp::I32Add { dst: add_dst, a, b }) = ops.last().copied()
                            && add_dst == r_addr
                        {
                            ops.pop();
                            Some((a, b))
                        } else {
                            None
                        };
                        let fused_imm = if fused_addr.is_none()
                            && can_fuse_addr
                            && off32 == 0
                            && let Some(RegOp::I32AddImm {
                                dst: add_dst,
                                src,
                                imm,
                            }) = ops.last().copied()
                            && add_dst == r_addr
                        {
                            ops.pop();
                            Some((src, imm as u32))
                        } else {
                            None
                        };
                        match (insn.opcode, fused_addr, fused_imm) {
                            (0x28 | 0x2a, Some((a, b)), _) => ops.push(RegOp::LoadI32AddAddr {
                                dst,
                                a,
                                b,
                                offset: off32,
                                src_off,
                            }),
                            (0x2d, Some((a, b)), _) => ops.push(RegOp::LoadU8AddAddr {
                                dst,
                                a,
                                b,
                                offset: off32,
                                src_off,
                            }),
                            (0x28 | 0x2a, None, Some((base, imm))) => {
                                ops.push(RegOp::LoadI32AddImm {
                                    dst,
                                    addr: base,
                                    imm,
                                    src_off,
                                })
                            }
                            (0x2d, None, Some((base, imm))) => ops.push(RegOp::LoadU8AddImm {
                                dst,
                                addr: base,
                                imm,
                                src_off,
                            }),
                            (0x28 | 0x2a, None, None) => ops.push(RegOp::LoadI32 {
                                dst,
                                addr: r_addr,
                                offset: off32,
                                src_off,
                            }),
                            (0x29 | 0x2b, None, None) => ops.push(RegOp::LoadI64 {
                                dst,
                                addr: r_addr,
                                offset: off32,
                                src_off,
                            }),
                            (0x2d, None, None) => ops.push(RegOp::LoadI32U8 {
                                dst,
                                addr: r_addr,
                                offset: off32,
                                src_off,
                            }),
                            (0x2f, None, None) => ops.push(RegOp::LoadI32U16 {
                                dst,
                                addr: r_addr,
                                offset: off32,
                                src_off,
                            }),
                            _ => {
                                let arg_idx = mem_args.len() as u32;
                                mem_args.push(MemGenArg { memory, offset });
                                ops.push(RegOp::MemGen {
                                    op: insn.opcode,
                                    addr: r_addr,
                                    val: dst,
                                    arg_idx,
                                    src_off,
                                });
                            }
                        }
                    } else {
                        let arg_idx = mem_args.len() as u32;
                        mem_args.push(MemGenArg { memory, offset });
                        ops.push(RegOp::MemGen {
                            op: insn.opcode,
                            addr: r_addr,
                            val: dst,
                            arg_idx,
                            src_off,
                        });
                    }
                    finish_dst(
                        dst,
                        consumed,
                        is_tee,
                        ty,
                        &mut vstack,
                        &mut max_slots,
                        &mut cur_block_ops,
                        &mut pc,
                    );
                }
            }
            0x3f => {
                // memory.size
                let Immediate::Index(mem) = insn.immediate else {
                    return None;
                };
                let is64 = declared_memory(module, mem)?.index_type == IndexType::I64;
                let ty = if is64 { FastType::I64 } else { FastType::I32 };
                let (dst, consumed, is_tee) =
                    try_direct_local_dst(pc, cur_block_ops, &mut vstack, &mut ops, ty);
                ops.push(RegOp::MemSize { dst, mem });
                finish_dst(
                    dst,
                    consumed,
                    is_tee,
                    ty,
                    &mut vstack,
                    &mut max_slots,
                    &mut cur_block_ops,
                    &mut pc,
                );
            }
            0x40 => {
                // memory.grow
                let Immediate::Index(mem) = insn.immediate else {
                    return None;
                };
                let is64 = declared_memory(module, mem)?.index_type == IndexType::I64;
                let ty = if is64 { FastType::I64 } else { FastType::I32 };
                let delta_v = vstack.pop()?;
                let r_delta = mat(
                    delta_v.src,
                    slot_top(&vstack),
                    &mut max_slots,
                    &mut ops,
                    &mut consts,
                );
                let (dst, consumed, is_tee) =
                    try_direct_local_dst(pc, cur_block_ops, &mut vstack, &mut ops, ty);
                ops.push(RegOp::MemGrow {
                    dst,
                    delta: r_delta,
                    mem,
                    src_off,
                });
                finish_dst(
                    dst,
                    consumed,
                    is_tee,
                    ty,
                    &mut vstack,
                    &mut max_slots,
                    &mut cur_block_ops,
                    &mut pc,
                );
            }
            0xfc08 => {
                // memory.init data_seg mem
                let Immediate::Pair(seg, mem) = insn.immediate else {
                    return None;
                };
                let len_v = vstack.pop()?;
                let src_v = vstack.pop()?;
                let dst_v = vstack.pop()?;
                let base_tmp = slot_top(&vstack);
                let r_dst = mat(dst_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                let r_src = mat(
                    src_v.src,
                    base_tmp + 1,
                    &mut max_slots,
                    &mut ops,
                    &mut consts,
                );
                let r_len = mat(
                    len_v.src,
                    base_tmp + 2,
                    &mut max_slots,
                    &mut ops,
                    &mut consts,
                );
                let arg_idx = u16::try_from(bulk_args.len()).ok()?;
                bulk_args.push(BulkMemArg {
                    dst_mem: mem,
                    src_mem: seg,
                });
                ops.push(RegOp::MemBulkGen {
                    kind: 2,
                    src_off,
                    dst: r_dst,
                    src_or_val: r_src,
                    len: r_len,
                    arg_idx,
                });
            }
            0xfc09 => {
                // data.drop data_seg
                let Immediate::Index(seg) = insn.immediate else {
                    return None;
                };
                let arg_idx = u16::try_from(bulk_args.len()).ok()?;
                bulk_args.push(BulkMemArg {
                    dst_mem: seg,
                    src_mem: 0,
                });
                ops.push(RegOp::MemBulkGen {
                    kind: 3,
                    src_off,
                    dst: 0,
                    src_or_val: 0,
                    len: 0,
                    arg_idx,
                });
            }
            0xfc0a => {
                // memory.copy dst_mem src_mem
                let Immediate::Pair(dst_mem, src_mem) = insn.immediate else {
                    return None;
                };
                let len_v = vstack.pop()?;
                let src_v = vstack.pop()?;
                let dst_v = vstack.pop()?;
                let base_tmp = slot_top(&vstack);
                let r_dst = mat(dst_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                let r_src = mat(
                    src_v.src,
                    base_tmp + 1,
                    &mut max_slots,
                    &mut ops,
                    &mut consts,
                );
                let r_len = mat(
                    len_v.src,
                    base_tmp + 2,
                    &mut max_slots,
                    &mut ops,
                    &mut consts,
                );
                if dst_mem == 0 && src_mem == 0 && mem0_unshared {
                    ops.push(RegOp::MemCopy0 {
                        dst: r_dst,
                        src: r_src,
                        len: r_len,
                        src_off,
                    });
                } else {
                    let arg_idx = u16::try_from(bulk_args.len()).ok()?;
                    bulk_args.push(BulkMemArg { dst_mem, src_mem });
                    ops.push(RegOp::MemBulkGen {
                        kind: 0,
                        src_off,
                        dst: r_dst,
                        src_or_val: r_src,
                        len: r_len,
                        arg_idx,
                    });
                }
            }
            0xfc0b => {
                // memory.fill mem
                let Immediate::Index(mem) = insn.immediate else {
                    return None;
                };
                let len_v = vstack.pop()?;
                let val_v = vstack.pop()?;
                let dst_v = vstack.pop()?;
                let base_tmp = slot_top(&vstack);
                let r_dst = mat(dst_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                let r_val = mat(
                    val_v.src,
                    base_tmp + 1,
                    &mut max_slots,
                    &mut ops,
                    &mut consts,
                );
                let r_len = mat(
                    len_v.src,
                    base_tmp + 2,
                    &mut max_slots,
                    &mut ops,
                    &mut consts,
                );
                if mem == 0 && mem0_unshared {
                    ops.push(RegOp::MemFill0 {
                        dst: r_dst,
                        val: r_val,
                        len: r_len,
                        src_off,
                    });
                } else {
                    let arg_idx = u16::try_from(bulk_args.len()).ok()?;
                    bulk_args.push(BulkMemArg {
                        dst_mem: mem,
                        src_mem: mem,
                    });
                    ops.push(RegOp::MemBulkGen {
                        kind: 1,
                        src_off,
                        dst: r_dst,
                        src_or_val: r_val,
                        len: r_len,
                        arg_idx,
                    });
                }
            }
            0xfc0c => {
                // table.init elem_seg table
                let Immediate::Pair(elem, tbl) = insn.immediate else {
                    return None;
                };
                let len_v = vstack.pop()?;
                let src_v = vstack.pop()?;
                let dst_v = vstack.pop()?;
                let base_tmp = slot_top(&vstack);
                let r_dst = mat(dst_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                let r_src = mat(
                    src_v.src,
                    base_tmp + 1,
                    &mut max_slots,
                    &mut ops,
                    &mut consts,
                );
                let r_len = mat(
                    len_v.src,
                    base_tmp + 2,
                    &mut max_slots,
                    &mut ops,
                    &mut consts,
                );
                let arg_idx = u16::try_from(bulk_args.len()).ok()?;
                bulk_args.push(BulkMemArg {
                    dst_mem: tbl,
                    src_mem: elem,
                });
                ops.push(RegOp::MemBulkGen {
                    kind: 4,
                    src_off,
                    dst: r_dst,
                    src_or_val: r_src,
                    len: r_len,
                    arg_idx,
                });
            }
            0xfc0d => {
                // elem.drop elem_seg
                let Immediate::Index(elem) = insn.immediate else {
                    return None;
                };
                let arg_idx = u16::try_from(bulk_args.len()).ok()?;
                bulk_args.push(BulkMemArg {
                    dst_mem: elem,
                    src_mem: 0,
                });
                ops.push(RegOp::MemBulkGen {
                    kind: 5,
                    src_off,
                    dst: 0,
                    src_or_val: 0,
                    len: 0,
                    arg_idx,
                });
            }
            0xfc0e => {
                // table.copy dst_tbl src_tbl
                let Immediate::Pair(dst_tbl, src_tbl) = insn.immediate else {
                    return None;
                };
                let len_v = vstack.pop()?;
                let src_v = vstack.pop()?;
                let dst_v = vstack.pop()?;
                let base_tmp = slot_top(&vstack);
                let r_dst = mat(dst_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                let r_src = mat(
                    src_v.src,
                    base_tmp + 1,
                    &mut max_slots,
                    &mut ops,
                    &mut consts,
                );
                let r_len = mat(
                    len_v.src,
                    base_tmp + 2,
                    &mut max_slots,
                    &mut ops,
                    &mut consts,
                );
                let arg_idx = u16::try_from(bulk_args.len()).ok()?;
                bulk_args.push(BulkMemArg {
                    dst_mem: dst_tbl,
                    src_mem: src_tbl,
                });
                ops.push(RegOp::MemBulkGen {
                    kind: 6,
                    src_off,
                    dst: r_dst,
                    src_or_val: r_src,
                    len: r_len,
                    arg_idx,
                });
            }
            0xfc10 => {
                // table.size table
                let Immediate::Index(table) = insn.immediate else {
                    return None;
                };
                let is64 = declared_table(module, table)?.index_type == IndexType::I64;
                let ty = if is64 { FastType::I64 } else { FastType::I32 };
                let (dst, consumed, is_tee) =
                    try_direct_local_dst(pc, cur_block_ops, &mut vstack, &mut ops, ty);
                ops.push(RegOp::TableSize { dst, table });
                finish_dst(
                    dst,
                    consumed,
                    is_tee,
                    ty,
                    &mut vstack,
                    &mut max_slots,
                    &mut cur_block_ops,
                    &mut pc,
                );
            }
            0xfc13..=0xfc16 => {
                // wide-arithmetic: i64.add128, i64.sub128, i64.mul_wide_s, i64.mul_wide_u
                let kind = (insn.opcode - 0xfc13) as u8;
                let argc = if kind < 2 { 4usize } else { 2usize };
                flush_all(&mut vstack, &mut ops, &mut consts);
                let arg_start = vstack.len().checked_sub(argc)?;
                let base = vstack[arg_start].slot_off;
                vstack.truncate(arg_start);
                ops.push(RegOp::WideArith { kind, base });
                vstack.push(VSlot {
                    src: VVal::Slot(base),
                    slot_off: base,
                    ty: FastType::I64,
                });
                vstack.push(VSlot {
                    src: VVal::Slot(base + 1),
                    slot_off: base + 1,
                    ty: FastType::I64,
                });
                max_slots = max_slots.max(base as usize + 2);
            }
            0x41 => {
                let Immediate::I32(v) = insn.immediate else {
                    return None;
                };
                let slot_off = slot_top(&vstack);
                vstack.push(VSlot {
                    src: VVal::Imm32(v as u32),
                    slot_off,
                    ty: FastType::I32,
                });
                max_slots = max_slots.max(slot_off as usize + 1);
            }
            0x42 => {
                let Immediate::I64(v) = insn.immediate else {
                    return None;
                };
                let slot_off = slot_top(&vstack);
                vstack.push(VSlot {
                    src: VVal::Imm64(v as u64),
                    slot_off,
                    ty: FastType::I64,
                });
                max_slots = max_slots.max(slot_off as usize + 1);
            }
            0x43 => {
                let Immediate::F32(v) = insn.immediate else {
                    return None;
                };
                let slot_off = slot_top(&vstack);
                vstack.push(VSlot {
                    src: VVal::Imm32(v),
                    slot_off,
                    ty: FastType::F32,
                });
                max_slots = max_slots.max(slot_off as usize + 1);
            }
            0x44 => {
                let Immediate::F64(v) = insn.immediate else {
                    return None;
                };
                let slot_off = slot_top(&vstack);
                vstack.push(VSlot {
                    src: VVal::Imm64(v),
                    slot_off,
                    ty: FastType::F64,
                });
                max_slots = max_slots.max(slot_off as usize + 1);
            }
            0x45..=0xc4 | 0xfc00..=0xfc07 => {
                let (_in_ty, raw_res_ty, is_binary) = super::ir::numeric_signature(insn.opcode)?;
                let res_ty = fast_from_numeric(raw_res_ty);
                if !is_binary {
                    let a_v = vstack.pop()?;
                    // Peephole: `i32.eqz` followed by `br_if` (arity 0) or `if`!
                    if insn.opcode == 0x45
                        && pc + 1 < n
                        && !is_block_start[pc + 1]
                        && cur_block_ops < MAX_BLOCK_OPS
                    {
                        let next_insn = &code.instructions[pc + 1];
                        if next_insn.opcode == 0x0d
                            && let Immediate::Index(depth) = next_insn.immediate
                            && (depth as usize) < labels.len()
                        {
                            let l_idx = labels.len() - 1 - depth as usize;
                            let arity = if labels[l_idx].loop_ {
                                labels[l_idx].params.len()
                            } else {
                                labels[l_idx].results.len()
                            };
                            if arity == 0 {
                                let target_pc = if labels[l_idx].loop_ {
                                    labels[l_idx].start
                                } else {
                                    labels[l_idx].end + 1
                                };
                                flush_all(&mut vstack, &mut ops, &mut consts);
                                let ra = mat(
                                    a_v.src,
                                    slot_top(&vstack),
                                    &mut max_slots,
                                    &mut ops,
                                    &mut consts,
                                );
                                let br_ip = ops.len();
                                ops.push(RegOp::BrIfZero {
                                    cond: ra,
                                    target: 0,
                                });
                                branch_patches.push((br_ip, target_pc));
                                cur_block_ops += 1;
                                pc += 1;
                                close_block(
                                    &mut in_block,
                                    &mut cur_block_ip,
                                    &mut cur_block_meta,
                                    &mut cur_block_ops,
                                    &mut ops,
                                    &mut blocks,
                                );
                                pc += 1;
                                continue;
                            }
                        } else if next_insn.opcode == 0x04 {
                            let (params, results) = block_sig(module, &next_insn.immediate)?;
                            let param_slots = params.iter().map(|t| t.slots()).sum();
                            let result_slots = results.iter().map(|t| t.slots()).sum();
                            let (end, other) = module.data.control.get(body, pc + 1)?;
                            flush_all(&mut vstack, &mut ops, &mut consts);
                            let ra = mat(
                                a_v.src,
                                slot_top(&vstack),
                                &mut max_slots,
                                &mut ops,
                                &mut consts,
                            );
                            let br_ip = ops.len();
                            // `if (i32.eqz ra)` jumps to false branch when `ra != 0`!
                            ops.push(RegOp::BrIf {
                                cond: ra,
                                target: 0,
                            });
                            let base_height = vstack.len().checked_sub(params.len())?;
                            let base_slot = if base_height < vstack.len() {
                                vstack[base_height].slot_off
                            } else {
                                slot_top(&vstack)
                            };
                            let prefix = vstack[..base_height].iter().map(|s| s.ty).collect();
                            labels.push(CompileLabel {
                                start: pc + 2,
                                end,
                                other,
                                loop_: false,
                                base_height,
                                base_slot,
                                prefix,
                                params,
                                results,
                                param_slots,
                                result_slots,
                                parent_dead: false,
                            });
                            let false_pc = other.map_or(end, |e| e + 1);
                            branch_patches.push((br_ip, false_pc));
                            cur_block_ops += 1;
                            pc += 1;
                            close_block(
                                &mut in_block,
                                &mut cur_block_ip,
                                &mut cur_block_meta,
                                &mut cur_block_ops,
                                &mut ops,
                                &mut blocks,
                            );
                            pc += 1;
                            continue;
                        }
                    }
                    let ra = mat(
                        a_v.src,
                        slot_top(&vstack),
                        &mut max_slots,
                        &mut ops,
                        &mut consts,
                    );
                    let (dst, consumed, is_tee) =
                        try_direct_local_dst(pc, cur_block_ops, &mut vstack, &mut ops, res_ty);
                    match insn.opcode {
                        0x45 => ops.push(RegOp::I32Eqz { dst, src: ra }),
                        0xa7 | 0xad => ops.push(RegOp::I32WrapI64 { dst, src: ra }),
                        0xac => ops.push(RegOp::I64ExtendI32S { dst, src: ra }),
                        _ => ops.push(RegOp::Unary {
                            op: insn.opcode,
                            dst,
                            src: ra,
                            src_off,
                        }),
                    }
                    finish_dst(
                        dst,
                        consumed,
                        is_tee,
                        res_ty,
                        &mut vstack,
                        &mut max_slots,
                        &mut cur_block_ops,
                        &mut pc,
                    );
                } else {
                    let b_v = vstack.pop()?;
                    let a_v = vstack.pop()?;

                    // Peephole 1: `i32` compare (`0x46..=0x4f`) or `i64` compare (`0x51..=0x5a`)
                    // + `br_if` (arity 0) or `if`!
                    if ((0x46..=0x4f).contains(&insn.opcode)
                        || (0x51..=0x5a).contains(&insn.opcode))
                        && pc + 1 < n
                        && !is_block_start[pc + 1]
                        && cur_block_ops < MAX_BLOCK_OPS
                    {
                        let is_i64_cmp = insn.opcode >= 0x51;
                        let cmp = (insn.opcode - if is_i64_cmp { 0x51 } else { 0x46 }) as u8;
                        let next_insn = &code.instructions[pc + 1];
                        if next_insn.opcode == 0x0d
                            && let Immediate::Index(depth) = next_insn.immediate
                            && (depth as usize) < labels.len()
                        {
                            let l_idx = labels.len() - 1 - depth as usize;
                            let arity = if labels[l_idx].loop_ {
                                labels[l_idx].params.len()
                            } else {
                                labels[l_idx].results.len()
                            };
                            if arity == 0 {
                                let target_pc = if labels[l_idx].loop_ {
                                    labels[l_idx].start
                                } else {
                                    labels[l_idx].end + 1
                                };
                                flush_all(&mut vstack, &mut ops, &mut consts);
                                let base_tmp = slot_top(&vstack);
                                let br_ip = if is_i64_cmp {
                                    let ra = mat(
                                        a_v.src,
                                        base_tmp,
                                        &mut max_slots,
                                        &mut ops,
                                        &mut consts,
                                    );
                                    let rb = mat(
                                        b_v.src,
                                        base_tmp + 1,
                                        &mut max_slots,
                                        &mut ops,
                                        &mut consts,
                                    );
                                    let ip = ops.len();
                                    ops.push(RegOp::I64CmpBrIf {
                                        cmp,
                                        a: ra,
                                        b: rb,
                                        target: 0,
                                    });
                                    ip
                                } else if let VVal::Imm32(imm) = b_v.src {
                                    let ra = mat(
                                        a_v.src,
                                        base_tmp,
                                        &mut max_slots,
                                        &mut ops,
                                        &mut consts,
                                    );
                                    let ip = ops.len();
                                    ops.push(RegOp::I32CmpImmBrIf {
                                        cmp,
                                        a: ra,
                                        imm: imm as i32,
                                        target: 0,
                                    });
                                    ip
                                } else {
                                    let ra = mat(
                                        a_v.src,
                                        base_tmp,
                                        &mut max_slots,
                                        &mut ops,
                                        &mut consts,
                                    );
                                    let rb = mat(
                                        b_v.src,
                                        base_tmp + 1,
                                        &mut max_slots,
                                        &mut ops,
                                        &mut consts,
                                    );
                                    if (cmp == 3 || cmp == 2)
                                        && rb != ra
                                        && ops.len() > cur_block_start_ip
                                        && let Some(RegOp::I32AddImm { dst, src, imm }) =
                                            ops.last().copied()
                                        && dst == ra
                                        && src == ra
                                        && let Ok(step) = i16::try_from(imm)
                                    {
                                        ops.pop();
                                        let ip = ops.len();
                                        if cmp == 3 {
                                            ops.push(RegOp::I32IncLtUBrIf {
                                                reg: ra,
                                                limit: rb,
                                                step,
                                                target: 0,
                                            });
                                        } else {
                                            ops.push(RegOp::I32IncLtSBrIf {
                                                reg: ra,
                                                limit: rb,
                                                step,
                                                target: 0,
                                            });
                                        }
                                        ip
                                    } else {
                                        let ip = ops.len();
                                        ops.push(RegOp::I32CmpBrIf {
                                            cmp,
                                            a: ra,
                                            b: rb,
                                            target: 0,
                                        });
                                        ip
                                    }
                                };
                                branch_patches.push((br_ip, target_pc));
                                cur_block_ops += 1;
                                pc += 1;
                                close_block(
                                    &mut in_block,
                                    &mut cur_block_ip,
                                    &mut cur_block_meta,
                                    &mut cur_block_ops,
                                    &mut ops,
                                    &mut blocks,
                                );
                                pc += 1;
                                continue;
                            }
                        } else if next_insn.opcode == 0x04 {
                            let (params, results) = block_sig(module, &next_insn.immediate)?;
                            let param_slots = params.iter().map(|t| t.slots()).sum();
                            let result_slots = results.iter().map(|t| t.slots()).sum();
                            let (end, other) = module.data.control.get(body, pc + 1)?;
                            flush_all(&mut vstack, &mut ops, &mut consts);
                            let base_tmp = slot_top(&vstack);
                            let inv = invert_i32_cmp(cmp);
                            let br_ip = if is_i64_cmp {
                                let ra =
                                    mat(a_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                                let rb = mat(
                                    b_v.src,
                                    base_tmp + 1,
                                    &mut max_slots,
                                    &mut ops,
                                    &mut consts,
                                );
                                let ip = ops.len();
                                ops.push(RegOp::I64CmpBrIf {
                                    cmp: inv,
                                    a: ra,
                                    b: rb,
                                    target: 0,
                                });
                                ip
                            } else if let VVal::Imm32(imm) = b_v.src {
                                let ra =
                                    mat(a_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                                let ip = ops.len();
                                ops.push(RegOp::I32CmpImmBrIf {
                                    cmp: inv,
                                    a: ra,
                                    imm: imm as i32,
                                    target: 0,
                                });
                                ip
                            } else {
                                let ra =
                                    mat(a_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                                let rb = mat(
                                    b_v.src,
                                    base_tmp + 1,
                                    &mut max_slots,
                                    &mut ops,
                                    &mut consts,
                                );
                                let ip = ops.len();
                                ops.push(RegOp::I32CmpBrIf {
                                    cmp: inv,
                                    a: ra,
                                    b: rb,
                                    target: 0,
                                });
                                ip
                            };
                            let base_height = vstack.len().checked_sub(params.len())?;
                            let base_slot = if base_height < vstack.len() {
                                vstack[base_height].slot_off
                            } else {
                                slot_top(&vstack)
                            };
                            let prefix = vstack[..base_height].iter().map(|s| s.ty).collect();
                            labels.push(CompileLabel {
                                start: pc + 2,
                                end,
                                other,
                                loop_: false,
                                base_height,
                                base_slot,
                                prefix,
                                params,
                                results,
                                param_slots,
                                result_slots,
                                parent_dead: false,
                            });
                            let false_pc = other.map_or(end, |e| e + 1);
                            branch_patches.push((br_ip, false_pc));
                            cur_block_ops += 1;
                            pc += 1;
                            close_block(
                                &mut in_block,
                                &mut cur_block_ip,
                                &mut cur_block_meta,
                                &mut cur_block_ops,
                                &mut ops,
                                &mut blocks,
                            );
                            pc += 1;
                            continue;
                        }
                    }

                    // Constant-fold pure i32-i32 constant operations at compile time!
                    if let (VVal::Imm32(ca), VVal::Imm32(cb)) = (a_v.src, b_v.src)
                        && let Some(folded) = fold_i32_bin(insn.opcode, ca, cb)
                    {
                        let slot_off = slot_top(&vstack);
                        vstack.push(VSlot {
                            src: VVal::Imm32(folded),
                            slot_off,
                            ty: FastType::I32,
                        });
                        max_slots = max_slots.max(slot_off as usize + 1);
                        pc += 1;
                        continue;
                    }

                    let (dst, consumed, is_tee) =
                        try_direct_local_dst(pc, cur_block_ops, &mut vstack, &mut ops, res_ty);
                    let base_tmp = slot_top(&vstack);

                    // Immediate-RHS (or commutative LHS) superinstructions for i32/i64/f64 ALU!
                    let emitted_imm = match (insn.opcode, a_v.src, b_v.src) {
                        (0x6a, a, VVal::Imm32(c)) | (0x6a, VVal::Imm32(c), a) => {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            ops.push(RegOp::I32AddImm {
                                dst,
                                src: ra,
                                imm: c as i32,
                            });
                            true
                        }
                        (0x6b, a, VVal::Imm32(c)) => {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            ops.push(RegOp::I32SubImm {
                                dst,
                                src: ra,
                                imm: c as i32,
                            });
                            true
                        }
                        (0x6c, a, VVal::Imm32(c)) | (0x6c, VVal::Imm32(c), a) => {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            ops.push(RegOp::I32MulImm {
                                dst,
                                src: ra,
                                imm: c as i32,
                            });
                            true
                        }
                        (0x6e, a, VVal::Imm32(c)) if c != 0 => {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            if c == 1 {
                                if dst != ra {
                                    ops.push(RegOp::Mov { dst, src: ra });
                                }
                            } else if c.is_power_of_two() {
                                ops.push(RegOp::I32ShrUImm {
                                    dst,
                                    src: ra,
                                    shift: c.trailing_zeros(),
                                });
                            } else {
                                // Exact Lemire 64-bit reciprocal for all `u32 / c` (non-power-of-two c >= 3).
                                let magic = (u64::MAX / u64::from(c)) + 1;
                                let cidx = push_const(&mut consts, magic);
                                ops.push(RegOp::I32DivUConst { dst, src: ra, cidx });
                            }
                            true
                        }
                        (0x70, a, VVal::Imm32(c)) if c != 0 => {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            if c == 1 {
                                ops.push(RegOp::Const32 { dst, val: 0 });
                            } else if c.is_power_of_two() {
                                ops.push(RegOp::I32AndImm {
                                    dst,
                                    src: ra,
                                    imm: c - 1,
                                });
                            } else {
                                ops.push(RegOp::I32RemUImm {
                                    dst,
                                    src: ra,
                                    imm: c,
                                });
                            }
                            true
                        }
                        (0x71, a, VVal::Imm32(c)) | (0x71, VVal::Imm32(c), a) => {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            ops.push(RegOp::I32AndImm {
                                dst,
                                src: ra,
                                imm: c,
                            });
                            true
                        }
                        (0x72, a, VVal::Imm32(c)) | (0x72, VVal::Imm32(c), a) => {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            ops.push(RegOp::I32OrImm {
                                dst,
                                src: ra,
                                imm: c,
                            });
                            true
                        }
                        (0x73, a, VVal::Imm32(c)) | (0x73, VVal::Imm32(c), a) => {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            ops.push(RegOp::I32XorImm {
                                dst,
                                src: ra,
                                imm: c,
                            });
                            true
                        }
                        (0x74, a, VVal::Imm32(c)) => {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            ops.push(RegOp::I32ShlImm {
                                dst,
                                src: ra,
                                shift: c,
                            });
                            true
                        }
                        (0x75, a, VVal::Imm32(c)) => {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            ops.push(RegOp::I32ShrSImm {
                                dst,
                                src: ra,
                                shift: c,
                            });
                            true
                        }
                        (0x76, a, VVal::Imm32(c)) => {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            ops.push(RegOp::I32ShrUImm {
                                dst,
                                src: ra,
                                shift: c,
                            });
                            true
                        }
                        (0x77, a, VVal::Imm32(c)) => {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            ops.push(RegOp::I32RotlImm {
                                dst,
                                src: ra,
                                shift: c,
                            });
                            true
                        }
                        (0x78, a, VVal::Imm32(c)) => {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            ops.push(RegOp::I32RotrImm {
                                dst,
                                src: ra,
                                shift: c,
                            });
                            true
                        }
                        (op @ 0x46..=0x4f, a, VVal::Imm32(c)) => {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            ops.push(RegOp::I32CmpImm {
                                cmp: (op - 0x46) as u8,
                                dst,
                                a: ra,
                                imm: c as i32,
                            });
                            true
                        }
                        (0x7c, a, VVal::Imm64(c)) | (0x7c, VVal::Imm64(c), a)
                            if i32::try_from(c as i64).is_ok() =>
                        {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            ops.push(RegOp::I64AddImm {
                                dst,
                                src: ra,
                                imm: c as i64 as i32,
                            });
                            true
                        }
                        (0xa0, a, VVal::Imm64(c)) | (0xa0, VVal::Imm64(c), a) => {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            let cidx = push_const(&mut consts, c);
                            ops.push(RegOp::F64AddConst { dst, src: ra, cidx });
                            true
                        }
                        (0xa1, a, VVal::Imm64(c)) => {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            let cidx = push_const(&mut consts, c);
                            ops.push(RegOp::F64SubConst { dst, src: ra, cidx });
                            true
                        }
                        (0xa2, a, VVal::Imm64(c)) | (0xa2, VVal::Imm64(c), a) => {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            let cidx = push_const(&mut consts, c);
                            ops.push(RegOp::F64MulConst { dst, src: ra, cidx });
                            true
                        }
                        (0xa4, a, VVal::Imm64(c)) | (0xa4, VVal::Imm64(c), a) => {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            let cidx = push_const(&mut consts, c);
                            ops.push(RegOp::F64MinConst { dst, src: ra, cidx });
                            true
                        }
                        (0xa5, a, VVal::Imm64(c)) | (0xa5, VVal::Imm64(c), a) => {
                            let ra = mat(a, base_tmp, &mut max_slots, &mut ops, &mut consts);
                            let cidx = push_const(&mut consts, c);
                            ops.push(RegOp::F64MaxConst { dst, src: ra, cidx });
                            true
                        }
                        _ => false,
                    };

                    if !emitted_imm {
                        let ra = mat(a_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                        let rb = mat(b_v.src, base_tmp + 1, &mut max_slots, &mut ops, &mut consts);
                        let in_cur_block = ops.len() > cur_block_start_ip;
                        let fused_load_add = if in_cur_block && insn.opcode == 0x6a {
                            match ops.last().copied() {
                                Some(RegOp::LoadI32U8 {
                                    dst: ldst,
                                    addr,
                                    offset,
                                    src_off: l_src_off,
                                }) if ldst >= local_reg_slots
                                    && ((rb == ldst && ra == dst) || (ra == ldst && rb == dst))
                                    && addr != dst =>
                                {
                                    ops.pop();
                                    ops.push(RegOp::LoadU8Add {
                                        acc: dst,
                                        addr,
                                        offset,
                                        src_off: l_src_off,
                                    });
                                    true
                                }
                                Some(RegOp::LoadU8AddImm {
                                    dst: ldst,
                                    addr,
                                    imm,
                                    src_off: l_src_off,
                                }) if ldst >= local_reg_slots
                                    && ((rb == ldst && ra == dst) || (ra == ldst && rb == dst))
                                    && addr != dst =>
                                {
                                    ops.pop();
                                    ops.push(RegOp::LoadU8AddImmAcc {
                                        acc: dst,
                                        addr,
                                        imm,
                                        src_off: l_src_off,
                                    });
                                    true
                                }
                                _ => false,
                            }
                        } else if in_cur_block
                            && insn.opcode == 0xa0
                            && let Some(RegOp::LoadI64 {
                                dst: ldst,
                                addr,
                                offset,
                                src_off: l_src_off,
                            }) = ops.last().copied()
                            && ldst >= local_reg_slots
                            && ((rb == ldst && ra == dst) || (ra == ldst && rb == dst))
                            && addr != dst
                        {
                            ops.pop();
                            ops.push(RegOp::LoadF64Add {
                                acc: dst,
                                addr,
                                offset,
                                src_off: l_src_off,
                            });
                            true
                        } else {
                            false
                        };
                        if !fused_load_add {
                            let rop = match insn.opcode {
                                0x46..=0x4f => RegOp::I32Cmp {
                                    cmp: (insn.opcode - 0x46) as u8,
                                    dst,
                                    a: ra,
                                    b: rb,
                                },
                                0x51..=0x5a => RegOp::I64Cmp {
                                    cmp: (insn.opcode - 0x51) as u8,
                                    dst,
                                    a: ra,
                                    b: rb,
                                },
                                0x61..=0x66 => RegOp::F64Cmp {
                                    cmp: (insn.opcode - 0x61) as u8,
                                    dst,
                                    a: ra,
                                    b: rb,
                                },
                                0x6a => RegOp::I32Add { dst, a: ra, b: rb },
                                0x6b => RegOp::I32Sub { dst, a: ra, b: rb },
                                0x6c => RegOp::I32Mul { dst, a: ra, b: rb },
                                0x6d => RegOp::I32DivS {
                                    dst,
                                    a: ra,
                                    b: rb,
                                    src_off,
                                },
                                0x6e => RegOp::I32DivU {
                                    dst,
                                    a: ra,
                                    b: rb,
                                    src_off,
                                },
                                0x6f => RegOp::I32RemS {
                                    dst,
                                    a: ra,
                                    b: rb,
                                    src_off,
                                },
                                0x70 => RegOp::I32RemU {
                                    dst,
                                    a: ra,
                                    b: rb,
                                    src_off,
                                },
                                0x71 => RegOp::I32And { dst, a: ra, b: rb },
                                0x72 => RegOp::I32Or { dst, a: ra, b: rb },
                                0x73 => RegOp::I32Xor { dst, a: ra, b: rb },
                                0x74 => RegOp::I32Shl { dst, a: ra, b: rb },
                                0x75 => RegOp::I32ShrS { dst, a: ra, b: rb },
                                0x76 => RegOp::I32ShrU { dst, a: ra, b: rb },
                                0x77 => RegOp::I32Rotl { dst, a: ra, b: rb },
                                0x78 => RegOp::I32Rotr { dst, a: ra, b: rb },
                                0x7c => RegOp::I64Add { dst, a: ra, b: rb },
                                0x7d => RegOp::I64Sub { dst, a: ra, b: rb },
                                0x7e => RegOp::I64Mul { dst, a: ra, b: rb },
                                0x83 => RegOp::I64And { dst, a: ra, b: rb },
                                0x84 => RegOp::I64Or { dst, a: ra, b: rb },
                                0x85 => RegOp::I64Xor { dst, a: ra, b: rb },
                                0x86 => RegOp::I64Shl { dst, a: ra, b: rb },
                                0x87 => RegOp::I64ShrS { dst, a: ra, b: rb },
                                0x88 => RegOp::I64ShrU { dst, a: ra, b: rb },
                                0x92 => RegOp::F32Add { dst, a: ra, b: rb },
                                0x93 => RegOp::F32Sub { dst, a: ra, b: rb },
                                0x94 => RegOp::F32Mul { dst, a: ra, b: rb },
                                0x95 => RegOp::F32Div { dst, a: ra, b: rb },
                                0xa0 => RegOp::F64Add { dst, a: ra, b: rb },
                                0xa1 => RegOp::F64Sub { dst, a: ra, b: rb },
                                0xa2 => RegOp::F64Mul { dst, a: ra, b: rb },
                                0xa3 => RegOp::F64Div { dst, a: ra, b: rb },
                                0xa4 => RegOp::F64Min { dst, a: ra, b: rb },
                                0xa5 => RegOp::F64Max { dst, a: ra, b: rb },
                                _ => RegOp::Binary {
                                    op: insn.opcode,
                                    dst,
                                    a: ra,
                                    b: rb,
                                    src_off,
                                },
                            };
                            ops.push(rop);
                        }
                    }
                    finish_dst(
                        dst,
                        consumed,
                        is_tee,
                        res_ty,
                        &mut vstack,
                        &mut max_slots,
                        &mut cur_block_ops,
                        &mut pc,
                    );
                }
            }
            0xfd00..=0xfdff | 0xfa00..=0xfa13 => match simd_op_shape(insn.opcode)? {
                SimdShape::Mem => {
                    let (memory, offset, lane) = match insn.immediate {
                        Immediate::MemArg { memory, offset, .. } => (memory, offset, 0u8),
                        Immediate::MemLane {
                            memory,
                            offset,
                            lane,
                            ..
                        } => (memory, offset, lane),
                        _ => return None,
                    };
                    let sub = insn.opcode & 0xff;
                    let lane_op = (0x54..=0x5b).contains(&sub);
                    let is_store = sub == 11 || (0x58..=0x5b).contains(&sub);
                    if is_store {
                        let val_v = vstack.pop()?;
                        let addr_v = vstack.pop()?;
                        let base_tmp = slot_top(&vstack);
                        let r_addr =
                            mat(addr_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                        let r_val = mat(
                            val_v.src,
                            base_tmp + 1,
                            &mut max_slots,
                            &mut ops,
                            &mut consts,
                        );
                        if insn.opcode == 0xfd0b
                            && memory == 0
                            && mem0_unshared
                            && let Ok(off32) = u32::try_from(offset)
                        {
                            ops.push(RegOp::StoreV128 {
                                addr: r_addr,
                                val: r_val,
                                offset: off32,
                                src_off,
                            });
                        } else {
                            let arg_idx = simd_mem_args.len() as u32;
                            simd_mem_args.push(SimdMemArg {
                                memory,
                                offset,
                                src_vec: r_val,
                                lane,
                            });
                            ops.push(RegOp::SimdMemGen {
                                op: insn.opcode,
                                addr: r_addr,
                                val: r_val,
                                arg_idx,
                                src_off,
                            });
                        }
                    } else if lane_op {
                        let vec_v = vstack.pop()?;
                        let addr_v = vstack.pop()?;
                        let base_tmp = slot_top(&vstack);
                        let r_addr =
                            mat(addr_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                        let r_vec = mat(
                            vec_v.src,
                            base_tmp + 1,
                            &mut max_slots,
                            &mut ops,
                            &mut consts,
                        );
                        let (dst, consumed, is_tee) = try_direct_local_dst(
                            pc,
                            cur_block_ops,
                            &mut vstack,
                            &mut ops,
                            FastType::V128,
                        );
                        let arg_idx = simd_mem_args.len() as u32;
                        simd_mem_args.push(SimdMemArg {
                            memory,
                            offset,
                            src_vec: r_vec,
                            lane,
                        });
                        ops.push(RegOp::SimdMemGen {
                            op: insn.opcode,
                            addr: r_addr,
                            val: dst,
                            arg_idx,
                            src_off,
                        });
                        finish_dst(
                            dst,
                            consumed,
                            is_tee,
                            FastType::V128,
                            &mut vstack,
                            &mut max_slots,
                            &mut cur_block_ops,
                            &mut pc,
                        );
                    } else {
                        let addr_v = vstack.pop()?;
                        let r_addr = mat(
                            addr_v.src,
                            slot_top(&vstack),
                            &mut max_slots,
                            &mut ops,
                            &mut consts,
                        );
                        let (dst, consumed, is_tee) = try_direct_local_dst(
                            pc,
                            cur_block_ops,
                            &mut vstack,
                            &mut ops,
                            FastType::V128,
                        );
                        if insn.opcode == 0xfd00
                            && memory == 0
                            && mem0_unshared
                            && let Ok(off32) = u32::try_from(offset)
                        {
                            ops.push(RegOp::LoadV128 {
                                dst,
                                addr: r_addr,
                                offset: off32,
                                src_off,
                            });
                        } else {
                            let arg_idx = simd_mem_args.len() as u32;
                            simd_mem_args.push(SimdMemArg {
                                memory,
                                offset,
                                src_vec: dst,
                                lane: 0,
                            });
                            ops.push(RegOp::SimdMemGen {
                                op: insn.opcode,
                                addr: r_addr,
                                val: dst,
                                arg_idx,
                                src_off,
                            });
                        }
                        finish_dst(
                            dst,
                            consumed,
                            is_tee,
                            FastType::V128,
                            &mut vstack,
                            &mut max_slots,
                            &mut cur_block_ops,
                            &mut pc,
                        );
                    }
                }
                SimdShape::Const => {
                    let Immediate::Bytes(b) = &insn.immediate else {
                        return None;
                    };
                    if b.len() != 16 {
                        return None;
                    }
                    let lo = u64::from_le_bytes(b[0..8].try_into().ok()?);
                    let hi = u64::from_le_bytes(b[8..16].try_into().ok()?);
                    let pair = [lo, hi];
                    let idx = if v128_consts.len() <= 32
                        && let Some(pos) = v128_consts.iter().position(|&c| c == pair)
                    {
                        pos as u32
                    } else {
                        let i = v128_consts.len() as u32;
                        v128_consts.push(pair);
                        i
                    };
                    let slot_off = slot_top(&vstack);
                    vstack.push(VSlot {
                        src: VVal::ImmV128(idx),
                        slot_off,
                        ty: FastType::V128,
                    });
                    max_slots = max_slots.max(slot_off as usize + 2);
                }
                SimdShape::Shuffle => {
                    let Immediate::Bytes(b) = &insn.immediate else {
                        return None;
                    };
                    let mask: [u8; 16] = b.as_ref().try_into().ok()?;
                    let mask_idx = if simd_masks.len() <= 32
                        && let Some(pos) = simd_masks.iter().position(|&m| m == mask)
                    {
                        pos as u16
                    } else {
                        let i = u16::try_from(simd_masks.len()).ok()?;
                        simd_masks.push(mask);
                        i
                    };
                    let b_v = vstack.pop()?;
                    let a_v = vstack.pop()?;
                    let base_tmp = slot_top(&vstack);
                    let ra = mat(a_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                    let rb = mat(b_v.src, base_tmp + 2, &mut max_slots, &mut ops, &mut consts);
                    let (dst, consumed, is_tee) = try_direct_local_dst(
                        pc,
                        cur_block_ops,
                        &mut vstack,
                        &mut ops,
                        FastType::V128,
                    );
                    ops.push(RegOp::SimdShuffle {
                        dst,
                        a: ra,
                        b: rb,
                        mask_idx,
                    });
                    finish_dst(
                        dst,
                        consumed,
                        is_tee,
                        FastType::V128,
                        &mut vstack,
                        &mut max_slots,
                        &mut cur_block_ops,
                        &mut pc,
                    );
                }
                SimdShape::Unary { out_ty, has_lane } => {
                    let lane = if has_lane {
                        let Immediate::Index(l) = insn.immediate else {
                            return None;
                        };
                        u8::try_from(l).ok()?
                    } else {
                        0
                    };
                    let a_v = vstack.pop()?;
                    let ra = mat(
                        a_v.src,
                        slot_top(&vstack),
                        &mut max_slots,
                        &mut ops,
                        &mut consts,
                    );
                    let (dst, consumed, is_tee) =
                        try_direct_local_dst(pc, cur_block_ops, &mut vstack, &mut ops, out_ty);
                    ops.push(RegOp::SimdUnary {
                        op: insn.opcode,
                        dst,
                        src: ra,
                        lane,
                    });
                    finish_dst(
                        dst,
                        consumed,
                        is_tee,
                        out_ty,
                        &mut vstack,
                        &mut max_slots,
                        &mut cur_block_ops,
                        &mut pc,
                    );
                }
                SimdShape::Binary { has_lane } => {
                    let lane = if has_lane {
                        let Immediate::Index(l) = insn.immediate else {
                            return None;
                        };
                        u8::try_from(l).ok()?
                    } else {
                        0
                    };
                    let b_v = vstack.pop()?;
                    let a_v = vstack.pop()?;
                    let base_tmp = slot_top(&vstack);
                    let ra = mat(a_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                    let rb = mat(
                        b_v.src,
                        base_tmp + a_v.ty.slots(),
                        &mut max_slots,
                        &mut ops,
                        &mut consts,
                    );
                    let (dst, consumed, is_tee) = try_direct_local_dst(
                        pc,
                        cur_block_ops,
                        &mut vstack,
                        &mut ops,
                        FastType::V128,
                    );
                    ops.push(RegOp::SimdBinary {
                        op: insn.opcode,
                        dst,
                        a: ra,
                        b: rb,
                        lane,
                    });
                    finish_dst(
                        dst,
                        consumed,
                        is_tee,
                        FastType::V128,
                        &mut vstack,
                        &mut max_slots,
                        &mut cur_block_ops,
                        &mut pc,
                    );
                }
                SimdShape::Ternary => {
                    let c_v = vstack.pop()?;
                    let b_v = vstack.pop()?;
                    let a_v = vstack.pop()?;
                    let base_tmp = slot_top(&vstack);
                    let ra = mat(a_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                    let rb = mat(b_v.src, base_tmp + 2, &mut max_slots, &mut ops, &mut consts);
                    let rc = mat(c_v.src, base_tmp + 4, &mut max_slots, &mut ops, &mut consts);
                    let (dst, consumed, is_tee) = try_direct_local_dst(
                        pc,
                        cur_block_ops,
                        &mut vstack,
                        &mut ops,
                        FastType::V128,
                    );
                    ops.push(RegOp::SimdTernary {
                        op: insn.opcode,
                        dst,
                        a: ra,
                        b: rb,
                        c: rc,
                    });
                    finish_dst(
                        dst,
                        consumed,
                        is_tee,
                        FastType::V128,
                        &mut vstack,
                        &mut max_slots,
                        &mut cur_block_ops,
                        &mut pc,
                    );
                }
            },
            _ => return None,
        }

        pc += 1;
    }

    close_block(
        &mut in_block,
        &mut cur_block_ip,
        &mut cur_block_meta,
        &mut cur_block_ops,
        &mut ops,
        &mut blocks,
    );

    if max_slots > MAX_FN_SLOTS {
        return None;
    }

    // Patch all forward/backward branch targets; if any target is unresolved (e.g. past an early Bail), decline.
    for (br_ip, target_pc) in branch_patches {
        let target_ip = *pc_to_ip.get(target_pc)?;
        if target_ip == u32::MAX {
            return None;
        }
        match &mut ops[br_ip] {
            RegOp::Jump { target }
            | RegOp::BrIf { target, .. }
            | RegOp::BrIfZero { target, .. }
            | RegOp::I32CmpBrIf { target, .. }
            | RegOp::I32CmpImmBrIf { target, .. }
            | RegOp::I64CmpBrIf { target, .. }
            | RegOp::I32IncLtUBrIf { target, .. }
            | RegOp::I32IncLtSBrIf { target, .. }
            | RegOp::I32AddImmJump { target, .. } => *target = target_ip,
            _ => return None,
        }
    }
    for (ti, target_pc) in table_patches {
        let target_ip = *pc_to_ip.get(target_pc)?;
        if target_ip == u32::MAX {
            return None;
        }
        table_targets[ti] = target_ip;
    }

    Some(Arc::new(FastFunction {
        ops: ops.into_boxed_slice(),
        consts: consts.into_boxed_slice(),
        v128_consts: v128_consts.into_boxed_slice(),
        simd_masks: simd_masks.into_boxed_slice(),
        mem_args: mem_args.into_boxed_slice(),
        simd_mem_args: simd_mem_args.into_boxed_slice(),
        bulk_args: bulk_args.into_boxed_slice(),
        call_sites: call_sites.into_boxed_slice(),
        call_indirect_sites: call_indirect_sites.into_boxed_slice(),
        table_ranges: table_ranges.into_boxed_slice(),
        table_targets: table_targets.into_boxed_slice(),
        blocks: blocks.into_boxed_slice(),
        stack_types: stack_types.into_boxed_slice(),
        static_labels: static_labels.into_boxed_slice(),
        local_types: local_types.into_boxed_slice(),
        result_types: result_types.into_boxed_slice(),
        num_locals: num_locals as u16,
        local_reg_slots,
        slots: max_slots as u16,
        stack_peak: bounds.stack_peak,
        max_nesting: bounds.maximum_nesting,
        mem0_is_64,
        fuel_blocks,
    }))
}

#[inline(always)]
fn value_to_bits(v: Value) -> Option<u64> {
    match v {
        Value::I32(n) => Some(n as u32 as u64),
        Value::I64(n) => Some(n as u64),
        Value::F32(n) => Some(n as u64),
        Value::F64(n) => Some(n),
        _ => None,
    }
}

#[inline(always)]
fn read_v128(regs: &[u64], slot: u16) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[0..8].copy_from_slice(&regs[slot as usize].to_le_bytes());
    out[8..16].copy_from_slice(&regs[slot as usize + 1].to_le_bytes());
    out
}

#[inline(always)]
fn write_v128(regs: &mut [u64], slot: u16, bytes: [u8; 16]) {
    regs[slot as usize] = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
    regs[slot as usize + 1] = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
}

#[inline(always)]
fn slots_to_value(ty: FastType, slots: &[u64], off: usize) -> Value {
    match ty {
        FastType::I32 => Value::I32(slots[off] as u32 as i32),
        FastType::I64 => Value::I64(slots[off] as i64),
        FastType::F32 => Value::F32(slots[off] as u32),
        FastType::F64 => Value::F64(slots[off]),
        FastType::V128 => {
            let mut b = [0u8; 16];
            b[0..8].copy_from_slice(&slots[off].to_le_bytes());
            b[8..16].copy_from_slice(&slots[off + 1].to_le_bytes());
            Value::V128(V128(b))
        }
    }
}

#[inline(always)]
fn valtype_to_value(ty: ValType, bits: u64) -> Option<Value> {
    Some(match ty {
        ValType::I32 => Value::I32(bits as u32 as i32),
        ValType::I64 => Value::I64(bits as i64),
        ValType::F32 => Value::F32(bits as u32),
        ValType::F64 => Value::F64(bits),
        _ => return None,
    })
}

#[inline(never)]
fn simd_unary_slow(
    op: u16,
    lane: u8,
    regs: &mut [u64],
    dst: u16,
    src: u16,
    scratch: &mut Vec<Value>,
) -> Result<()> {
    scratch.clear();
    match op {
        0xfd0f..=0xfd11 => scratch.push(Value::I32(regs[src as usize] as u32 as i32)),
        0xfd12 => scratch.push(Value::I64(regs[src as usize] as i64)),
        0xfd13 => scratch.push(Value::F32(regs[src as usize] as u32)),
        0xfd14 => scratch.push(Value::F64(regs[src as usize])),
        _ => scratch.push(Value::V128(V128(read_v128(regs, src)))),
    }
    let imm = if (0xfd15..=0xfd22).contains(&op) {
        Immediate::Index(u32::from(lane))
    } else {
        Immediate::None
    };
    crate::simd::execute(op, &imm, scratch)?;
    match scratch.pop().ok_or_else(|| trap("simd result"))? {
        Value::V128(v) => write_v128(regs, dst, v.0),
        other => regs[dst as usize] = value_to_bits(other).ok_or_else(|| trap("simd scalar"))?,
    }
    Ok(())
}

#[inline(never)]
fn simd_binary_slow(
    op: u16,
    lane: u8,
    regs: &mut [u64],
    dst: u16,
    a: u16,
    b: u16,
    scratch: &mut Vec<Value>,
) -> Result<()> {
    scratch.clear();
    scratch.push(Value::V128(V128(read_v128(regs, a))));
    match op {
        0xfd6b..=0xfd6d
        | 0xfd8b..=0xfd8d
        | 0xfdab..=0xfdad
        | 0xfdcb..=0xfdcd
        | 0xfd17
        | 0xfd1a
        | 0xfd1c => scratch.push(Value::I32(regs[b as usize] as u32 as i32)),
        0xfd1e => scratch.push(Value::I64(regs[b as usize] as i64)),
        0xfd20 => scratch.push(Value::F32(regs[b as usize] as u32)),
        0xfd22 => scratch.push(Value::F64(regs[b as usize])),
        _ => scratch.push(Value::V128(V128(read_v128(regs, b)))),
    }
    let imm = if (0xfd15..=0xfd22).contains(&op) {
        Immediate::Index(u32::from(lane))
    } else {
        Immediate::None
    };
    crate::simd::execute(op, &imm, scratch)?;
    let Value::V128(v) = scratch.pop().ok_or_else(|| trap("simd result"))? else {
        return Err(trap("simd v128 result"));
    };
    write_v128(regs, dst, v.0);
    Ok(())
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn simd_ternary_slow(
    op: u16,
    regs: &mut [u64],
    dst: u16,
    a: u16,
    b: u16,
    c: u16,
    scratch: &mut Vec<Value>,
) -> Result<()> {
    scratch.clear();
    scratch.push(Value::V128(V128(read_v128(regs, a))));
    scratch.push(Value::V128(V128(read_v128(regs, b))));
    scratch.push(Value::V128(V128(read_v128(regs, c))));
    crate::simd::execute(op, &Immediate::None, scratch)?;
    let Value::V128(v) = scratch.pop().ok_or_else(|| trap("simd result"))? else {
        return Err(trap("simd v128 result"));
    };
    write_v128(regs, dst, v.0);
    Ok(())
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn simd_mem_slow(
    store: &mut Store,
    inst_id: usize,
    mem_idx_in_inst: u32,
    op: u16,
    offset: u64,
    raw_addr: u64,
    in_vec: [u8; 16],
    lane: u8,
) -> Result<(Option<[u8; 16]>, u64)> {
    let h = *store.instances[inst_id]
        .mems
        .get(mem_idx_in_inst as usize)
        .ok_or_else(|| trap("memory index"))?;
    let idx = store.check(h.0, &store.mems)?;
    let mem = &mut store.mems[idx];
    let addr = if mem.ty.index_type == IndexType::I64 {
        raw_addr
    } else {
        raw_addr as u32 as u64
    };
    let eff = addr
        .checked_add(offset)
        .ok_or_else(|| trap("address overflow"))?;
    let sub = op & 0xff;
    let lane_op = (0x54..=0x5b).contains(&sub);
    let is_store = sub == 11 || (0x58..=0x5b).contains(&sub);
    let n = match sub {
        0 | 11 => 16,
        1..=6 => 8,
        7 => 1,
        8 => 2,
        9 | 0x5c => 4,
        10 | 0x5d => 8,
        0x54 | 0x58 => 1,
        0x55 | 0x59 => 2,
        0x56 | 0x5a => 4,
        0x57 | 0x5b => 8,
        _ => return Err(trap("SIMD memory opcode")),
    };
    range(eff, n, mem.len()?)?;
    if is_store {
        if lane_op {
            let start = (lane as usize) * n;
            if start + n > 16 {
                return Err(trap("invalid lane"));
            }
            mem.write(eff, &in_vec[start..start + n])?;
        } else {
            mem.write(eff, &in_vec)?;
        }
        return Ok((None, eff));
    }
    let mut raw_buf = [0u8; 16];
    mem.read_into(eff, &mut raw_buf[..n])?;
    let bytes = &raw_buf[..n];
    let mut out = if lane_op { in_vec } else { [0u8; 16] };
    if lane_op {
        let start = (lane as usize) * n;
        if start + n > 16 {
            return Err(trap("invalid lane"));
        }
        out[start..start + n].copy_from_slice(bytes);
    } else {
        match sub {
            0 => out.copy_from_slice(bytes),
            1..=6 => {
                let width = 1usize << ((sub - 1) / 2);
                let signed = sub % 2 == 1;
                for (i, chunk) in bytes.chunks(width).enumerate() {
                    let mut b = [0u8; 8];
                    b[..width].copy_from_slice(chunk);
                    let mut x = u64::from_le_bytes(b);
                    if signed {
                        x = ((x << (64 - width * 8)) as i64 >> (64 - width * 8)) as u64;
                    }
                    out[i * width * 2..(i + 1) * width * 2]
                        .copy_from_slice(&x.to_le_bytes()[..width * 2]);
                }
            }
            7..=10 => {
                for c in out.chunks_mut(n) {
                    c.copy_from_slice(bytes);
                }
            }
            0x5c | 0x5d => out[..n].copy_from_slice(bytes),
            _ => return Err(trap("SIMD load opcode")),
        }
    }
    Ok((Some(out), eff))
}

#[inline(always)]
fn get_or_compile_ref(module: &Module, body: usize) -> Option<&FastFunction> {
    let slot = module.data.register_fast.get(body)?;
    slot.get_or_init(|| compile_fast(module, body)).as_deref()
}

#[derive(Clone, Copy)]
struct ActiveFrame {
    inst_id: usize,
    body: usize,
    func_handle: crate::Func,
    /// Next instruction pointer to resume when callee returns.
    ip: usize,
    /// Offset in `reg_bank` where this frame's slots begin.
    reg_base: usize,
    /// Relative slot in caller where callee results should be written.
    ret_dst_base: u16,
    /// Logical operand-stack base of this frame (for `max_stack_height` checks).
    logical_base: usize,
    /// BlockMeta for materialization if a nested callee bails.
    meta_after_call: u32,
    mem0: Option<usize>,
}

#[inline(always)]
fn resolve_mem0(store: &Store, inst_id: usize) -> Option<usize> {
    let h = *store.instances[inst_id].mems.first()?;
    let idx = h.0.index;
    let m = store.mems.get(idx)?;
    (m.shared.is_none()).then_some(idx)
}

#[inline(always)]
fn fast_read_bytes<const N: usize, const DENSE: bool>(
    at: usize,
    p0_base: usize,
    p0: &[u8],
    p1_base: usize,
    p1: &[u8],
) -> Option<[u8; N]> {
    if DENSE {
        return p0.get(at..).and_then(|s| s.first_chunk::<N>()).copied();
    }
    let r0 = at.wrapping_sub(p0_base);
    if let Some(chunk) = p0.get(r0..).and_then(|s| s.first_chunk::<N>()) {
        return Some(*chunk);
    }
    let r1 = at.wrapping_sub(p1_base);
    if let Some(chunk) = p1.get(r1..).and_then(|s| s.first_chunk::<N>()) {
        return Some(*chunk);
    }
    None
}

#[inline(always)]
fn fast_read_u8<const DENSE: bool>(
    at: usize,
    p0_base: usize,
    p0: &[u8],
    p1_base: usize,
    p1: &[u8],
) -> Option<u8> {
    if DENSE {
        return p0.get(at).copied();
    }
    if let Some(&b) = p0.get(at.wrapping_sub(p0_base)) {
        return Some(b);
    }
    if let Some(&b) = p1.get(at.wrapping_sub(p1_base)) {
        return Some(b);
    }
    None
}

#[inline(always)]
fn fast_write_bytes<const N: usize, const DENSE: bool>(
    at: usize,
    data: [u8; N],
    p0_base: usize,
    p0: &mut [u8],
    p1_base: usize,
    p1: &mut [u8],
) -> bool {
    if DENSE {
        if let Some(chunk) = p0.get_mut(at..).and_then(|s| s.first_chunk_mut::<N>()) {
            *chunk = data;
            return true;
        }
        return false;
    }
    let r0 = at.wrapping_sub(p0_base);
    if let Some(chunk) = p0.get_mut(r0..).and_then(|s| s.first_chunk_mut::<N>()) {
        *chunk = data;
        return true;
    }
    let r1 = at.wrapping_sub(p1_base);
    if let Some(chunk) = p1.get_mut(r1..).and_then(|s| s.first_chunk_mut::<N>()) {
        *chunk = data;
        return true;
    }
    false
}

#[inline(always)]
fn fast_write_u8<const DENSE: bool>(
    at: usize,
    val: u8,
    p0_base: usize,
    p0: &mut [u8],
    p1_base: usize,
    p1: &mut [u8],
) -> bool {
    if DENSE {
        if let Some(slot) = p0.get_mut(at) {
            *slot = val;
            return true;
        }
        return false;
    }
    if let Some(slot) = p0.get_mut(at.wrapping_sub(p0_base)) {
        *slot = val;
        return true;
    }
    if let Some(slot) = p1.get_mut(at.wrapping_sub(p1_base)) {
        *slot = val;
        return true;
    }
    false
}

#[inline(never)]
fn mem_load_slow(
    store: &mut Store,
    inst_id: usize,
    mem_idx_in_inst: u32,
    op: u16,
    offset: u64,
    raw_addr: u64,
) -> Result<(u64, u64)> {
    let h = *store.instances[inst_id]
        .mems
        .get(mem_idx_in_inst as usize)
        .ok_or_else(|| trap("memory index"))?;
    let idx = store.check(h.0, &store.mems)?;
    let mem = &store.mems[idx];
    let addr = if mem.ty.index_type == IndexType::I64 {
        raw_addr
    } else {
        raw_addr as u32 as u64
    };
    let eff = addr
        .checked_add(offset)
        .ok_or_else(|| trap("address overflow"))?;
    let (_, width, _) = mem_sig(op).ok_or_else(|| trap("memory opcode"))?;
    let mut buf = [0u8; 8];
    mem.read_into(eff, &mut buf[..width])?;
    let x = u64::from_le_bytes(buf);
    let val = match op {
        0x2c => x as i8 as i32 as u32 as u64,
        0x2e => x as i16 as i32 as u32 as u64,
        0x30 => x as i8 as i64 as u64,
        0x32 => x as i16 as i64 as u64,
        0x34 => x as i32 as i64 as u64,
        _ => x,
    };
    Ok((val, eff))
}

#[inline(never)]
fn mem_store_slow(
    store: &mut Store,
    inst_id: usize,
    mem_idx_in_inst: u32,
    op: u16,
    offset: u64,
    raw_addr: u64,
    val: u64,
) -> Result<u64> {
    let h = *store.instances[inst_id]
        .mems
        .get(mem_idx_in_inst as usize)
        .ok_or_else(|| trap("memory index"))?;
    let idx = store.check(h.0, &store.mems)?;
    let mem = &mut store.mems[idx];
    let addr = if mem.ty.index_type == IndexType::I64 {
        raw_addr
    } else {
        raw_addr as u32 as u64
    };
    let eff = addr
        .checked_add(offset)
        .ok_or_else(|| trap("address overflow"))?;
    let (_, width, _) = mem_sig(op).ok_or_else(|| trap("memory opcode"))?;
    let _ = range(eff, width, mem.len()?)?;
    mem.write(eff, &val.to_le_bytes()[..width])?;
    Ok(eff)
}

#[inline(never)]
fn mem_copy_slow(
    store: &mut Store,
    inst_id: usize,
    dst_mem: u32,
    src_mem: u32,
    dst: u64,
    src: u64,
    len: u64,
) -> Result<()> {
    let n = usize::try_from(len).map_err(|_| trap("length overflow"))?;
    let dest = store.instances[inst_id].mems[dst_mem as usize].0.index;
    range(dst, n, store.mems[dest].len()?)?;
    let source = store.instances[inst_id].mems[src_mem as usize].0.index;
    range(src, n, store.mems[source].len()?)?;
    let same_unshared = source == dest && store.mems[dest].shared.is_none();
    let mut scratch = Vec::new();
    let mut done = 0;
    while done < n {
        if store.cancellation.is_cancelled() {
            return Err(err(ErrorKind::ResourceLimit, "execution cancelled"));
        }
        let amount = (n - done).min(65536);
        let chunk = if dst > src { n - done - amount } else { done };
        let from = src as usize + chunk;
        if same_unshared {
            store.mems[dest]
                .bytes
                .copy_within(from, dst as usize + chunk, amount, &mut scratch)
                .map_err(allocation_failed)?;
        } else if store.mems[source].shared.is_none() && store.mems[dest].shared.is_none() {
            scratch
                .try_reserve(amount.saturating_sub(scratch.len()))
                .map_err(|_| err(ErrorKind::ResourceLimit, "memory allocation failed"))?;
            scratch.resize(amount, 0);
            store.mems[source].read_into(from as u64, &mut scratch)?;
            store.mems[dest].write(dst + chunk as u64, &scratch)?;
        } else {
            let bytes = store.mems[source].read(from as u64, amount)?;
            store.mems[dest].write(dst + chunk as u64, &bytes)?;
        }
        done += amount;
    }
    Ok(())
}

#[inline(never)]
fn mem_fill_slow(
    store: &mut Store,
    inst_id: usize,
    mem: u32,
    dst: u64,
    val: u64,
    len: u64,
) -> Result<()> {
    const FILL_CHUNK: usize = 65536;
    let n = usize::try_from(len).map_err(|_| trap("length overflow"))?;
    let dest = store.instances[inst_id].mems[mem as usize].0.index;
    range(dst, n, store.mems[dest].len()?)?;
    let unshared = store.mems[dest].shared.is_none();
    let buffer = if unshared {
        Vec::new()
    } else {
        vec![val as u8; n.min(FILL_CHUNK)]
    };
    let mut done = 0;
    while done < n {
        if store.cancellation.is_cancelled() {
            return Err(err(ErrorKind::ResourceLimit, "execution cancelled"));
        }
        let amount = (n - done).min(FILL_CHUNK);
        let at = dst as usize + done;
        if unshared {
            store.mems[dest]
                .bytes
                .fill(at, amount, val as u8)
                .map_err(allocation_failed)?;
        } else {
            store.mems[dest].write(at as u64, &buffer[..amount])?;
        }
        done += amount;
        #[cfg(test)]
        if store
            .bulk_cancel_after
            .is_some_and(|threshold| done >= threshold)
        {
            store.bulk_cancel_after = None;
            store.cancellation.cancel();
        }
    }
    Ok(())
}

#[inline(never)]
fn mem_grow_slow(store: &mut Store, inst_id: usize, mem: u32, delta: u64) -> Result<u64> {
    let remaining_memory = store
        .engine
        .config
        .limits
        .max_store_memory_bytes
        .saturating_sub(store.memory_usage()?);
    let mem_idx = store.instances[inst_id].mems[mem as usize].0.index;
    let m = &mut store.mems[mem_idx];
    let old = (m.len()? as u64) >> m.ty.page_size_log2;
    let wide = m.ty.index_type == IndexType::I64;
    let delta = if wide { delta } else { delta as u32 as u64 };
    let mut result = old;
    let new = old.checked_add(delta);
    if let Some(n) = new.filter(|n| {
        *n <= m.ty.limits.max.unwrap_or(u64::MAX)
            && *n <= m.ty.addressable_pages()
            && *n <= store.engine.config.limits.max_memory_pages
    }) {
        if let Some(bytes) = n
            .checked_mul(1u64.checked_shl(m.ty.page_size_log2 as u32).unwrap_or(0))
            .filter(|n| {
                *n <= store.engine.config.limits.max_memory_bytes
                    && n.saturating_sub(old << m.ty.page_size_log2) <= remaining_memory
            })
            .and_then(|n| usize::try_from(n).ok())
        {
            if let Some(shared) = &m.shared {
                if !shared.grow(bytes)? {
                    result = u64::MAX;
                }
            } else if !m.grow_unshared(bytes) {
                result = u64::MAX;
            }
        } else {
            result = u64::MAX;
        }
    } else {
        result = u64::MAX;
    }
    if result != u64::MAX {
        m.ty.limits.min = (m.len()? as u64) >> m.ty.page_size_log2;
    }
    Ok(if wide { result } else { result as u32 as u64 })
}

/// Materializes all active fast-register frames into canonical `frames` + `stack` at `cur_meta`.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn materialize_all(
    store: &mut Store,
    frames: &mut Vec<Frame>,
    stack: &mut Vec<Value>,
    call_stack: &[ActiveFrame],
    cur_inst: usize,
    cur_body: usize,
    cur_handle: crate::Func,
    cur_reg_base: usize,
    cur_meta_idx: u32,
    reg_bank: &[u64],
) -> Result<()> {
    let entry_frame = frames
        .last_mut()
        .ok_or_else(|| trap("missing entry frame"))?;
    let entry_base = entry_frame.base;
    stack.truncate(entry_base);

    let fill_labels = |store: &mut Store,
                       target_labels: &mut Vec<Label>,
                       func: &FastFunction,
                       meta: &BlockMeta,
                       base: usize|
     -> Result<()> {
        let old = target_labels.len();
        target_labels.clear();
        store.release_labels(old);
        let l_start = meta.labels_start as usize;
        let l_end = l_start + meta.labels_len as usize;
        for sl in &func.static_labels[l_start..l_end] {
            store.push_label(
                target_labels,
                Label {
                    try_table_pc: None,
                    start: sl.start as usize,
                    end: sl.end as usize,
                    height: base + sl.height as usize,
                    params: sl.params as usize,
                    results: sl.results as usize,
                    loop_: sl.loop_,
                },
            )?;
        }
        Ok(())
    };

    let push_operands =
        |stack: &mut Vec<Value>, func: &FastFunction, meta: &BlockMeta, slots: &[u64]| {
            let s_start = meta.stack_start as usize;
            let s_end = s_start + meta.stack_len as usize;
            let mut off = func.local_reg_slots as usize;
            for &ty in &func.stack_types[s_start..s_end] {
                stack.push(slots_to_value(ty, slots, off));
                off += ty.slots() as usize;
            }
        };

    let push_locals = |dst: &mut Vec<Value>, func: &FastFunction, slots: &[u64]| {
        let mut off = 0usize;
        for &ty in &func.local_types {
            dst.push(slots_to_value(ty, slots, off));
            off += ty.slots() as usize;
        }
    };

    let cur_func = Arc::clone(
        store.instances[cur_inst].module.data.register_fast[cur_body]
            .get()
            .and_then(|o| o.as_ref())
            .ok_or_else(|| trap("missing compiled register function"))?,
    );

    if call_stack.is_empty() {
        let meta = &cur_func.blocks[cur_meta_idx as usize];
        let slots = &reg_bank[cur_reg_base..cur_reg_base + cur_func.slots as usize];
        entry_frame.function = store.funcs[cur_handle.0.index].clone();
        entry_frame.locals.clear();
        if entry_frame
            .locals
            .try_reserve(cur_func.local_types.len())
            .is_err()
        {
            return Err(err(ErrorKind::ResourceLimit, "frame allocation failed"));
        }
        push_locals(&mut entry_frame.locals, &cur_func, slots);
        entry_frame.pc = meta.source_pc as usize;
        entry_frame.register = None;
        fill_labels(store, &mut entry_frame.labels, &cur_func, meta, entry_base)?;
        push_operands(stack, &cur_func, meta, slots);
        return Ok(());
    }

    // 1. Update the entry frame (call_stack[0]).
    {
        let f0 = &call_stack[0];
        let f0_func = Arc::clone(
            store.instances[f0.inst_id].module.data.register_fast[f0.body]
                .get()
                .and_then(|o| o.as_ref())
                .ok_or_else(|| trap("missing compiled register function"))?,
        );
        let meta = &f0_func.blocks[f0.meta_after_call as usize];
        let slots = &reg_bank[f0.reg_base..f0.reg_base + f0_func.slots as usize];
        entry_frame.function = store.funcs[f0.func_handle.0.index].clone();
        entry_frame.locals.clear();
        if entry_frame
            .locals
            .try_reserve(f0_func.local_types.len())
            .is_err()
        {
            return Err(err(ErrorKind::ResourceLimit, "frame allocation failed"));
        }
        push_locals(&mut entry_frame.locals, &f0_func, slots);
        entry_frame.pc = meta.source_pc as usize;
        entry_frame.register = None;
        fill_labels(store, &mut entry_frame.labels, &f0_func, meta, entry_base)?;
        push_operands(stack, &f0_func, meta, slots);
    }

    // 2. Push intermediate nested frames (call_stack[1..]).
    for af in &call_stack[1..] {
        let af_func = Arc::clone(
            store.instances[af.inst_id].module.data.register_fast[af.body]
                .get()
                .and_then(|o| o.as_ref())
                .ok_or_else(|| trap("missing compiled register function"))?,
        );
        let meta = &af_func.blocks[af.meta_after_call as usize];
        let slots = &reg_bank[af.reg_base..af.reg_base + af_func.slots as usize];
        let base = stack.len();
        let mut locals = Vec::with_capacity(af_func.num_locals as usize);
        push_locals(&mut locals, &af_func, slots);
        let mut labels = Vec::new();
        fill_labels(store, &mut labels, &af_func, meta, base)?;
        push_operands(stack, &af_func, meta, slots);
        let fun = store.funcs[af.func_handle.0.index].clone();
        frames.push(Frame {
            register: None,
            #[cfg(feature = "compact-executor")]
            compact: false,
            function: fun,
            pc: meta.source_pc as usize,
            locals,
            base,
            labels,
        });
    }

    // 3. Push the innermost (currently executing) frame at `cur_meta_idx`.
    {
        let meta = &cur_func.blocks[cur_meta_idx as usize];
        let slots = &reg_bank[cur_reg_base..cur_reg_base + cur_func.slots as usize];
        let base = stack.len();
        let mut locals = Vec::with_capacity(cur_func.num_locals as usize);
        push_locals(&mut locals, &cur_func, slots);
        let mut labels = Vec::new();
        fill_labels(store, &mut labels, &cur_func, meta, base)?;
        push_operands(stack, &cur_func, meta, slots);
        let fun = store.funcs[cur_handle.0.index].clone();
        frames.push(Frame {
            register: None,
            #[cfg(feature = "compact-executor")]
            compact: false,
            function: fun,
            pc: meta.source_pc as usize,
            locals,
            base,
            labels,
        });
    }

    Ok(())
}

/// Executes the frame at the top of `frames` on the lazy untyped-`u64` register tier.
///
/// Precondition: the caller has already charged 1 source fuel for `frames.last().pc` (`== 0`),
/// checked cancellation, and verified entry limits for the frame.
pub(crate) fn run(
    store: &mut Store,
    frames: &mut Vec<Frame>,
    stack: &mut Vec<Value>,
    fuel: &mut Option<u64>,
) -> Result<FastOutcome> {
    let entry_frame = frames.last().ok_or_else(|| trap("missing frame"))?;
    if entry_frame.pc != 0 || !entry_frame.labels.is_empty() {
        return Ok(FastOutcome::Declined);
    }
    let entry_inst = entry_frame.function.instance;
    let entry_body = entry_frame.function.body;
    let entry_base = entry_frame.base;
    let entry_handle = {
        let inst = &store.instances[entry_inst];
        let decoded = inst.module.data.inner.decoded();
        let num_import_funcs = decoded
            .imports
            .iter()
            .filter(|i| matches!(i.kind, ImportKind::Func(_)))
            .count();
        inst.funcs[num_import_funcs + entry_body]
    };

    // Honor opt-in entry threshold if configured: let cold entries run on Decoded
    // until the threshold is reached.
    if let Some(threshold) = store.engine.register_entry_threshold {
        let count = store.instances[entry_inst].module.data.heat[entry_body]
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        if count < threshold {
            return Ok(FastOutcome::Declined);
        }
    }

    let (entry_slots, entry_stack_peak, entry_max_nesting, entry_fuel_blocks, first_op) = {
        let Some(entry_func) = get_or_compile_ref(&store.instances[entry_inst].module, entry_body)
        else {
            return Ok(FastOutcome::Declined);
        };
        (
            entry_func.slots as usize,
            entry_func.stack_peak,
            entry_func.max_nesting,
            entry_func.fuel_blocks,
            entry_func.ops.first().copied(),
        )
    };
    if fuel.is_some() && !entry_fuel_blocks {
        return Ok(FastOutcome::Declined);
    }

    let max_stack_height = store.engine.config.limits.max_stack_height;
    let max_call_depth = store.engine.config.limits.max_call_depth;
    if entry_base
        .checked_add(entry_stack_peak)
        .is_none_or(|h| h > max_stack_height)
    {
        return Ok(FastOutcome::Declined);
    }
    if store.check_label_budget(entry_max_nesting).is_err() {
        return Ok(FastOutcome::Declined);
    }

    // Check first block fuel (recall op 0 was ALREADY charged by `execution.rs`).
    let (start_ip, first_block_fuel) = match first_op {
        Some(RegOp::Block { fuel: bf, .. } | RegOp::CheckBlock { fuel: bf, .. }) => {
            let extra = bf.saturating_sub(1) as u64;
            if let Some(rem) = *fuel
                && rem < extra
            {
                return Ok(FastOutcome::Declined);
            }
            (1usize, bf)
        }
        None | Some(RegOp::Bail { .. }) => {
            // Empty or unsupported opcode at pc=0.
            return Ok(FastOutcome::Declined);
        }
        Some(_) => {
            // Pure bounded leaf compiled without fuel/entry CheckBlock.
            if fuel.is_some() {
                return Ok(FastOutcome::Declined);
            }
            (0usize, 0u16)
        }
    };

    // Populate register bank for the entry frame.
    let mut reg_bank: Vec<u64> = Vec::new();
    if reg_bank.try_reserve_exact(entry_slots).is_err() {
        return Err(err(ErrorKind::ResourceLimit, "frame allocation failed"));
    }
    for &v in &entry_frame.locals {
        if let Value::V128(vec) = v {
            reg_bank.push(u64::from_le_bytes(vec.0[0..8].try_into().unwrap()));
            reg_bank.push(u64::from_le_bytes(vec.0[8..16].try_into().unwrap()));
        } else if let Some(bits) = value_to_bits(v) {
            reg_bank.push(bits);
        } else {
            return Ok(FastOutcome::Declined);
        }
    }
    reg_bank.resize(entry_slots, 0);

    let cancel = store.cancellation.clone();
    let mut local_fuel = *fuel;
    // Charge the remaining ops of block 0 once up front so the hot loop has zero
    // `first_entry_block` checks on every basic block!
    if let Some(rem) = &mut local_fuel {
        *rem -= first_block_fuel.saturating_sub(1) as u64;
    }
    let initial_active_local_slots = store.active_local_slots;
    let res = if local_fuel.is_some() {
        run_loop::<true>(
            store,
            frames,
            stack,
            &mut local_fuel,
            &cancel.0,
            entry_inst,
            entry_body,
            entry_handle,
            entry_base,
            start_ip,
            first_block_fuel,
            &mut reg_bank,
            max_stack_height,
            max_call_depth,
        )
    } else {
        run_loop::<false>(
            store,
            frames,
            stack,
            &mut local_fuel,
            &cancel.0,
            entry_inst,
            entry_body,
            entry_handle,
            entry_base,
            start_ip,
            first_block_fuel,
            &mut reg_bank,
            max_stack_height,
            max_call_depth,
        )
    };
    if res.is_err() {
        store.active_local_slots = initial_active_local_slots;
    }
    *fuel = local_fuel;
    store.current_fuel = local_fuel;
    res
}

#[derive(Clone, Copy)]
enum SlowLoadKind {
    Assign,
    AddU32,
    AddF64,
}

#[derive(Clone, Copy)]
struct ExecCursors {
    inst: usize,
    body: usize,
    handle: crate::Func,
    reg_base: usize,
    logical_base: usize,
    mem0: Option<usize>,
    ip: usize,
    block_fuel: u16,
    active_local_slots: usize,
}

enum InnerExit {
    CrossInstanceCall {
        callee_handle: crate::Func,
        callee_inst: usize,
        callee_body: usize,
        arg_base: u16,
        arg_slots: u16,
        logical_arg_start: u16,
        meta_at_call: u32,
        meta_after_call: u32,
        src_off: u8,
    },
    CrossInstanceTailCall {
        callee_handle: crate::Func,
        callee_inst: usize,
        callee_body: usize,
        arg_base: u16,
        arg_slots: u16,
        src_off: u8,
    },
    SwitchInst,
    ReturnTop {
        base: u16,
        count: u16,
    },
    Bail {
        meta: u32,
    },
    BailAtCall {
        meta: u32,
    },
    SlowLoad {
        op: u16,
        mem: u32,
        offset: u64,
        raw: u64,
        dst: u16,
        kind: SlowLoadKind,
        src_off: u8,
    },
    SlowStore {
        op: u16,
        mem: u32,
        offset: u64,
        raw: u64,
        val: u64,
        src_off: u8,
    },
    SlowSimdMem {
        op: u16,
        mem: u32,
        offset: u64,
        raw: u64,
        dst: u16,
        in_vec: [u8; 16],
        lane: u8,
        src_off: u8,
    },
    MemSize {
        dst: u16,
        mem: u32,
    },
    MemGrow {
        dst: u16,
        delta: u64,
        mem: u32,
    },
    MemCopy {
        dst_mem: u32,
        src_mem: u32,
        dst: u64,
        src: u64,
        len: u64,
        src_off: u8,
    },
    MemFill {
        mem: u32,
        dst: u64,
        val: u64,
        len: u64,
        src_off: u8,
    },
    MemInit {
        mem: u32,
        seg: u32,
        dst: u64,
        src: u64,
        len: u64,
        src_off: u8,
    },
    DataDrop {
        seg: u32,
    },
    TableInit {
        table: u32,
        seg: u32,
        dst: u64,
        src: u64,
        len: u64,
        src_off: u8,
    },
    ElemDrop {
        seg: u32,
    },
    TableCopy {
        dst_table: u32,
        src_table: u32,
        dst: u64,
        src: u64,
        len: u64,
        src_off: u8,
    },
    TableSize {
        dst: u16,
        table: u32,
    },
}

/// Inner register dispatch loop specialized on `FUEL` and `DENSE` memory backing.
/// When `DENSE == true`, `p0_slice` spans the entire contiguous linear memory 0 (`p0_base == 0`,
/// `p1_slice` dead), freeing registers and eliminating all 2-page LRU checks and misses.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn dispatch_inner<const FUEL: bool, const DENSE: bool>(
    store: &mut Store,
    cursors: &mut ExecCursors,
    call_stack: &mut Vec<ActiveFrame>,
    reg_bank: &mut Vec<u64>,
    simd_scratch: &mut Vec<Value>,
    fuel: &mut Option<u64>,
    cancel: &std::sync::atomic::AtomicBool,
    cache_p0_idx: usize,
    cache_p1_idx: usize,
    frames_len: usize,
    store_id: u64,
    max_local_slots_limit: usize,
    max_labels_limit: usize,
    max_stack_height: usize,
    max_call_depth: usize,
) -> Result<InnerExit> {
    let mut cur_inst = cursors.inst;
    let mut cur_body = cursors.body;
    let mut cur_handle = cursors.handle;
    let mut cur_reg_base = cursors.reg_base;
    let mut cur_logical_base = cursors.logical_base;
    let mut cur_mem0 = cursors.mem0;
    let mut ip = cursors.ip;
    let mut cur_block_fuel = cursors.block_fuel;
    let mut active_local_slots = cursors.active_local_slots;
    let active_labels = store.active_labels;
    #[cfg(test)]
    let has_bulk_cancel_hook = store.bulk_cancel_after.is_some();

    macro_rules! refund_on_trap {
        ($src_off:expr) => {{
            if FUEL && let Some(rem) = fuel {
                let unexec = cur_block_fuel.saturating_sub(($src_off as u16) + 1) as u64;
                *rem = rem.saturating_add(unexec);
            }
        }};
    }

    let exit = {
        let instances = &store.instances;
        let funcs = &store.funcs;
        let tabs = &store.tabs;
        let globals = &mut store.globals;
        let canonical_lock = &store.engine.canonical;
        let max_type_work = store.engine.config.limits.max_type_work;
        let (p0_base, p0_slice, p1_base, p1_slice): (usize, &mut [u8], usize, &mut [u8]) = if DENSE
        {
            match cur_mem0.and_then(|i| store.mems[i].bytes.dense_slice_mut()) {
                Some(slice) => (0, slice, 0, &mut []),
                None => (0, &mut [], 0, &mut []),
            }
        } else {
            match cur_mem0 {
                Some(i) => {
                    let (s0, s1) = store.mems[i]
                        .bytes
                        .allocated_two_pages_mut(cache_p0_idx, cache_p1_idx);
                    (cache_p0_idx << PAGE_BITS, s0, cache_p1_idx << PAGE_BITS, s1)
                }
                None => (0, &mut [], 0, &mut []),
            }
        };
        let cur_inst_ref = &instances[cur_inst];
        let inst_globals = &cur_inst_ref.globals;
        let inst_funcs = &cur_inst_ref.funcs;
        let inst_tabs = &cur_inst_ref.tabs;
        let inst_canonical = &cur_inst_ref.module.data.canonical;
        let inst_fast = &cur_inst_ref.module.data.register_fast;
        let mut cur_func: &FastFunction = inst_fast[cur_body]
            .get()
            .and_then(|o| o.as_deref())
            .unwrap();
        let mut ops: &[RegOp] = &cur_func.ops;
        let mut consts: &[u64] = &cur_func.consts;
        let mut mem0_64 = cur_func.mem0_is_64;
        let mut regs: &mut [u64] = &mut reg_bank[cur_reg_base..];
        let mut remaining_fuel = if FUEL { fuel.unwrap_or(0) } else { 0 };

        macro_rules! inline_call_target {
            ($callee_handle:expr, $arg_base:expr, $arg_slots:expr, $logical_arg_start:expr, $meta_at_call:expr, $meta_after_call:expr, $src_off:expr) => {{
                let callee_handle = $callee_handle;
                let Some(callee_fn) = funcs
                    .get_live(callee_handle.0.index, callee_handle.0.generation)
                    .filter(|_| callee_handle.0.store == store_id)
                else {
                    if FUEL {
                        *fuel = Some(remaining_fuel);
                    }
                    refund_on_trap!($src_off);
                    return Err(err(ErrorKind::Host, "foreign or stale handle"));
                };
                let callee_inst = callee_fn.instance;
                let callee_body = callee_fn.body;
                if callee_fn.host.is_none()
                    && let Some(callee_fast) =
                        get_or_compile_ref(&instances[callee_inst].module, callee_body)
                {
                    if frames_len + call_stack.len() >= max_call_depth {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!($src_off);
                        return Err(err(ErrorKind::ResourceLimit, "call depth exhausted"));
                    }
                    let nloc = callee_fast.num_locals as usize;
                    if active_local_slots
                        .checked_add(nloc)
                        .is_none_or(|n| n > max_local_slots_limit)
                    {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!($src_off);
                        return Err(err(ErrorKind::ResourceLimit, "active local slots limit"));
                    }
                    let next_logical_base = cur_logical_base + $logical_arg_start as usize;
                    let stack_ok = next_logical_base
                        .checked_add(callee_fast.stack_peak)
                        .is_some_and(|h| h <= max_stack_height);
                    let labels_ok = active_labels
                        .checked_add(callee_fast.max_nesting)
                        .is_some_and(|n| n <= max_labels_limit);
                    if stack_ok && labels_ok {
                        if callee_inst == cur_inst {
                            let new_reg_base = cur_reg_base + cur_func.slots as usize;
                            let needed = new_reg_base + callee_fast.slots as usize;
                            if needed > reg_bank.len() {
                                if reg_bank.try_reserve(needed - reg_bank.len()).is_err() {
                                    if FUEL {
                                        *fuel = Some(remaining_fuel);
                                    }
                                    refund_on_trap!($src_off);
                                    return Err(err(
                                        ErrorKind::ResourceLimit,
                                        "frame allocation failed",
                                    ));
                                }
                                reg_bank.resize(needed, 0);
                            }
                            let arg_src = cur_reg_base + $arg_base as usize;
                            let arg_slots = $arg_slots as usize;
                            let nloc_reg = callee_fast.local_reg_slots as usize;
                            if arg_slots > 0 {
                                reg_bank.copy_within(arg_src..arg_src + arg_slots, new_reg_base);
                            }
                            if nloc_reg > arg_slots {
                                reg_bank[new_reg_base + arg_slots..new_reg_base + nloc_reg].fill(0);
                            }
                            active_local_slots += nloc;
                            call_stack.push(ActiveFrame {
                                inst_id: cur_inst,
                                body: cur_body,
                                func_handle: cur_handle,
                                ip,
                                reg_base: cur_reg_base,
                                ret_dst_base: $arg_base,
                                logical_base: cur_logical_base,
                                meta_after_call: $meta_after_call,
                                mem0: cur_mem0,
                            });
                            cur_body = callee_body;
                            cur_handle = callee_handle;
                            cur_func = callee_fast;
                            cur_reg_base = new_reg_base;
                            cur_logical_base = next_logical_base;
                            ops = &cur_func.ops;
                            consts = &cur_func.consts;
                            mem0_64 = cur_func.mem0_is_64;
                            regs = &mut reg_bank[cur_reg_base..];
                            ip = 0;
                            continue;
                        } else {
                            if FUEL {
                                *fuel = Some(remaining_fuel);
                            }
                            break InnerExit::CrossInstanceCall {
                                callee_handle,
                                callee_inst,
                                callee_body,
                                arg_base: $arg_base,
                                arg_slots: $arg_slots,
                                logical_arg_start: $logical_arg_start,
                                meta_at_call: $meta_at_call,
                                meta_after_call: $meta_after_call,
                                src_off: $src_off,
                            };
                        }
                    }
                }
                if FUEL {
                    *fuel = Some(remaining_fuel);
                }
                break InnerExit::BailAtCall {
                    meta: $meta_at_call,
                };
            }};
        }

        macro_rules! inline_tail_call_target {
            ($callee_handle:expr, $arg_base:expr, $arg_slots:expr, $meta_at_call:expr, $src_off:expr) => {{
                let callee_handle = $callee_handle;
                let Some(callee_fn) = funcs
                    .get_live(callee_handle.0.index, callee_handle.0.generation)
                    .filter(|_| callee_handle.0.store == store_id)
                else {
                    if FUEL {
                        *fuel = Some(remaining_fuel);
                    }
                    refund_on_trap!($src_off);
                    return Err(err(ErrorKind::Host, "foreign or stale handle"));
                };
                let callee_inst = callee_fn.instance;
                let callee_body = callee_fn.body;
                if callee_fn.host.is_none()
                    && let Some(callee_fast) =
                        get_or_compile_ref(&instances[callee_inst].module, callee_body)
                {
                    let old_nloc = cur_func.num_locals as usize;
                    let new_nloc = callee_fast.num_locals as usize;
                    let Some(next_active_locals) = active_local_slots
                        .saturating_sub(old_nloc)
                        .checked_add(new_nloc)
                        .filter(|&n| n <= max_local_slots_limit)
                    else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!($src_off);
                        return Err(err(ErrorKind::ResourceLimit, "active local slots limit"));
                    };
                    let stack_ok = cur_logical_base
                        .checked_add(callee_fast.stack_peak)
                        .is_some_and(|h| h <= max_stack_height);
                    let labels_ok = active_labels
                        .checked_add(callee_fast.max_nesting)
                        .is_some_and(|n| n <= max_labels_limit);
                    if stack_ok && labels_ok {
                        if callee_inst == cur_inst {
                            let needed = cur_reg_base + callee_fast.slots as usize;
                            if needed > reg_bank.len() {
                                if reg_bank.try_reserve(needed - reg_bank.len()).is_err() {
                                    if FUEL {
                                        *fuel = Some(remaining_fuel);
                                    }
                                    refund_on_trap!($src_off);
                                    return Err(err(
                                        ErrorKind::ResourceLimit,
                                        "frame allocation failed",
                                    ));
                                }
                                reg_bank.resize(needed, 0);
                            }
                            let arg_src = cur_reg_base + $arg_base as usize;
                            let arg_slots = $arg_slots as usize;
                            let new_nloc_reg = callee_fast.local_reg_slots as usize;
                            if arg_slots > 0 && arg_src != cur_reg_base {
                                reg_bank.copy_within(arg_src..arg_src + arg_slots, cur_reg_base);
                            }
                            if new_nloc_reg > arg_slots {
                                reg_bank[cur_reg_base + arg_slots..cur_reg_base + new_nloc_reg]
                                    .fill(0);
                            }
                            active_local_slots = next_active_locals;
                            cur_body = callee_body;
                            cur_handle = callee_handle;
                            cur_func = callee_fast;
                            ops = &cur_func.ops;
                            consts = &cur_func.consts;
                            mem0_64 = cur_func.mem0_is_64;
                            regs = &mut reg_bank[cur_reg_base..];
                            ip = 0;
                            continue;
                        } else {
                            if FUEL {
                                *fuel = Some(remaining_fuel);
                            }
                            break InnerExit::CrossInstanceTailCall {
                                callee_handle,
                                callee_inst,
                                callee_body,
                                arg_base: $arg_base,
                                arg_slots: $arg_slots,
                                src_off: $src_off,
                            };
                        }
                    }
                }
                if FUEL {
                    *fuel = Some(remaining_fuel);
                }
                break InnerExit::BailAtCall {
                    meta: $meta_at_call,
                };
            }};
        }

        loop {
            let op = ops[ip];
            ip += 1;
            match op {
                RegOp::Block { meta, fuel: bf } => {
                    if FUEL {
                        let need = bf as u64;
                        if remaining_fuel < need {
                            *fuel = Some(remaining_fuel);
                            break InnerExit::Bail { meta };
                        }
                        remaining_fuel -= need;
                        cur_block_fuel = bf;
                    }
                }
                RegOp::CheckBlock { meta, fuel: bf } => {
                    if cancel.load(Ordering::Relaxed) {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        return Err(err(ErrorKind::ResourceLimit, "execution cancelled"));
                    }
                    if FUEL {
                        let need = bf as u64;
                        if remaining_fuel < need {
                            *fuel = Some(remaining_fuel);
                            break InnerExit::Bail { meta };
                        }
                        remaining_fuel -= need;
                        cur_block_fuel = bf;
                    }
                }
                RegOp::Mov { dst, src } => {
                    regs[dst as usize] = regs[src as usize];
                }
                RegOp::Const32 { dst, val } => {
                    regs[dst as usize] = val as u64;
                }
                RegOp::Const64 { dst, idx } => {
                    regs[dst as usize] = consts[idx as usize];
                }

                // ---- i32 3-address ALU ----
                RegOp::I32Eqz { dst, src } => {
                    regs[dst as usize] = u64::from(regs[src as usize] as u32 == 0);
                }
                RegOp::I32Add { dst, a, b } => {
                    regs[dst as usize] =
                        (regs[a as usize] as u32).wrapping_add(regs[b as usize] as u32) as u64;
                }
                RegOp::I32Sub { dst, a, b } => {
                    regs[dst as usize] =
                        (regs[a as usize] as u32).wrapping_sub(regs[b as usize] as u32) as u64;
                }
                RegOp::I32Mul { dst, a, b } => {
                    regs[dst as usize] =
                        (regs[a as usize] as u32).wrapping_mul(regs[b as usize] as u32) as u64;
                }
                RegOp::I32DivS { dst, a, b, src_off } => {
                    let va = regs[a as usize] as i32;
                    let vb = regs[b as usize] as i32;
                    if vb == 0 {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!(src_off);
                        return Err(trap("integer divide by zero"));
                    }
                    let Some(q) = va.checked_div(vb) else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!(src_off);
                        return Err(trap("integer overflow"));
                    };
                    regs[dst as usize] = q as u32 as u64;
                }
                RegOp::I32DivU { dst, a, b, src_off } => {
                    let vb = regs[b as usize] as u32;
                    if vb == 0 {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!(src_off);
                        return Err(trap("integer divide by zero"));
                    }
                    regs[dst as usize] = ((regs[a as usize] as u32) / vb) as u64;
                }
                RegOp::I32RemS { dst, a, b, src_off } => {
                    let vb = regs[b as usize] as i32;
                    if vb == 0 {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!(src_off);
                        return Err(trap("integer divide by zero"));
                    }
                    regs[dst as usize] = (regs[a as usize] as i32).wrapping_rem(vb) as u32 as u64;
                }
                RegOp::I32RemU { dst, a, b, src_off } => {
                    let vb = regs[b as usize] as u32;
                    if vb == 0 {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!(src_off);
                        return Err(trap("integer divide by zero"));
                    }
                    regs[dst as usize] = ((regs[a as usize] as u32) % vb) as u64;
                }
                RegOp::I32And { dst, a, b } => {
                    regs[dst as usize] =
                        ((regs[a as usize] as u32) & (regs[b as usize] as u32)) as u64;
                }
                RegOp::I32Or { dst, a, b } => {
                    regs[dst as usize] =
                        ((regs[a as usize] as u32) | (regs[b as usize] as u32)) as u64;
                }
                RegOp::I32Xor { dst, a, b } => {
                    regs[dst as usize] =
                        ((regs[a as usize] as u32) ^ (regs[b as usize] as u32)) as u64;
                }
                RegOp::I32Shl { dst, a, b } => {
                    regs[dst as usize] =
                        (regs[a as usize] as u32).wrapping_shl(regs[b as usize] as u32) as u64;
                }
                RegOp::I32ShrS { dst, a, b } => {
                    regs[dst as usize] = ((regs[a as usize] as i32)
                        .wrapping_shr(regs[b as usize] as u32))
                        as u32 as u64;
                }
                RegOp::I32ShrU { dst, a, b } => {
                    regs[dst as usize] =
                        (regs[a as usize] as u32).wrapping_shr(regs[b as usize] as u32) as u64;
                }
                RegOp::I32Rotl { dst, a, b } => {
                    regs[dst as usize] =
                        (regs[a as usize] as u32).rotate_left(regs[b as usize] as u32) as u64;
                }
                RegOp::I32Rotr { dst, a, b } => {
                    regs[dst as usize] =
                        (regs[a as usize] as u32).rotate_right(regs[b as usize] as u32) as u64;
                }
                RegOp::I32Cmp { cmp, dst, a, b } => {
                    regs[dst as usize] = u64::from(eval_i32_cmp(
                        cmp,
                        regs[a as usize] as u32,
                        regs[b as usize] as u32,
                    ));
                }
                RegOp::I32CmpImm { cmp, dst, a, imm } => {
                    regs[dst as usize] =
                        u64::from(eval_i32_cmp(cmp, regs[a as usize] as u32, imm as u32));
                }

                // ---- i32 immediate superinstructions ----
                RegOp::I32AddImm { dst, src, imm } => {
                    regs[dst as usize] =
                        (regs[src as usize] as u32).wrapping_add(imm as u32) as u64;
                }
                RegOp::I32SubImm { dst, src, imm } => {
                    regs[dst as usize] =
                        (regs[src as usize] as u32).wrapping_sub(imm as u32) as u64;
                }
                RegOp::I32MulImm { dst, src, imm } => {
                    regs[dst as usize] =
                        (regs[src as usize] as u32).wrapping_mul(imm as u32) as u64;
                }
                RegOp::I32DivUConst { dst, src, cidx } => {
                    regs[dst as usize] = (((regs[src as usize] as u32 as u128)
                        * (consts[cidx as usize] as u128))
                        >> 64) as u64;
                }
                RegOp::I32RemUImm { dst, src, imm } => {
                    regs[dst as usize] = ((regs[src as usize] as u32) % imm) as u64;
                }
                RegOp::I32AndImm { dst, src, imm } => {
                    regs[dst as usize] = ((regs[src as usize] as u32) & imm) as u64;
                }
                RegOp::I32OrImm { dst, src, imm } => {
                    regs[dst as usize] = ((regs[src as usize] as u32) | imm) as u64;
                }
                RegOp::I32XorImm { dst, src, imm } => {
                    regs[dst as usize] = ((regs[src as usize] as u32) ^ imm) as u64;
                }
                RegOp::I32ShlImm { dst, src, shift } => {
                    regs[dst as usize] = (regs[src as usize] as u32).wrapping_shl(shift) as u64;
                }
                RegOp::I32ShrSImm { dst, src, shift } => {
                    regs[dst as usize] =
                        ((regs[src as usize] as i32).wrapping_shr(shift)) as u32 as u64;
                }
                RegOp::I32ShrUImm { dst, src, shift } => {
                    regs[dst as usize] = (regs[src as usize] as u32).wrapping_shr(shift) as u64;
                }
                RegOp::I32RotlImm { dst, src, shift } => {
                    regs[dst as usize] = (regs[src as usize] as u32).rotate_left(shift) as u64;
                }
                RegOp::I32RotrImm { dst, src, shift } => {
                    regs[dst as usize] = (regs[src as usize] as u32).rotate_right(shift) as u64;
                }

                // ---- i64 3-address ALU & conversions ----
                RegOp::I64Add { dst, a, b } => {
                    regs[dst as usize] = regs[a as usize].wrapping_add(regs[b as usize]);
                }
                RegOp::I64Sub { dst, a, b } => {
                    regs[dst as usize] = regs[a as usize].wrapping_sub(regs[b as usize]);
                }
                RegOp::I64Mul { dst, a, b } => {
                    regs[dst as usize] = regs[a as usize].wrapping_mul(regs[b as usize]);
                }
                RegOp::I64And { dst, a, b } => {
                    regs[dst as usize] = regs[a as usize] & regs[b as usize];
                }
                RegOp::I64Or { dst, a, b } => {
                    regs[dst as usize] = regs[a as usize] | regs[b as usize];
                }
                RegOp::I64Xor { dst, a, b } => {
                    regs[dst as usize] = regs[a as usize] ^ regs[b as usize];
                }
                RegOp::I64Shl { dst, a, b } => {
                    regs[dst as usize] = regs[a as usize].wrapping_shl(regs[b as usize] as u32);
                }
                RegOp::I64ShrS { dst, a, b } => {
                    regs[dst as usize] =
                        ((regs[a as usize] as i64).wrapping_shr(regs[b as usize] as u32)) as u64;
                }
                RegOp::I64ShrU { dst, a, b } => {
                    regs[dst as usize] = regs[a as usize].wrapping_shr(regs[b as usize] as u32);
                }
                RegOp::I64AddImm { dst, src, imm } => {
                    regs[dst as usize] = regs[src as usize].wrapping_add(imm as i64 as u64);
                }
                RegOp::I64Cmp { cmp, dst, a, b } => {
                    regs[dst as usize] =
                        u64::from(eval_i64_cmp(cmp, regs[a as usize], regs[b as usize]));
                }
                RegOp::I32WrapI64 { dst, src } => {
                    regs[dst as usize] = regs[src as usize] as u32 as u64;
                }
                RegOp::I64ExtendI32S { dst, src } => {
                    regs[dst as usize] = regs[src as usize] as u32 as i32 as i64 as u64;
                }

                // ---- f32 / f64 3-address ALU ----
                RegOp::F32Add { dst, a, b } => {
                    let r = f32::from_bits(regs[a as usize] as u32)
                        + f32::from_bits(regs[b as usize] as u32);
                    regs[dst as usize] = r.to_bits() as u64;
                }
                RegOp::F32Sub { dst, a, b } => {
                    let r = f32::from_bits(regs[a as usize] as u32)
                        - f32::from_bits(regs[b as usize] as u32);
                    regs[dst as usize] = r.to_bits() as u64;
                }
                RegOp::F32Mul { dst, a, b } => {
                    let r = f32::from_bits(regs[a as usize] as u32)
                        * f32::from_bits(regs[b as usize] as u32);
                    regs[dst as usize] = r.to_bits() as u64;
                }
                RegOp::F32Div { dst, a, b } => {
                    let r = f32::from_bits(regs[a as usize] as u32)
                        / f32::from_bits(regs[b as usize] as u32);
                    regs[dst as usize] = r.to_bits() as u64;
                }
                RegOp::F64Add { dst, a, b } => {
                    let r = f64::from_bits(regs[a as usize]) + f64::from_bits(regs[b as usize]);
                    regs[dst as usize] = r.to_bits();
                }
                RegOp::F64Sub { dst, a, b } => {
                    let r = f64::from_bits(regs[a as usize]) - f64::from_bits(regs[b as usize]);
                    regs[dst as usize] = r.to_bits();
                }
                RegOp::F64Mul { dst, a, b } => {
                    let r = f64::from_bits(regs[a as usize]) * f64::from_bits(regs[b as usize]);
                    regs[dst as usize] = r.to_bits();
                }
                RegOp::F64Div { dst, a, b } => {
                    let r = f64::from_bits(regs[a as usize]) / f64::from_bits(regs[b as usize]);
                    regs[dst as usize] = r.to_bits();
                }
                RegOp::F64Min { dst, a, b } => {
                    regs[dst as usize] = wasm_f64_min(
                        f64::from_bits(regs[a as usize]),
                        f64::from_bits(regs[b as usize]),
                    );
                }
                RegOp::F64Max { dst, a, b } => {
                    regs[dst as usize] = wasm_f64_max(
                        f64::from_bits(regs[a as usize]),
                        f64::from_bits(regs[b as usize]),
                    );
                }
                RegOp::F64Cmp { cmp, dst, a, b } => {
                    regs[dst as usize] = u64::from(eval_f64_cmp(
                        cmp,
                        f64::from_bits(regs[a as usize]),
                        f64::from_bits(regs[b as usize]),
                    ));
                }
                RegOp::F64AddConst { dst, src, cidx } => {
                    let r =
                        f64::from_bits(regs[src as usize]) + f64::from_bits(consts[cidx as usize]);
                    regs[dst as usize] = r.to_bits();
                }
                RegOp::F64SubConst { dst, src, cidx } => {
                    let r =
                        f64::from_bits(regs[src as usize]) - f64::from_bits(consts[cidx as usize]);
                    regs[dst as usize] = r.to_bits();
                }
                RegOp::F64MulConst { dst, src, cidx } => {
                    let r =
                        f64::from_bits(regs[src as usize]) * f64::from_bits(consts[cidx as usize]);
                    regs[dst as usize] = r.to_bits();
                }
                RegOp::F64MinConst { dst, src, cidx } => {
                    regs[dst as usize] = wasm_f64_min(
                        f64::from_bits(regs[src as usize]),
                        f64::from_bits(consts[cidx as usize]),
                    );
                }
                RegOp::F64MaxConst { dst, src, cidx } => {
                    regs[dst as usize] = wasm_f64_max(
                        f64::from_bits(regs[src as usize]),
                        f64::from_bits(consts[cidx as usize]),
                    );
                }

                // ---- Generic scalar unary/binary ----
                RegOp::Unary {
                    op: uop,
                    dst,
                    src,
                    src_off,
                } => {
                    let a = regs[src as usize];
                    let res = if integer::supported(uop) {
                        integer::unary(uop, a)
                    } else {
                        float::unary(uop, a)
                    };
                    match res {
                        Ok(v) => regs[dst as usize] = v,
                        Err(e) => {
                            if FUEL {
                                *fuel = Some(remaining_fuel);
                            }
                            refund_on_trap!(src_off);
                            return Err(e);
                        }
                    }
                }
                RegOp::Binary {
                    op: bop,
                    dst,
                    a,
                    b,
                    src_off,
                } => {
                    let va = regs[a as usize];
                    let vb = regs[b as usize];
                    let res = if integer::supported(bop) {
                        integer::binary(bop, va, vb)
                    } else {
                        float::binary(bop, va, vb)
                    };
                    match res {
                        Ok(v) => regs[dst as usize] = v,
                        Err(e) => {
                            if FUEL {
                                *fuel = Some(remaining_fuel);
                            }
                            refund_on_trap!(src_off);
                            return Err(e);
                        }
                    }
                }

                RegOp::Select { dst, a, b, cond } => {
                    regs[dst as usize] = if regs[cond as usize] as u32 != 0 {
                        regs[a as usize]
                    } else {
                        regs[b as usize]
                    };
                }

                // ---- Scalar globals (disjoint borrow!) ----
                RegOp::GlobalGet { dst, global } => {
                    let gh = inst_globals[global as usize];
                    let val = globals[gh.0.index].value;
                    regs[dst as usize] = value_to_bits(val).ok_or_else(|| trap("global type"))?;
                }
                RegOp::GlobalSet { src, global } => {
                    let gh = inst_globals[global as usize];
                    let slot = &mut globals[gh.0.index];
                    let v = valtype_to_value(slot.ty.value, regs[src as usize])
                        .ok_or_else(|| trap("global type"))?;
                    slot.value = v;
                }

                // ---- Fast memory-0 loads & stores (cached 2-page &mut [u8] slices!) ----
                RegOp::LoadI32 {
                    dst,
                    addr,
                    offset,
                    src_off,
                } => {
                    let raw = regs[addr as usize];
                    let at = if !mem0_64 {
                        Some((raw as u32 as usize) + (offset as usize))
                    } else {
                        raw.checked_add(offset as u64)
                            .and_then(|e| usize::try_from(e).ok())
                    };
                    if let Some(at) = at
                        && let Some(arr) =
                            fast_read_bytes::<4, DENSE>(at, p0_base, p0_slice, p1_base, p1_slice)
                    {
                        regs[dst as usize] = u32::from_le_bytes(arr) as u64;
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x28,
                            mem: 0,
                            offset: offset as u64,
                            raw,
                            dst,
                            kind: SlowLoadKind::Assign,
                            src_off,
                        };
                    }
                }
                RegOp::LoadI64 {
                    dst,
                    addr,
                    offset,
                    src_off,
                } => {
                    let raw = regs[addr as usize];
                    let at = if !mem0_64 {
                        Some((raw as u32 as usize) + (offset as usize))
                    } else {
                        raw.checked_add(offset as u64)
                            .and_then(|e| usize::try_from(e).ok())
                    };
                    if let Some(at) = at
                        && let Some(arr) =
                            fast_read_bytes::<8, DENSE>(at, p0_base, p0_slice, p1_base, p1_slice)
                    {
                        regs[dst as usize] = u64::from_le_bytes(arr);
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x29,
                            mem: 0,
                            offset: offset as u64,
                            raw,
                            dst,
                            kind: SlowLoadKind::Assign,
                            src_off,
                        };
                    }
                }
                RegOp::LoadI32U8 {
                    dst,
                    addr,
                    offset,
                    src_off,
                } => {
                    let raw = regs[addr as usize];
                    let at = if !mem0_64 {
                        Some((raw as u32 as usize) + (offset as usize))
                    } else {
                        raw.checked_add(offset as u64)
                            .and_then(|e| usize::try_from(e).ok())
                    };
                    if let Some(at) = at
                        && let Some(b) =
                            fast_read_u8::<DENSE>(at, p0_base, p0_slice, p1_base, p1_slice)
                    {
                        regs[dst as usize] = b as u64;
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x2d,
                            mem: 0,
                            offset: offset as u64,
                            raw,
                            dst,
                            kind: SlowLoadKind::Assign,
                            src_off,
                        };
                    }
                }
                RegOp::LoadI32U16 {
                    dst,
                    addr,
                    offset,
                    src_off,
                } => {
                    let raw = regs[addr as usize];
                    let at = if !mem0_64 {
                        Some((raw as u32 as usize) + (offset as usize))
                    } else {
                        raw.checked_add(offset as u64)
                            .and_then(|e| usize::try_from(e).ok())
                    };
                    if let Some(at) = at
                        && let Some(arr) =
                            fast_read_bytes::<2, DENSE>(at, p0_base, p0_slice, p1_base, p1_slice)
                    {
                        regs[dst as usize] = u16::from_le_bytes(arr) as u64;
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x2f,
                            mem: 0,
                            offset: offset as u64,
                            raw,
                            dst,
                            kind: SlowLoadKind::Assign,
                            src_off,
                        };
                    }
                }
                RegOp::LoadU8Add {
                    acc,
                    addr,
                    offset,
                    src_off,
                } => {
                    let raw = regs[addr as usize];
                    let at = if !mem0_64 {
                        Some((raw as u32 as usize) + (offset as usize))
                    } else {
                        raw.checked_add(offset as u64)
                            .and_then(|e| usize::try_from(e).ok())
                    };
                    if let Some(at) = at
                        && let Some(b) =
                            fast_read_u8::<DENSE>(at, p0_base, p0_slice, p1_base, p1_slice)
                    {
                        regs[acc as usize] =
                            (regs[acc as usize] as u32).wrapping_add(b as u32) as u64;
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x2d,
                            mem: 0,
                            offset: offset as u64,
                            raw,
                            dst: acc,
                            kind: SlowLoadKind::AddU32,
                            src_off,
                        };
                    }
                }
                RegOp::LoadF64Add {
                    acc,
                    addr,
                    offset,
                    src_off,
                } => {
                    let raw = regs[addr as usize];
                    let at = if !mem0_64 {
                        Some((raw as u32 as usize) + (offset as usize))
                    } else {
                        raw.checked_add(offset as u64)
                            .and_then(|e| usize::try_from(e).ok())
                    };
                    if let Some(at) = at
                        && let Some(arr) =
                            fast_read_bytes::<8, DENSE>(at, p0_base, p0_slice, p1_base, p1_slice)
                    {
                        let v = f64::from_bits(u64::from_le_bytes(arr));
                        let cur = f64::from_bits(regs[acc as usize]);
                        regs[acc as usize] = (cur + v).to_bits();
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x2b,
                            mem: 0,
                            offset: offset as u64,
                            raw,
                            dst: acc,
                            kind: SlowLoadKind::AddF64,
                            src_off,
                        };
                    }
                }
                RegOp::LoadI32AddAddr {
                    dst,
                    a,
                    b,
                    offset,
                    src_off,
                } => {
                    let raw =
                        (regs[a as usize] as u32).wrapping_add(regs[b as usize] as u32) as u64;
                    let at = (raw as usize) + (offset as usize);
                    if let Some(arr) =
                        fast_read_bytes::<4, DENSE>(at, p0_base, p0_slice, p1_base, p1_slice)
                    {
                        regs[dst as usize] = u32::from_le_bytes(arr) as u64;
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x28,
                            mem: 0,
                            offset: offset as u64,
                            raw,
                            dst,
                            kind: SlowLoadKind::Assign,
                            src_off,
                        };
                    }
                }
                RegOp::LoadU8AddAddr {
                    dst,
                    a,
                    b,
                    offset,
                    src_off,
                } => {
                    let raw =
                        (regs[a as usize] as u32).wrapping_add(regs[b as usize] as u32) as u64;
                    let at = (raw as usize) + (offset as usize);
                    if let Some(byte) =
                        fast_read_u8::<DENSE>(at, p0_base, p0_slice, p1_base, p1_slice)
                    {
                        regs[dst as usize] = byte as u64;
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x2d,
                            mem: 0,
                            offset: offset as u64,
                            raw,
                            dst,
                            kind: SlowLoadKind::Assign,
                            src_off,
                        };
                    }
                }
                RegOp::LoadI32AddImm {
                    dst,
                    addr,
                    imm,
                    src_off,
                } => {
                    let raw = (regs[addr as usize] as u32).wrapping_add(imm) as u64;
                    let at = raw as usize;
                    if let Some(arr) =
                        fast_read_bytes::<4, DENSE>(at, p0_base, p0_slice, p1_base, p1_slice)
                    {
                        regs[dst as usize] = u32::from_le_bytes(arr) as u64;
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x28,
                            mem: 0,
                            offset: 0,
                            raw,
                            dst,
                            kind: SlowLoadKind::Assign,
                            src_off,
                        };
                    }
                }
                RegOp::LoadU8AddImm {
                    dst,
                    addr,
                    imm,
                    src_off,
                } => {
                    let raw = (regs[addr as usize] as u32).wrapping_add(imm) as u64;
                    let at = raw as usize;
                    if let Some(byte) =
                        fast_read_u8::<DENSE>(at, p0_base, p0_slice, p1_base, p1_slice)
                    {
                        regs[dst as usize] = byte as u64;
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x2d,
                            mem: 0,
                            offset: 0,
                            raw,
                            dst,
                            kind: SlowLoadKind::Assign,
                            src_off,
                        };
                    }
                }
                RegOp::LoadU8AddImmAcc {
                    acc,
                    addr,
                    imm,
                    src_off,
                } => {
                    let raw = (regs[addr as usize] as u32).wrapping_add(imm) as u64;
                    let at = raw as usize;
                    if let Some(byte) =
                        fast_read_u8::<DENSE>(at, p0_base, p0_slice, p1_base, p1_slice)
                    {
                        regs[acc as usize] =
                            (regs[acc as usize] as u32).wrapping_add(byte as u32) as u64;
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x2d,
                            mem: 0,
                            offset: 0,
                            raw,
                            dst: acc,
                            kind: SlowLoadKind::AddU32,
                            src_off,
                        };
                    }
                }
                RegOp::StoreI32 {
                    addr,
                    val,
                    offset,
                    src_off,
                } => {
                    let raw = regs[addr as usize];
                    let v = regs[val as usize];
                    let at = if !mem0_64 {
                        Some((raw as u32 as usize) + (offset as usize))
                    } else {
                        raw.checked_add(offset as u64)
                            .and_then(|e| usize::try_from(e).ok())
                    };
                    if let Some(at) = at
                        && fast_write_bytes::<4, DENSE>(
                            at,
                            (v as u32).to_le_bytes(),
                            p0_base,
                            p0_slice,
                            p1_base,
                            p1_slice,
                        )
                    {
                        // committed
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowStore {
                            op: 0x36,
                            mem: 0,
                            offset: offset as u64,
                            raw,
                            val: v,
                            src_off,
                        };
                    }
                }
                RegOp::StoreI64 {
                    addr,
                    val,
                    offset,
                    src_off,
                } => {
                    let raw = regs[addr as usize];
                    let v = regs[val as usize];
                    let at = if !mem0_64 {
                        Some((raw as u32 as usize) + (offset as usize))
                    } else {
                        raw.checked_add(offset as u64)
                            .and_then(|e| usize::try_from(e).ok())
                    };
                    if let Some(at) = at
                        && fast_write_bytes::<8, DENSE>(
                            at,
                            v.to_le_bytes(),
                            p0_base,
                            p0_slice,
                            p1_base,
                            p1_slice,
                        )
                    {
                        // committed
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowStore {
                            op: 0x37,
                            mem: 0,
                            offset: offset as u64,
                            raw,
                            val: v,
                            src_off,
                        };
                    }
                }
                RegOp::StoreI32_8 {
                    addr,
                    val,
                    offset,
                    src_off,
                } => {
                    let raw = regs[addr as usize];
                    let v = regs[val as usize];
                    let at = if !mem0_64 {
                        Some((raw as u32 as usize) + (offset as usize))
                    } else {
                        raw.checked_add(offset as u64)
                            .and_then(|e| usize::try_from(e).ok())
                    };
                    if let Some(at) = at
                        && fast_write_u8::<DENSE>(at, v as u8, p0_base, p0_slice, p1_base, p1_slice)
                    {
                        // committed
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowStore {
                            op: 0x3a,
                            mem: 0,
                            offset: offset as u64,
                            raw,
                            val: v,
                            src_off,
                        };
                    }
                }
                RegOp::StoreI32_16 {
                    addr,
                    val,
                    offset,
                    src_off,
                } => {
                    let raw = regs[addr as usize];
                    let v = regs[val as usize];
                    let at = if !mem0_64 {
                        Some((raw as u32 as usize) + (offset as usize))
                    } else {
                        raw.checked_add(offset as u64)
                            .and_then(|e| usize::try_from(e).ok())
                    };
                    if let Some(at) = at
                        && fast_write_bytes::<2, DENSE>(
                            at,
                            (v as u16).to_le_bytes(),
                            p0_base,
                            p0_slice,
                            p1_base,
                            p1_slice,
                        )
                    {
                        // committed
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowStore {
                            op: 0x3b,
                            mem: 0,
                            offset: offset as u64,
                            raw,
                            val: v,
                            src_off,
                        };
                    }
                }
                RegOp::StoreI32AddImm {
                    addr,
                    val,
                    imm,
                    src_off,
                } => {
                    let raw = (regs[addr as usize] as u32).wrapping_add(imm) as u64;
                    let v = regs[val as usize];
                    if fast_write_bytes::<4, DENSE>(
                        raw as usize,
                        (v as u32).to_le_bytes(),
                        p0_base,
                        p0_slice,
                        p1_base,
                        p1_slice,
                    ) {
                        // committed
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowStore {
                            op: 0x36,
                            mem: 0,
                            offset: 0,
                            raw,
                            val: v,
                            src_off,
                        };
                    }
                }
                RegOp::StoreI32_8AddImm {
                    addr,
                    val,
                    imm,
                    src_off,
                } => {
                    let raw = (regs[addr as usize] as u32).wrapping_add(imm) as u64;
                    let v = regs[val as usize];
                    if fast_write_u8::<DENSE>(
                        raw as usize,
                        v as u8,
                        p0_base,
                        p0_slice,
                        p1_base,
                        p1_slice,
                    ) {
                        // committed
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowStore {
                            op: 0x3a,
                            mem: 0,
                            offset: 0,
                            raw,
                            val: v,
                            src_off,
                        };
                    }
                }
                RegOp::MemGen {
                    op: mop,
                    addr,
                    val,
                    arg_idx,
                    src_off,
                } => {
                    let arg = cur_func.mem_args[arg_idx as usize];
                    let raw = regs[addr as usize];
                    if FUEL {
                        *fuel = Some(remaining_fuel);
                    }
                    if mop >= 0x36 {
                        break InnerExit::SlowStore {
                            op: mop,
                            mem: arg.memory,
                            offset: arg.offset,
                            raw,
                            val: regs[val as usize],
                            src_off,
                        };
                    } else {
                        break InnerExit::SlowLoad {
                            op: mop,
                            mem: arg.memory,
                            offset: arg.offset,
                            raw,
                            dst: val,
                            kind: SlowLoadKind::Assign,
                            src_off,
                        };
                    }
                }
                RegOp::MemSize { dst, mem } => {
                    if FUEL {
                        *fuel = Some(remaining_fuel);
                    }
                    break InnerExit::MemSize { dst, mem };
                }
                RegOp::MemGrow {
                    dst,
                    delta,
                    mem,
                    src_off: _,
                } => {
                    let d = regs[delta as usize];
                    if FUEL {
                        *fuel = Some(remaining_fuel);
                    }
                    break InnerExit::MemGrow { dst, delta: d, mem };
                }
                RegOp::MemCopy0 {
                    dst,
                    src,
                    len,
                    src_off,
                } => {
                    let d_raw = regs[dst as usize];
                    let s_raw = regs[src as usize];
                    let n_raw = regs[len as usize];
                    let fast_done = if n_raw <= 4096
                        && let (Ok(d), Ok(s), Ok(n)) = (
                            usize::try_from(d_raw),
                            usize::try_from(s_raw),
                            usize::try_from(n_raw),
                        ) {
                        if DENSE {
                            if d.checked_add(n).is_some_and(|e| e <= p0_slice.len())
                                && s.checked_add(n).is_some_and(|e| e <= p0_slice.len())
                            {
                                p0_slice.copy_within(s..s + n, d);
                                true
                            } else {
                                false
                            }
                        } else {
                            let d0 = d.wrapping_sub(p0_base);
                            let s0 = s.wrapping_sub(p0_base);
                            if d0.checked_add(n).is_some_and(|e| e <= p0_slice.len())
                                && s0.checked_add(n).is_some_and(|e| e <= p0_slice.len())
                            {
                                p0_slice.copy_within(s0..s0 + n, d0);
                                true
                            } else {
                                let d1 = d.wrapping_sub(p1_base);
                                let s1 = s.wrapping_sub(p1_base);
                                if d1.checked_add(n).is_some_and(|e| e <= p1_slice.len())
                                    && s1.checked_add(n).is_some_and(|e| e <= p1_slice.len())
                                {
                                    p1_slice.copy_within(s1..s1 + n, d1);
                                    true
                                } else {
                                    false
                                }
                            }
                        }
                    } else {
                        false
                    };
                    if !fast_done {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::MemCopy {
                            dst_mem: 0,
                            src_mem: 0,
                            dst: d_raw,
                            src: s_raw,
                            len: n_raw,
                            src_off,
                        };
                    }
                }
                RegOp::MemFill0 {
                    dst,
                    val,
                    len,
                    src_off,
                } => {
                    let d_raw = regs[dst as usize];
                    let v_raw = regs[val as usize];
                    let n_raw = regs[len as usize];
                    #[cfg(test)]
                    let allow_fast = !has_bulk_cancel_hook && n_raw <= 4096;
                    #[cfg(not(test))]
                    let allow_fast = n_raw <= 4096;
                    let fast_done = if allow_fast
                        && let (Ok(d), Ok(n)) = (usize::try_from(d_raw), usize::try_from(n_raw))
                    {
                        if DENSE {
                            if let Some(sub) = d.checked_add(n).and_then(|e| p0_slice.get_mut(d..e))
                            {
                                sub.fill(v_raw as u8);
                                true
                            } else {
                                false
                            }
                        } else {
                            let d0 = d.wrapping_sub(p0_base);
                            if let Some(sub) =
                                d0.checked_add(n).and_then(|e| p0_slice.get_mut(d0..e))
                            {
                                sub.fill(v_raw as u8);
                                true
                            } else {
                                let d1 = d.wrapping_sub(p1_base);
                                if let Some(sub) =
                                    d1.checked_add(n).and_then(|e| p1_slice.get_mut(d1..e))
                                {
                                    sub.fill(v_raw as u8);
                                    true
                                } else {
                                    false
                                }
                            }
                        }
                    } else {
                        false
                    };
                    if !fast_done {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::MemFill {
                            mem: 0,
                            dst: d_raw,
                            val: v_raw,
                            len: n_raw,
                            src_off,
                        };
                    }
                }
                RegOp::MemBulkGen {
                    kind,
                    src_off,
                    dst,
                    src_or_val,
                    len,
                    arg_idx,
                } => {
                    let arg = cur_func.bulk_args[arg_idx as usize];
                    if FUEL {
                        *fuel = Some(remaining_fuel);
                    }
                    match kind {
                        0 => {
                            break InnerExit::MemCopy {
                                dst_mem: arg.dst_mem,
                                src_mem: arg.src_mem,
                                dst: regs[dst as usize],
                                src: regs[src_or_val as usize],
                                len: regs[len as usize],
                                src_off,
                            };
                        }
                        1 => {
                            break InnerExit::MemFill {
                                mem: arg.dst_mem,
                                dst: regs[dst as usize],
                                val: regs[src_or_val as usize],
                                len: regs[len as usize],
                                src_off,
                            };
                        }
                        2 => {
                            break InnerExit::MemInit {
                                mem: arg.dst_mem,
                                seg: arg.src_mem,
                                dst: regs[dst as usize],
                                src: regs[src_or_val as usize],
                                len: regs[len as usize],
                                src_off,
                            };
                        }
                        3 => {
                            break InnerExit::DataDrop { seg: arg.dst_mem };
                        }
                        4 => {
                            break InnerExit::TableInit {
                                table: arg.dst_mem,
                                seg: arg.src_mem,
                                dst: regs[dst as usize],
                                src: regs[src_or_val as usize],
                                len: regs[len as usize],
                                src_off,
                            };
                        }
                        5 => {
                            break InnerExit::ElemDrop { seg: arg.dst_mem };
                        }
                        _ => {
                            break InnerExit::TableCopy {
                                dst_table: arg.dst_mem,
                                src_table: arg.src_mem,
                                dst: regs[dst as usize],
                                src: regs[src_or_val as usize],
                                len: regs[len as usize],
                                src_off,
                            };
                        }
                    }
                }
                RegOp::TableSize { dst, table } => {
                    if FUEL {
                        *fuel = Some(remaining_fuel);
                    }
                    break InnerExit::TableSize { dst, table };
                }

                // ---- Superinstructions: compare + branch & counted loop ----
                RegOp::I32CmpBrIf { cmp, a, b, target } => {
                    if eval_i32_cmp(cmp, regs[a as usize] as u32, regs[b as usize] as u32) {
                        ip = target as usize;
                    }
                }
                RegOp::I32CmpImmBrIf {
                    cmp,
                    a,
                    imm,
                    target,
                } => {
                    if eval_i32_cmp(cmp, regs[a as usize] as u32, imm as u32) {
                        ip = target as usize;
                    }
                }
                RegOp::I64CmpBrIf { cmp, a, b, target } => {
                    if eval_i64_cmp(cmp, regs[a as usize], regs[b as usize]) {
                        ip = target as usize;
                    }
                }
                RegOp::I32IncLtUBrIf {
                    reg,
                    limit,
                    step,
                    target,
                } => {
                    let next = (regs[reg as usize] as u32).wrapping_add(step as i32 as u32);
                    regs[reg as usize] = next as u64;
                    if next < regs[limit as usize] as u32 {
                        ip = target as usize;
                    }
                }
                RegOp::I32IncLtSBrIf {
                    reg,
                    limit,
                    step,
                    target,
                } => {
                    let next = (regs[reg as usize] as i32).wrapping_add(i32::from(step));
                    regs[reg as usize] = next as u32 as u64;
                    if next < regs[limit as usize] as i32 {
                        ip = target as usize;
                    }
                }
                RegOp::I32AddImmJump { reg, imm, target } => {
                    regs[reg as usize] =
                        (regs[reg as usize] as u32).wrapping_add(imm as u32) as u64;
                    ip = target as usize;
                }

                // ---- Control flow ----
                RegOp::Jump { target } => {
                    ip = target as usize;
                }
                RegOp::BrIf { cond, target } => {
                    if regs[cond as usize] as u32 != 0 {
                        ip = target as usize;
                    }
                }
                RegOp::BrIfZero { cond, target } => {
                    if regs[cond as usize] as u32 == 0 {
                        ip = target as usize;
                    }
                }
                RegOp::BrTable {
                    selector,
                    table_idx,
                } => {
                    let which = regs[selector as usize] as u32 as usize;
                    let r = cur_func.table_ranges[table_idx as usize].clone();
                    let targets = &cur_func.table_targets[r.start as usize..r.end as usize];
                    let t = targets
                        .get(which)
                        .copied()
                        .unwrap_or_else(|| *targets.last().unwrap());
                    ip = t as usize;
                }
                RegOp::MoveRange {
                    dst_base,
                    src_base,
                    count,
                } => {
                    let s = src_base as usize;
                    let d = dst_base as usize;
                    let c = count as usize;
                    regs.copy_within(s..s + c, d);
                }
                RegOp::Call { call_idx, src_off } => {
                    let site = cur_func.call_sites[call_idx as usize];
                    let callee_handle = inst_funcs[site.func_idx as usize];
                    inline_call_target!(
                        callee_handle,
                        site.arg_base,
                        site.arg_slots,
                        site.logical_arg_start,
                        site.meta_at_call,
                        site.meta_after_call,
                        src_off
                    );
                }
                RegOp::CallIndirect { call_idx, src_off } => {
                    let site = cur_func.call_indirect_sites[call_idx as usize];
                    let raw_ix = regs[site.index_reg as usize];
                    let ix = if site.table_is_64 {
                        raw_ix
                    } else {
                        raw_ix as u32 as u64
                    };
                    let tab_h = inst_tabs[site.table_idx as usize];
                    let tab = &tabs[tab_h.0.index];
                    let Some(&val) = usize::try_from(ix).ok().and_then(|i| tab.values.get(i))
                    else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!(src_off);
                        return Err(trap("table out of bounds"));
                    };
                    let Value::Ref(RefValue::Func(callee_handle)) = val else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!(src_off);
                        return Err(trap("null indirect call"));
                    };
                    let Some(callee_fn) = funcs
                        .get_live(callee_handle.0.index, callee_handle.0.generation)
                        .filter(|_| callee_handle.0.store == store_id)
                    else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!(src_off);
                        return Err(err(ErrorKind::Host, "foreign or stale handle"));
                    };
                    let expected_canonical = inst_canonical[site.type_idx as usize];
                    let compatible = if callee_fn.canonical == expected_canonical {
                        true
                    } else {
                        let Ok(guard) = canonical_lock.lock() else {
                            if FUEL {
                                *fuel = Some(remaining_fuel);
                            }
                            refund_on_trap!(src_off);
                            return Err(err(ErrorKind::Host, "canonical registry poisoned"));
                        };
                        match guard.is_subtype(
                            callee_fn.canonical,
                            expected_canonical,
                            false,
                            max_type_work,
                        ) {
                            Ok(ok) => ok,
                            Err(e) => {
                                if FUEL {
                                    *fuel = Some(remaining_fuel);
                                }
                                refund_on_trap!(src_off);
                                return Err(e);
                            }
                        }
                    };
                    if !compatible {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!(src_off);
                        return Err(trap("indirect call type mismatch"));
                    }
                    inline_call_target!(
                        callee_handle,
                        site.arg_base,
                        site.arg_slots,
                        site.logical_arg_start,
                        site.meta_at_call,
                        site.meta_after_call,
                        src_off
                    );
                }
                RegOp::ReturnCall { call_idx, src_off } => {
                    let site = cur_func.call_sites[call_idx as usize];
                    let callee_handle = inst_funcs[site.func_idx as usize];
                    inline_tail_call_target!(
                        callee_handle,
                        site.arg_base,
                        site.arg_slots,
                        site.meta_at_call,
                        src_off
                    );
                }
                RegOp::ReturnCallIndirect { call_idx, src_off } => {
                    let site = cur_func.call_indirect_sites[call_idx as usize];
                    let raw_ix = regs[site.index_reg as usize];
                    let ix = if site.table_is_64 {
                        raw_ix
                    } else {
                        raw_ix as u32 as u64
                    };
                    let tab_h = inst_tabs[site.table_idx as usize];
                    let tab = &tabs[tab_h.0.index];
                    let Some(&val) = usize::try_from(ix).ok().and_then(|i| tab.values.get(i))
                    else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!(src_off);
                        return Err(trap("table out of bounds"));
                    };
                    let Value::Ref(RefValue::Func(callee_handle)) = val else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!(src_off);
                        return Err(trap("null indirect call"));
                    };
                    let Some(callee_fn) = funcs
                        .get_live(callee_handle.0.index, callee_handle.0.generation)
                        .filter(|_| callee_handle.0.store == store_id)
                    else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!(src_off);
                        return Err(err(ErrorKind::Host, "foreign or stale handle"));
                    };
                    let expected_canonical = inst_canonical[site.type_idx as usize];
                    let compatible = if callee_fn.canonical == expected_canonical {
                        true
                    } else {
                        let Ok(guard) = canonical_lock.lock() else {
                            if FUEL {
                                *fuel = Some(remaining_fuel);
                            }
                            refund_on_trap!(src_off);
                            return Err(err(ErrorKind::Host, "canonical registry poisoned"));
                        };
                        match guard.is_subtype(
                            callee_fn.canonical,
                            expected_canonical,
                            false,
                            max_type_work,
                        ) {
                            Ok(ok) => ok,
                            Err(e) => {
                                if FUEL {
                                    *fuel = Some(remaining_fuel);
                                }
                                refund_on_trap!(src_off);
                                return Err(e);
                            }
                        }
                    };
                    if !compatible {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!(src_off);
                        return Err(trap("indirect call type mismatch"));
                    }
                    inline_tail_call_target!(
                        callee_handle,
                        site.arg_base,
                        site.arg_slots,
                        site.meta_at_call,
                        src_off
                    );
                }
                RegOp::WideArith { kind, base } => {
                    let b = base as usize;
                    let bits = match kind {
                        0 | 1 => {
                            let a = (u128::from(regs[b + 1]) << 64) | u128::from(regs[b]);
                            let rhs = (u128::from(regs[b + 3]) << 64) | u128::from(regs[b + 2]);
                            if kind == 0 {
                                a.wrapping_add(rhs)
                            } else {
                                a.wrapping_sub(rhs)
                            }
                        }
                        2 => ((regs[b] as i64 as i128) * (regs[b + 1] as i64 as i128)) as u128,
                        _ => u128::from(regs[b]) * u128::from(regs[b + 1]),
                    };
                    regs[b] = bits as u64;
                    regs[b + 1] = (bits >> 64) as u64;
                }
                // ---- Native SIMD & Relaxed SIMD ----
                RegOp::MovV128 { dst, src } => {
                    let lo = regs[src as usize];
                    let hi = regs[src as usize + 1];
                    regs[dst as usize] = lo;
                    regs[dst as usize + 1] = hi;
                }
                RegOp::ConstV128 { dst, idx } => {
                    let [lo, hi] = cur_func.v128_consts[idx as usize];
                    regs[dst as usize] = lo;
                    regs[dst as usize + 1] = hi;
                }
                RegOp::SelectV128 { dst, a, b, cond } => {
                    let s = if regs[cond as usize] as u32 != 0 {
                        a
                    } else {
                        b
                    };
                    let lo = regs[s as usize];
                    let hi = regs[s as usize + 1];
                    regs[dst as usize] = lo;
                    regs[dst as usize + 1] = hi;
                }
                RegOp::GlobalGetV128 { dst, global } => {
                    let gh = inst_globals[global as usize];
                    let Value::V128(v) = globals[gh.0.index].value else {
                        return Err(trap("global type"));
                    };
                    write_v128(regs, dst, v.0);
                }
                RegOp::GlobalSetV128 { src, global } => {
                    let gh = inst_globals[global as usize];
                    let slot = &mut globals[gh.0.index];
                    if slot.ty.value != ValType::V128 {
                        return Err(trap("global type"));
                    }
                    slot.value = Value::V128(V128(read_v128(regs, src)));
                }
                RegOp::LoadV128 {
                    dst,
                    addr,
                    offset,
                    src_off,
                } => {
                    let raw = regs[addr as usize];
                    let at = if !mem0_64 {
                        Some((raw as u32 as usize) + (offset as usize))
                    } else {
                        raw.checked_add(offset as u64)
                            .and_then(|e| usize::try_from(e).ok())
                    };
                    if let Some(at) = at
                        && let Some(arr) =
                            fast_read_bytes::<16, DENSE>(at, p0_base, p0_slice, p1_base, p1_slice)
                    {
                        write_v128(regs, dst, arr);
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowSimdMem {
                            op: 0xfd00,
                            mem: 0,
                            offset: offset as u64,
                            raw,
                            dst,
                            in_vec: [0u8; 16],
                            lane: 0,
                            src_off,
                        };
                    }
                }
                RegOp::StoreV128 {
                    addr,
                    val,
                    offset,
                    src_off,
                } => {
                    let raw = regs[addr as usize];
                    let data = read_v128(regs, val);
                    let at = if !mem0_64 {
                        Some((raw as u32 as usize) + (offset as usize))
                    } else {
                        raw.checked_add(offset as u64)
                            .and_then(|e| usize::try_from(e).ok())
                    };
                    if let Some(at) = at
                        && fast_write_bytes::<16, DENSE>(
                            at, data, p0_base, p0_slice, p1_base, p1_slice,
                        )
                    {
                        // committed
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowSimdMem {
                            op: 0xfd0b,
                            mem: 0,
                            offset: offset as u64,
                            raw,
                            dst: val,
                            in_vec: data,
                            lane: 0,
                            src_off,
                        };
                    }
                }
                RegOp::SimdMemGen {
                    op: mop,
                    addr,
                    val,
                    arg_idx,
                    src_off,
                } => {
                    let arg = cur_func.simd_mem_args[arg_idx as usize];
                    let raw = regs[addr as usize];
                    let sub = mop & 0xff;
                    let in_vec = if sub == 11 || (0x54..=0x5b).contains(&sub) {
                        read_v128(regs, arg.src_vec)
                    } else {
                        [0u8; 16]
                    };
                    if FUEL {
                        *fuel = Some(remaining_fuel);
                    }
                    break InnerExit::SlowSimdMem {
                        op: mop,
                        mem: arg.memory,
                        offset: arg.offset,
                        raw,
                        dst: val,
                        in_vec,
                        lane: arg.lane,
                        src_off,
                    };
                }
                RegOp::SimdShuffle {
                    dst,
                    a,
                    b,
                    mask_idx,
                } => {
                    let va = read_v128(regs, a);
                    let vb = read_v128(regs, b);
                    let ix = cur_func.simd_masks[mask_idx as usize];
                    let mut out = [0u8; 16];
                    for i in 0..16 {
                        let m = ix[i] as usize;
                        out[i] = if m < 16 { va[m] } else { vb[m - 16] };
                    }
                    write_v128(regs, dst, out);
                }
                RegOp::SimdUnary {
                    op: sop,
                    dst,
                    src,
                    lane,
                } => match sop {
                    0xfd4d => {
                        let lo = !regs[src as usize];
                        let hi = !regs[src as usize + 1];
                        regs[dst as usize] = lo;
                        regs[dst as usize + 1] = hi;
                    }
                    0xfd0f => {
                        let w = u64::from_ne_bytes([regs[src as usize] as u8; 8]);
                        regs[dst as usize] = w;
                        regs[dst as usize + 1] = w;
                    }
                    0xfd10 => {
                        let b = (regs[src as usize] as u16).to_le_bytes();
                        let w =
                            u64::from_le_bytes([b[0], b[1], b[0], b[1], b[0], b[1], b[0], b[1]]);
                        regs[dst as usize] = w;
                        regs[dst as usize + 1] = w;
                    }
                    0xfd11 | 0xfd13 => {
                        let x = regs[src as usize] as u32 as u64;
                        let w = x | (x << 32);
                        regs[dst as usize] = w;
                        regs[dst as usize + 1] = w;
                    }
                    0xfd12 | 0xfd14 => {
                        let w = regs[src as usize];
                        regs[dst as usize] = w;
                        regs[dst as usize + 1] = w;
                    }
                    0xfd1b | 0xfd1f => {
                        let word = regs[src as usize + (lane as usize >> 1)];
                        regs[dst as usize] = ((word >> ((lane & 1) * 32)) as u32) as u64;
                    }
                    0xfd1d | 0xfd21 => {
                        regs[dst as usize] = regs[src as usize + lane as usize];
                    }
                    _ => simd_unary_slow(sop, lane, regs, dst, src, simd_scratch)?,
                },
                RegOp::SimdBinary {
                    op: sop,
                    dst,
                    a,
                    b,
                    lane,
                } => match sop {
                    0xfd4e => {
                        let lo = regs[a as usize] & regs[b as usize];
                        let hi = regs[a as usize + 1] & regs[b as usize + 1];
                        regs[dst as usize] = lo;
                        regs[dst as usize + 1] = hi;
                    }
                    0xfd4f => {
                        let lo = regs[a as usize] & !regs[b as usize];
                        let hi = regs[a as usize + 1] & !regs[b as usize + 1];
                        regs[dst as usize] = lo;
                        regs[dst as usize + 1] = hi;
                    }
                    0xfd50 => {
                        let lo = regs[a as usize] | regs[b as usize];
                        let hi = regs[a as usize + 1] | regs[b as usize + 1];
                        regs[dst as usize] = lo;
                        regs[dst as usize + 1] = hi;
                    }
                    0xfd51 => {
                        let lo = regs[a as usize] ^ regs[b as usize];
                        let hi = regs[a as usize + 1] ^ regs[b as usize + 1];
                        regs[dst as usize] = lo;
                        regs[dst as usize + 1] = hi;
                    }
                    0xfdae | 0xfdb1 | 0xfdb5 => {
                        let alo = regs[a as usize];
                        let ahi = regs[a as usize + 1];
                        let blo = regs[b as usize];
                        let bhi = regs[b as usize + 1];
                        let lane_op = |x: u32, y: u32| -> u32 {
                            match sop {
                                0xfdae => x.wrapping_add(y),
                                0xfdb1 => x.wrapping_sub(y),
                                _ => x.wrapping_mul(y),
                            }
                        };
                        let r0 = lane_op(alo as u32, blo as u32) as u64;
                        let r1 = lane_op((alo >> 32) as u32, (blo >> 32) as u32) as u64;
                        let r2 = lane_op(ahi as u32, bhi as u32) as u64;
                        let r3 = lane_op((ahi >> 32) as u32, (bhi >> 32) as u32) as u64;
                        regs[dst as usize] = r0 | (r1 << 32);
                        regs[dst as usize + 1] = r2 | (r3 << 32);
                    }
                    0xfde4..=0xfde6 => {
                        let alo = regs[a as usize];
                        let ahi = regs[a as usize + 1];
                        let blo = regs[b as usize];
                        let bhi = regs[b as usize + 1];
                        let lane_op = |x: u32, y: u32| -> u32 {
                            let fx = f32::from_bits(x);
                            let fy = f32::from_bits(y);
                            match sop {
                                0xfde4 => (fx + fy).to_bits(),
                                0xfde5 => (fx - fy).to_bits(),
                                _ => (fx * fy).to_bits(),
                            }
                        };
                        let r0 = lane_op(alo as u32, blo as u32) as u64;
                        let r1 = lane_op((alo >> 32) as u32, (blo >> 32) as u32) as u64;
                        let r2 = lane_op(ahi as u32, bhi as u32) as u64;
                        let r3 = lane_op((ahi >> 32) as u32, (bhi >> 32) as u32) as u64;
                        regs[dst as usize] = r0 | (r1 << 32);
                        regs[dst as usize + 1] = r2 | (r3 << 32);
                    }
                    0xfdf0..=0xfdf2 => {
                        let ax0 = f64::from_bits(regs[a as usize]);
                        let ax1 = f64::from_bits(regs[a as usize + 1]);
                        let bx0 = f64::from_bits(regs[b as usize]);
                        let bx1 = f64::from_bits(regs[b as usize + 1]);
                        let (r0, r1) = match sop {
                            0xfdf0 => (ax0 + bx0, ax1 + bx1),
                            0xfdf1 => (ax0 - bx0, ax1 - bx1),
                            _ => (ax0 * bx0, ax1 * bx1),
                        };
                        regs[dst as usize] = r0.to_bits();
                        regs[dst as usize + 1] = r1.to_bits();
                    }
                    _ => simd_binary_slow(sop, lane, regs, dst, a, b, simd_scratch)?,
                },
                RegOp::SimdTernary {
                    op: sop,
                    dst,
                    a,
                    b,
                    c,
                } => match sop {
                    0xfd52 | 0xfa09..=0xfa0c => {
                        let alo = regs[a as usize];
                        let ahi = regs[a as usize + 1];
                        let blo = regs[b as usize];
                        let bhi = regs[b as usize + 1];
                        let clo = regs[c as usize];
                        let chi = regs[c as usize + 1];
                        regs[dst as usize] = (alo & clo) | (blo & !clo);
                        regs[dst as usize + 1] = (ahi & chi) | (bhi & !chi);
                    }
                    _ => simd_ternary_slow(sop, regs, dst, a, b, c, simd_scratch)?,
                },
                RegOp::Return { base, count } => {
                    if let Some(caller) = call_stack.pop() {
                        let released = cur_func.num_locals as usize;
                        let src_start = cur_reg_base + base as usize;
                        let dst_start = caller.reg_base + caller.ret_dst_base as usize;
                        let c = count as usize;
                        if c == 1 {
                            reg_bank[dst_start] = reg_bank[src_start];
                        } else if c == 2 {
                            let lo = reg_bank[src_start];
                            let hi = reg_bank[src_start + 1];
                            reg_bank[dst_start] = lo;
                            reg_bank[dst_start + 1] = hi;
                        } else if c > 2 {
                            reg_bank.copy_within(src_start..src_start + c, dst_start);
                        }
                        active_local_slots = active_local_slots.saturating_sub(released);
                        cur_body = caller.body;
                        cur_handle = caller.func_handle;
                        cur_reg_base = caller.reg_base;
                        cur_logical_base = caller.logical_base;
                        ip = caller.ip;
                        if caller.inst_id == cur_inst {
                            cur_func = inst_fast[cur_body]
                                .get()
                                .and_then(|o| o.as_deref())
                                .unwrap();
                            ops = &cur_func.ops;
                            consts = &cur_func.consts;
                            mem0_64 = cur_func.mem0_is_64;
                            regs = &mut reg_bank[cur_reg_base..];
                            continue;
                        } else {
                            cur_inst = caller.inst_id;
                            cur_mem0 = caller.mem0;
                            if FUEL {
                                *fuel = Some(remaining_fuel);
                            }
                            break InnerExit::SwitchInst;
                        }
                    }
                    if FUEL {
                        *fuel = Some(remaining_fuel);
                    }
                    break InnerExit::ReturnTop { base, count };
                }
                RegOp::TrapUnreachable { src_off } => {
                    if FUEL {
                        *fuel = Some(remaining_fuel);
                    }
                    refund_on_trap!(src_off);
                    return Err(trap("unreachable"));
                }
                RegOp::Bail { meta } => {
                    if FUEL {
                        *fuel = Some(remaining_fuel);
                    }
                    break InnerExit::Bail { meta };
                }
            }
        }
    };

    cursors.inst = cur_inst;
    cursors.body = cur_body;
    cursors.handle = cur_handle;
    cursors.reg_base = cur_reg_base;
    cursors.logical_base = cur_logical_base;
    cursors.mem0 = cur_mem0;
    cursors.ip = ip;
    cursors.block_fuel = cur_block_fuel;
    cursors.active_local_slots = active_local_slots;
    Ok(exit)
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn run_loop<const FUEL: bool>(
    store: &mut Store,
    frames: &mut Vec<Frame>,
    stack: &mut Vec<Value>,
    fuel: &mut Option<u64>,
    cancel: &std::sync::atomic::AtomicBool,
    mut cur_inst: usize,
    mut cur_body: usize,
    mut cur_handle: crate::Func,
    mut cur_logical_base: usize,
    mut ip: usize,
    mut cur_block_fuel: u16,
    reg_bank: &mut Vec<u64>,
    max_stack_height: usize,
    max_call_depth: usize,
) -> Result<FastOutcome> {
    let mut call_stack: Vec<ActiveFrame> = Vec::new();
    let mut simd_scratch: Vec<Value> = Vec::new();
    let mut cur_reg_base: usize = 0;
    let mut cur_mem0: Option<usize> = resolve_mem0(store, cur_inst);
    if let Some(i) = cur_mem0
        && store.mems[i].bytes.has_written_storage()
    {
        store.mems[i].bytes.try_promote_dense();
    }
    let mut cache_p0_idx: usize = 0;
    let mut cache_p1_idx: usize = 1;
    let frames_len = frames.len();
    let store_id = store.id;
    let max_local_slots_limit = {
        let lim = &store.engine.config.limits;
        let by_bytes = (lim.max_store_memory_bytes / std::mem::size_of::<Value>() as u64)
            .min(usize::MAX as u64) as usize;
        lim.max_total_locals.max(lim.max_locals).min(by_bytes)
    };
    let max_labels_limit = {
        let lim = &store.engine.config.limits;
        let by_bytes = (lim.max_store_memory_bytes / std::mem::size_of::<Label>() as u64)
            .min(usize::MAX as u64) as usize;
        lim.max_total_locals.max(lim.max_locals).min(by_bytes)
    };

    macro_rules! refund_on_trap {
        ($src_off:expr) => {{
            if FUEL && let Some(rem) = fuel {
                let unexec = cur_block_fuel.saturating_sub(($src_off as u16) + 1) as u64;
                *rem = rem.saturating_add(unexec);
            }
        }};
    }

    macro_rules! do_bail {
        ($meta:expr) => {{
            materialize_all(
                store,
                frames,
                stack,
                &call_stack,
                cur_inst,
                cur_body,
                cur_handle,
                cur_reg_base,
                $meta,
                reg_bank,
            )?;
            return Ok(FastOutcome::Bailed);
        }};
    }

    loop {
        let mut cursors = ExecCursors {
            inst: cur_inst,
            body: cur_body,
            handle: cur_handle,
            reg_base: cur_reg_base,
            logical_base: cur_logical_base,
            mem0: cur_mem0,
            ip,
            block_fuel: cur_block_fuel,
            active_local_slots: store.active_local_slots,
        };
        let is_dense = cur_mem0.is_some_and(|i| store.mems[i].bytes.is_dense());
        let exit = if is_dense {
            dispatch_inner::<FUEL, true>(
                store,
                &mut cursors,
                &mut call_stack,
                reg_bank,
                &mut simd_scratch,
                fuel,
                cancel,
                cache_p0_idx,
                cache_p1_idx,
                frames_len,
                store_id,
                max_local_slots_limit,
                max_labels_limit,
                max_stack_height,
                max_call_depth,
            )?
        } else {
            dispatch_inner::<FUEL, false>(
                store,
                &mut cursors,
                &mut call_stack,
                reg_bank,
                &mut simd_scratch,
                fuel,
                cancel,
                cache_p0_idx,
                cache_p1_idx,
                frames_len,
                store_id,
                max_local_slots_limit,
                max_labels_limit,
                max_stack_height,
                max_call_depth,
            )?
        };
        cur_inst = cursors.inst;
        cur_body = cursors.body;
        cur_handle = cursors.handle;
        cur_reg_base = cursors.reg_base;
        cur_logical_base = cursors.logical_base;
        cur_mem0 = cursors.mem0;
        ip = cursors.ip;
        cur_block_fuel = cursors.block_fuel;
        store.active_local_slots = cursors.active_local_slots;

        match exit {
            InnerExit::SwitchInst => {
                cache_p0_idx = 0;
                cache_p1_idx = 1;
                if let Some(i) = cur_mem0
                    && store.mems[i].bytes.has_written_storage()
                {
                    store.mems[i].bytes.try_promote_dense();
                }
            }
            InnerExit::SlowLoad {
                op: lop,
                mem,
                offset,
                raw,
                dst,
                kind,
                src_off,
            } => match mem_load_slow(store, cur_inst, mem, lop, offset, raw) {
                Ok((v, eff)) => {
                    let slot = &mut reg_bank[cur_reg_base + dst as usize];
                    match kind {
                        SlowLoadKind::Assign => *slot = v,
                        SlowLoadKind::AddU32 => {
                            *slot = (*slot as u32).wrapping_add(v as u32) as u64;
                        }
                        SlowLoadKind::AddF64 => {
                            *slot = (f64::from_bits(*slot) + f64::from_bits(v)).to_bits();
                        }
                    }
                    if mem == 0 {
                        if let Some(i) = cur_mem0 {
                            store.mems[i].bytes.try_promote_dense();
                        }
                        if let Ok(p) = usize::try_from(eff >> PAGE_BITS)
                            && p != cache_p0_idx
                        {
                            cache_p1_idx = cache_p0_idx;
                            cache_p0_idx = p;
                        }
                    }
                }
                Err(e) => {
                    refund_on_trap!(src_off);
                    return Err(e);
                }
            },
            InnerExit::SlowStore {
                op: sop,
                mem,
                offset,
                raw,
                val,
                src_off,
            } => match mem_store_slow(store, cur_inst, mem, sop, offset, raw, val) {
                Ok(eff) => {
                    if mem == 0 {
                        if let Some(i) = cur_mem0 {
                            store.mems[i].bytes.try_promote_dense();
                        }
                        if let Ok(p) = usize::try_from(eff >> PAGE_BITS)
                            && p != cache_p0_idx
                        {
                            cache_p1_idx = cache_p0_idx;
                            cache_p0_idx = p;
                        }
                    }
                }
                Err(e) => {
                    refund_on_trap!(src_off);
                    return Err(e);
                }
            },
            InnerExit::SlowSimdMem {
                op: sop,
                mem,
                offset,
                raw,
                dst,
                in_vec,
                lane,
                src_off,
            } => match simd_mem_slow(store, cur_inst, mem, sop, offset, raw, in_vec, lane) {
                Ok((res_vec, eff)) => {
                    if let Some(v) = res_vec {
                        write_v128(&mut reg_bank[cur_reg_base..], dst, v);
                    }
                    if mem == 0 {
                        if let Some(i) = cur_mem0 {
                            store.mems[i].bytes.try_promote_dense();
                        }
                        if let Ok(p) = usize::try_from(eff >> PAGE_BITS)
                            && p != cache_p0_idx
                        {
                            cache_p1_idx = cache_p0_idx;
                            cache_p0_idx = p;
                        }
                    }
                }
                Err(e) => {
                    refund_on_trap!(src_off);
                    return Err(e);
                }
            },
            InnerExit::MemSize { dst, mem } => {
                let h = store.instances[cur_inst].mems[mem as usize];
                let m = &store.mems[h.0.index];
                let pages = (m.bytes.len() >> m.ty.page_size_log2) as u64;
                reg_bank[cur_reg_base + dst as usize] = pages;
            }
            InnerExit::MemGrow { dst, delta, mem } => {
                let res = mem_grow_slow(store, cur_inst, mem, delta)?;
                reg_bank[cur_reg_base + dst as usize] = res;
            }
            InnerExit::MemCopy {
                dst_mem,
                src_mem,
                dst,
                src,
                len,
                src_off,
            } => {
                if let Err(e) = mem_copy_slow(store, cur_inst, dst_mem, src_mem, dst, src, len) {
                    refund_on_trap!(src_off);
                    return Err(e);
                }
                if (dst_mem == 0 || src_mem == 0)
                    && let Some(i) = cur_mem0
                {
                    store.mems[i].bytes.try_promote_dense();
                }
            }
            InnerExit::MemFill {
                mem,
                dst,
                val,
                len,
                src_off,
            } => {
                if let Err(e) = mem_fill_slow(store, cur_inst, mem, dst, val, len) {
                    refund_on_trap!(src_off);
                    return Err(e);
                }
                if mem == 0
                    && let Some(i) = cur_mem0
                {
                    store.mems[i].bytes.try_promote_dense();
                }
            }
            InnerExit::MemInit {
                mem,
                seg,
                dst,
                src,
                len,
                src_off,
            } => {
                let res: Result<()> = (|| {
                    let n = usize::try_from(len).map_err(|_| trap("length overflow"))?;
                    let dest = store.instances[cur_inst].mems[mem as usize].0.index;
                    range(dst, n, store.mems[dest].len()?)?;
                    let data = store.instances[cur_inst]
                        .data_segment(seg as usize)
                        .unwrap_or(&[]);
                    let r = range(src, n, data.len())?;
                    store.mems[dest].write(dst, &data[r])?;
                    Ok(())
                })();
                if let Err(e) = res {
                    refund_on_trap!(src_off);
                    return Err(e);
                }
                if mem == 0
                    && let Some(i) = cur_mem0
                {
                    store.mems[i].bytes.try_promote_dense();
                }
            }
            InnerExit::DataDrop { seg } => {
                store.instances[cur_inst].data[seg as usize] = false;
            }
            InnerExit::TableInit {
                table,
                seg,
                dst,
                src,
                len,
                src_off,
            } => {
                let res: Result<()> = (|| {
                    let n = usize::try_from(len).map_err(|_| trap("length overflow"))?;
                    let target = store.instances[cur_inst].tabs[table as usize].0.index;
                    let out = crate::table_range(dst, n, store.tabs[target].values.len())?;
                    let values = store.instances[cur_inst].elems[seg as usize]
                        .as_deref()
                        .unwrap_or(&[]);
                    let r = crate::table_range(src, n, values.len())?;
                    store.tabs[target].values[out].copy_from_slice(&values[r]);
                    Ok(())
                })();
                if let Err(e) = res {
                    refund_on_trap!(src_off);
                    return Err(e);
                }
            }
            InnerExit::ElemDrop { seg } => {
                store.instances[cur_inst].elems[seg as usize] = None;
            }
            InnerExit::TableCopy {
                dst_table,
                src_table,
                dst,
                src,
                len,
                src_off,
            } => {
                let res: Result<()> = (|| {
                    let n = usize::try_from(len).map_err(|_| trap("length overflow"))?;
                    let target = store.instances[cur_inst].tabs[dst_table as usize].0.index;
                    crate::table_range(dst, n, store.tabs[target].values.len())?;
                    let source = store.instances[cur_inst].tabs[src_table as usize].0.index;
                    crate::table_range(src, n, store.tabs[source].values.len())?;
                    if target == source {
                        store.tabs[target]
                            .values
                            .copy_within(src as usize..src as usize + n, dst as usize);
                    } else {
                        let mut done = 0usize;
                        while done < n {
                            if store.cancellation.is_cancelled() {
                                return Err(err(ErrorKind::ResourceLimit, "execution cancelled"));
                            }
                            let count = (n - done).min(4096);
                            let vals = store.tabs[source].values
                                [src as usize + done..src as usize + done + count]
                                .to_vec();
                            store.tabs[target].values
                                [dst as usize + done..dst as usize + done + count]
                                .copy_from_slice(&vals);
                            done += count;
                        }
                    }
                    Ok(())
                })();
                if let Err(e) = res {
                    refund_on_trap!(src_off);
                    return Err(e);
                }
            }
            InnerExit::TableSize { dst, table } => {
                let t = &store.tabs[store.instances[cur_inst].tabs[table as usize].0.index];
                reg_bank[cur_reg_base + dst as usize] = t.values.len() as u64;
            }
            InnerExit::Bail { meta } => {
                do_bail!(meta);
            }
            InnerExit::BailAtCall { meta } => {
                // Host call, non-fast callee, or tight stack/label limit:
                // refund the 1 fuel charged for `call`/`call_indirect` and bail cleanly!
                if FUEL && let Some(rem) = fuel {
                    *rem = rem.saturating_add(1);
                }
                do_bail!(meta);
            }
            InnerExit::CrossInstanceCall {
                callee_handle,
                callee_inst,
                callee_body,
                arg_base,
                arg_slots,
                logical_arg_start,
                meta_at_call: _,
                meta_after_call,
                src_off,
            } => {
                let cur_slots = store.instances[cur_inst].module.data.register_fast[cur_body]
                    .get()
                    .and_then(|o| o.as_deref())
                    .unwrap()
                    .slots as usize;
                let (callee_slots, callee_num_locals, callee_local_reg_slots) = {
                    let f = store.instances[callee_inst].module.data.register_fast[callee_body]
                        .get()
                        .and_then(|o| o.as_deref())
                        .unwrap();
                    (
                        f.slots as usize,
                        f.num_locals as usize,
                        f.local_reg_slots as usize,
                    )
                };
                let new_reg_base = cur_reg_base + cur_slots;
                let needed = new_reg_base + callee_slots;
                if needed > reg_bank.len() {
                    if reg_bank.try_reserve(needed - reg_bank.len()).is_err() {
                        refund_on_trap!(src_off);
                        return Err(err(ErrorKind::ResourceLimit, "frame allocation failed"));
                    }
                    reg_bank.resize(needed, 0);
                }
                let arg_src = cur_reg_base + arg_base as usize;
                let arg_slots_usize = arg_slots as usize;
                if arg_slots_usize > 0 {
                    reg_bank.copy_within(arg_src..arg_src + arg_slots_usize, new_reg_base);
                }
                if callee_local_reg_slots > arg_slots_usize {
                    reg_bank[new_reg_base + arg_slots_usize..new_reg_base + callee_local_reg_slots]
                        .fill(0);
                }
                store.acquire_local_slots(callee_num_locals);
                let next_logical_base = cur_logical_base + logical_arg_start as usize;
                let callee_mem0 = resolve_mem0(store, callee_inst);
                call_stack.push(ActiveFrame {
                    inst_id: cur_inst,
                    body: cur_body,
                    func_handle: cur_handle,
                    ip,
                    reg_base: cur_reg_base,
                    ret_dst_base: arg_base,
                    logical_base: cur_logical_base,
                    meta_after_call,
                    mem0: cur_mem0,
                });
                cur_inst = callee_inst;
                cur_body = callee_body;
                cur_handle = callee_handle;
                cur_reg_base = new_reg_base;
                cur_logical_base = next_logical_base;
                cur_mem0 = callee_mem0;
                if let Some(i) = cur_mem0
                    && store.mems[i].bytes.has_written_storage()
                {
                    store.mems[i].bytes.try_promote_dense();
                }
                cache_p0_idx = 0;
                cache_p1_idx = 1;
                ip = 0;
            }
            InnerExit::CrossInstanceTailCall {
                callee_handle,
                callee_inst,
                callee_body,
                arg_base,
                arg_slots,
                src_off,
            } => {
                let cur_num_locals = store.instances[cur_inst].module.data.register_fast[cur_body]
                    .get()
                    .and_then(|o| o.as_deref())
                    .unwrap()
                    .num_locals as usize;
                let (callee_slots, callee_num_locals, callee_local_reg_slots) = {
                    let f = store.instances[callee_inst].module.data.register_fast[callee_body]
                        .get()
                        .and_then(|o| o.as_deref())
                        .unwrap();
                    (
                        f.slots as usize,
                        f.num_locals as usize,
                        f.local_reg_slots as usize,
                    )
                };
                let needed = cur_reg_base + callee_slots;
                if needed > reg_bank.len() {
                    if reg_bank.try_reserve(needed - reg_bank.len()).is_err() {
                        refund_on_trap!(src_off);
                        return Err(err(ErrorKind::ResourceLimit, "frame allocation failed"));
                    }
                    reg_bank.resize(needed, 0);
                }
                let arg_src = cur_reg_base + arg_base as usize;
                let arg_slots_usize = arg_slots as usize;
                if arg_slots_usize > 0 && arg_src != cur_reg_base {
                    reg_bank.copy_within(arg_src..arg_src + arg_slots_usize, cur_reg_base);
                }
                if callee_local_reg_slots > arg_slots_usize {
                    reg_bank[cur_reg_base + arg_slots_usize..cur_reg_base + callee_local_reg_slots]
                        .fill(0);
                }
                store.release_local_slots(cur_num_locals);
                store.acquire_local_slots(callee_num_locals);
                cur_inst = callee_inst;
                cur_body = callee_body;
                cur_handle = callee_handle;
                cur_mem0 = resolve_mem0(store, callee_inst);
                if let Some(i) = cur_mem0
                    && store.mems[i].bytes.has_written_storage()
                {
                    store.mems[i].bytes.try_promote_dense();
                }
                cache_p0_idx = 0;
                cache_p1_idx = 1;
                ip = 0;
            }
            InnerExit::ReturnTop { base, count: _ } => {
                let entry_frame = frames.last().unwrap();
                let entry_base = entry_frame.base;
                let entry_locals = entry_frame.locals.len();
                stack.truncate(entry_base);
                let regs = &reg_bank[cur_reg_base..];
                let cur_func = store.instances[cur_inst].module.data.register_fast[cur_body]
                    .get()
                    .and_then(|o| o.as_deref())
                    .unwrap();
                let cur_num_locals = cur_func.num_locals as usize;
                let mut off = base as usize;
                for &ty in &cur_func.result_types {
                    stack.push(slots_to_value(ty, regs, off));
                    off += ty.slots() as usize;
                }
                // Reconcile `active_local_slots` to `entry_frame.locals.len()` (which `execution.rs`
                // releases on `FastOutcome::Returned`) in case a tail call replaced the entry frame.
                store.release_local_slots(cur_num_locals);
                store.acquire_local_slots(entry_locals);
                return Ok(FastOutcome::Returned);
            }
        }
    }
}

/// Checks whether `body` of `module` is admitted and compiled by the lazy fast register tier.
#[cfg(test)]
pub(crate) fn is_admitted(module: &Module, body: usize) -> bool {
    get_or_compile_ref(module, body).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Engine, ExecutionPolicy, External, FeatureSet, Linker};

    fn leb_u32(out: &mut Vec<u8>, mut v: u32) {
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            out.push(b);
            if v == 0 {
                break;
            }
        }
    }

    fn sec(id: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![id];
        leb_u32(&mut out, payload.len() as u32);
        out.extend_from_slice(payload);
        out
    }

    fn body(locals: &[(u32, u8)], code: &[u8]) -> Vec<u8> {
        let mut p = Vec::new();
        leb_u32(&mut p, locals.len() as u32);
        for &(c, t) in locals {
            leb_u32(&mut p, c);
            p.push(t);
        }
        p.extend_from_slice(code);
        p.push(0x0b);
        let mut out = Vec::new();
        leb_u32(&mut out, p.len() as u32);
        out.extend_from_slice(&p);
        out
    }

    fn wasm_mod(secs: &[Vec<u8>]) -> Vec<u8> {
        let mut out = b"\x00asm\x01\x00\x00\x00".to_vec();
        for s in secs {
            out.extend_from_slice(s);
        }
        out
    }

    fn build_ecs_f64_wasm() -> Vec<u8> {
        fn push_f64(out: &mut Vec<u8>, v: f64) {
            out.push(0x44);
            out.extend_from_slice(&v.to_le_bytes());
        }
        // export "prepare"(i32)->() and "run"(i32)->i64
        let types = vec![
            2, 0x60, 1, 0x7f, 0, // 0: (i32)->()
            0x60, 1, 0x7f, 1, 0x7e, // 1: (i32)->i64
        ];
        let funcs = vec![2, 0, 1];
        let mem = vec![1, 0x01, 4, 8];
        let mut exps = vec![3];
        for (name, kind, idx) in [("memory", 2u8, 0u32), ("prepare", 0, 0), ("run", 0, 1)] {
            leb_u32(&mut exps, name.len() as u32);
            exps.extend_from_slice(name.as_bytes());
            exps.push(kind);
            leb_u32(&mut exps, idx);
        }
        // prepare(n): locals 1:i32(i), 2:i32(off)
        let mut bp = Vec::new();
        bp.extend_from_slice(&[0x03, 0x40]); // loop
        bp.extend_from_slice(&[0x20, 1, 0x41, 5, 0x74, 0x22, 2]); // off = i << 5
        bp.extend_from_slice(&[0x20, 1, 0xb7]); // f64(i)
        push_f64(&mut bp, 0.25);
        bp.extend_from_slice(&[0xa2, 0x39, 3, 0]); // mem[off+0] = i*0.25
        bp.extend_from_slice(&[0x20, 2, 0x20, 1, 0xb7]);
        push_f64(&mut bp, -0.125);
        bp.extend_from_slice(&[0xa2, 0x39, 3, 8]); // mem[off+8]
        bp.extend_from_slice(&[0x20, 2]);
        push_f64(&mut bp, 0.5);
        bp.extend_from_slice(&[0x39, 3, 16]); // vx
        bp.extend_from_slice(&[0x20, 2]);
        push_f64(&mut bp, -0.25);
        bp.extend_from_slice(&[0x39, 3, 24]); // vy
        bp.extend_from_slice(&[
            0x20, 1, 0x41, 1, 0x6a, 0x22, 1, 0x20, 0, 0x49, 0x0d, 0, 0x0b,
        ]);

        // run(n): locals 1:i32(i), 2:i32(off), 3:f64(acc0), 4:f64(acc1)
        let mut br = Vec::new();
        br.extend_from_slice(&[0x03, 0x40]);
        br.extend_from_slice(&[0x20, 1, 0x41, 5, 0x74, 0x22, 2]); // off = i << 5
        // nx = min(max(x + vx*0.5, -10.0), 10.0)
        br.extend_from_slice(&[0x20, 2, 0x2b, 3, 0, 0x20, 2, 0x2b, 3, 16]);
        push_f64(&mut br, 0.5);
        br.extend_from_slice(&[0xa2, 0xa0]);
        push_f64(&mut br, -10.0);
        br.push(0xa5);
        push_f64(&mut br, 10.0);
        br.push(0xa4);
        br.extend_from_slice(&[0x39, 3, 0]);
        // ny = min(max(y + vy*0.5, -10.0), 10.0)
        br.extend_from_slice(&[0x20, 2, 0x20, 2, 0x2b, 3, 8, 0x20, 2, 0x2b, 3, 24]);
        push_f64(&mut br, 0.5);
        br.extend_from_slice(&[0xa2, 0xa0]);
        push_f64(&mut br, -10.0);
        br.push(0xa5);
        push_f64(&mut br, 10.0);
        br.push(0xa4);
        br.extend_from_slice(&[0x39, 3, 8]);
        // acc0 += mem[off+0]; acc1 += mem[off+8]
        br.extend_from_slice(&[0x20, 3, 0x20, 2, 0x2b, 3, 0, 0xa0, 0x21, 3]);
        br.extend_from_slice(&[0x20, 4, 0x20, 2, 0x2b, 3, 8, 0xa0, 0x21, 4]);
        br.extend_from_slice(&[
            0x20, 1, 0x41, 1, 0x6a, 0x22, 1, 0x20, 0, 0x49, 0x0d, 0, 0x0b,
        ]);
        br.extend_from_slice(&[0x20, 3, 0x20, 4, 0xa0, 0xbd]);

        let mut code = vec![2];
        code.extend_from_slice(&body(&[(2, 0x7f)], &bp));
        code.extend_from_slice(&body(&[(2, 0x7f), (2, 0x7c)], &br));
        wasm_mod(&[
            sec(1, &types),
            sec(3, &funcs),
            sec(5, &mem),
            sec(7, &exps),
            sec(10, &code),
        ])
    }

    #[test]
    fn fast_register_exact_fuel_prefix_traps_calls_memory64_and_cancellation() {
        // Module with:
        // - f0: helper(x: i32) -> i32 { x * 3 + 1 } (called via direct register call)
        // - f1: stores_and_calls(n: i32) -> i32 (writes 3 memory words + calls helper + loop + div)
        // - f2: rec(n: i32) -> i32 (recursive call to test max_call_depth)
        // - f3: call_host(x: i32) -> i32 (calls helper, then calls imported host func -> bail mid-stack!)
        let mut imp = vec![1];
        for s in ["env", "bump"] {
            leb_u32(&mut imp, s.len() as u32);
            imp.extend_from_slice(s.as_bytes());
        }
        imp.extend_from_slice(&[0x00, 0]); // func type 0: (i32)->i32

        let types = vec![
            2, 0x60, 1, 0x7f, 1, 0x7f, // 0: (i32)->i32
            0x60, 2, 0x7f, 0x7f, 2, 0x7f, 0x7f, // 1: (i32, i32) -> (i32, i32)
        ];
        let funcs = vec![5, 0, 0, 0, 0, 1];
        let mem = vec![1, 0x01, 1, 2];
        let mut exps = vec![6];
        for (name, kind, idx) in [
            ("memory", 2u8, 0u32),
            ("helper", 0, 1),
            ("stores_and_calls", 0, 2),
            ("rec", 0, 3),
            ("call_host", 0, 4),
            ("swap_mv", 0, 5),
        ] {
            leb_u32(&mut exps, name.len() as u32);
            exps.extend_from_slice(name.as_bytes());
            exps.push(kind);
            leb_u32(&mut exps, idx);
        }

        // f1 (idx 1): helper(x) = x*3 + 1
        let b1 = vec![0x20, 0, 0x41, 3, 0x6c, 0x41, 1, 0x6a];
        // f2 (idx 2): stores_and_calls(d):
        //   mem[0] = helper(5); mem[4] = 100 / d; mem[8] = helper(9); return mem[0] + mem[4] + mem[8]
        let b2 = vec![
            0x41, 0, 0x41, 5, 0x10, 1, 0x36, 2, 0, // store mem[0] = 16
            0x41, 4, 0x41, 0xe4, 0x00, 0x20, 0, 0x6d, 0x36, 2, 0, // store mem[4] = 100 / d
            0x41, 8, 0x41, 9, 0x10, 1, 0x36, 2, 0, // store mem[8] = 28
            0x41, 0, 0x28, 2, 0, 0x41, 4, 0x28, 2, 0, 0x6a, 0x41, 8, 0x28, 2, 0, 0x6a,
        ];
        // f3 (idx 3): rec(n) = if n == 0 { 1 } else { rec(n - 1) + 1 }
        let b3 = vec![
            0x20, 0, 0x45, 0x04, 0x7f, 0x41, 1, 0x05, 0x20, 0, 0x41, 1, 0x6b, 0x10, 3, 0x41, 1,
            0x6a, 0x0b,
        ];
        // f4 (idx 4): call_host(x) = helper(bump(helper(x)))
        let b4 = vec![0x20, 0, 0x10, 1, 0x10, 0, 0x10, 1];
        // f5 (idx 5): swap_mv(a, b) -> (b+1, a+2) via multivalue block + br_if
        let b5_full = vec![
            0x20, 0, 0x20, 1, // push block params (a, b)
            0x02, 1, // block (i32, i32) -> (i32, i32)
            0x1a, 0x1a, // drop block params
            0x20, 1, 0x41, 1, 0x6a, 0x20, 0, 0x41, 2, 0x6a, 0x41, 1, 0x0d, 0, 0x1a, 0x1a, 0x41, 0,
            0x41, 0, 0x0b,
        ];

        let mut code = vec![5];
        code.extend_from_slice(&body(&[], &b1));
        code.extend_from_slice(&body(&[], &b2));
        code.extend_from_slice(&body(&[], &b3));
        code.extend_from_slice(&body(&[], &b4));
        code.extend_from_slice(&body(&[], &b5_full));

        let wasm = wasm_mod(&[
            sec(1, &types),
            sec(2, &imp),
            sec(3, &funcs),
            sec(5, &mem),
            sec(7, &exps),
            sec(10, &code),
        ]);

        let run_case = |policy: ExecutionPolicy, fuel: Option<u64>, call_depth: usize| {
            let mut cfg = Config::default();
            cfg.limits.fuel = fuel;
            cfg.limits.max_call_depth = call_depth;
            let engine = Engine::with_execution_policy(cfg, policy).unwrap();
            let module = engine.compile(&wasm).unwrap();
            if policy == ExecutionPolicy::Register {
                for b in 0..5 {
                    assert!(is_admitted(&module, b), "body {b} must be admitted");
                }
            }
            let mut store = Store::new(&engine);
            let mut linker = Linker::new(&engine);
            let host_fn = store
                .create_host_func(
                    crate::FuncType {
                        params: vec![ValType::I32],
                        results: vec![ValType::I32],
                    },
                    |args| {
                        let Value::I32(v) = args[0] else { panic!() };
                        Ok(vec![Value::I32(v + 10)])
                    },
                )
                .unwrap();
            linker
                .define("env", "bump", External::Func(host_fn))
                .unwrap();
            let inst = linker.instantiate(&mut store, &module).unwrap();
            (store, inst)
        };

        // 1. Exact fuel prefix sweep (fuel = 0..=55): results AND memory bytes must match Decoded bit-for-bit!
        for fuel in 0..=55u64 {
            let (mut sd, id) = run_case(ExecutionPolicy::Decoded, Some(fuel), 64);
            let (mut sr, ir) = run_case(ExecutionPolicy::Register, Some(fuel), 64);
            let External::Func(fd) = id.export(&sd, "stores_and_calls").unwrap() else {
                panic!()
            };
            let External::Func(fr) = ir.export(&sr, "stores_and_calls").unwrap() else {
                panic!()
            };
            let External::Memory(md) = id.export(&sd, "memory").unwrap() else {
                panic!()
            };
            let External::Memory(mr) = ir.export(&sr, "memory").unwrap() else {
                panic!()
            };
            let od = sd.invoke(fd, &[Value::I32(4)]);
            let or = sr.invoke(fr, &[Value::I32(4)]);
            match (&od, &or) {
                (Ok(a), Ok(b)) => assert_eq!(a, b, "fuel={fuel} result mismatch"),
                (Err(a), Err(b)) => {
                    assert_eq!((a.kind, &a.message), (b.kind, &b.message), "fuel={fuel}")
                }
                _ => panic!("fuel={fuel}: Decoded={od:?} != Register={or:?}"),
            }
            assert_eq!(
                sd.read_memory(md, 0, 16).unwrap(),
                sr.read_memory(mr, 0, 16).unwrap(),
                "fuel={fuel} memory prefix mismatch"
            );
            assert_eq!(
                sd.current_fuel, sr.current_fuel,
                "fuel={fuel} remaining mismatch"
            );
        }

        // 2. Mid-block trap (div by zero) must leave exact earlier store AND exact remaining fuel!
        {
            let (mut sd, id) = run_case(ExecutionPolicy::Decoded, Some(100), 64);
            let (mut sr, ir) = run_case(ExecutionPolicy::Register, Some(100), 64);
            let External::Func(fd) = id.export(&sd, "stores_and_calls").unwrap() else {
                panic!()
            };
            let External::Func(fr) = ir.export(&sr, "stores_and_calls").unwrap() else {
                panic!()
            };
            let External::Memory(md) = id.export(&sd, "memory").unwrap() else {
                panic!()
            };
            let External::Memory(mr) = ir.export(&sr, "memory").unwrap() else {
                panic!()
            };
            let ed = sd.invoke(fd, &[Value::I32(0)]).unwrap_err();
            let er = sr.invoke(fr, &[Value::I32(0)]).unwrap_err();
            assert_eq!((ed.kind, ed.message), (er.kind, er.message));
            assert_eq!(
                sd.read_memory(md, 0, 16).unwrap(),
                sr.read_memory(mr, 0, 16).unwrap()
            );
            assert_eq!(sd.current_fuel, sr.current_fuel);
        }

        // 3. Direct recursion + call depth exhaustion + host bail + multivalue block!
        for n in [0, 5, 12, 20] {
            let (mut sd, id) = run_case(ExecutionPolicy::Decoded, Some(500), 10);
            let (mut sr, ir) = run_case(ExecutionPolicy::Register, Some(500), 10);
            let External::Func(fd) = id.export(&sd, "rec").unwrap() else {
                panic!()
            };
            let External::Func(fr) = ir.export(&sr, "rec").unwrap() else {
                panic!()
            };
            let od = sd.invoke(fd, &[Value::I32(n)]);
            let or = sr.invoke(fr, &[Value::I32(n)]);
            match (&od, &or) {
                (Ok(a), Ok(b)) => assert_eq!(a, b),
                (Err(a), Err(b)) => assert_eq!((a.kind, &a.message), (b.kind, &b.message)),
                _ => panic!("rec({n}): {od:?} vs {or:?}"),
            }
            assert_eq!(sd.current_fuel, sr.current_fuel, "rec({n}) fuel mismatch");
        }
        {
            let (mut sd, id) = run_case(ExecutionPolicy::Decoded, Some(100), 64);
            let (mut sr, ir) = run_case(ExecutionPolicy::Register, Some(100), 64);
            let External::Func(fd) = id.export(&sd, "call_host").unwrap() else {
                panic!()
            };
            let External::Func(fr) = ir.export(&sr, "call_host").unwrap() else {
                panic!()
            };
            assert_eq!(
                sd.invoke(fd, &[Value::I32(7)]).unwrap(),
                sr.invoke(fr, &[Value::I32(7)]).unwrap()
            );
            assert_eq!(sd.current_fuel, sr.current_fuel);

            let External::Func(mvd) = id.export(&sd, "swap_mv").unwrap() else {
                panic!()
            };
            let External::Func(mvr) = ir.export(&sr, "swap_mv").unwrap() else {
                panic!()
            };
            assert_eq!(
                sd.invoke(mvd, &[Value::I32(10), Value::I32(20)]).unwrap(),
                sr.invoke(mvr, &[Value::I32(10), Value::I32(20)]).unwrap()
            );
        }

        // 4. Cancellation stops an infinite/long loop cleanly!
        {
            let (mut sr, ir) = run_case(ExecutionPolicy::Register, None, 64);
            sr.cancellation_token().cancel();
            let External::Func(fr) = ir.export(&sr, "stores_and_calls").unwrap() else {
                panic!()
            };
            let e = sr.invoke(fr, &[Value::I32(4)]).unwrap_err();
            assert_eq!(
                (e.kind, e.message.as_str()),
                (ErrorKind::ResourceLimit, "execution cancelled")
            );
        }

        // 5. Memory64 parity!
        {
            let m64 = wasm_mod(&[
                sec(1, &[1, 0x60, 1, 0x7e, 1, 0x7e]),
                sec(3, &[1, 0]),
                sec(5, &[1, 0x05, 1, 2]),
                sec(7, &[1, 3, b'r', b'u', b'n', 0, 0]),
                sec(
                    10,
                    &[1].into_iter()
                        .chain(body(
                            &[],
                            &[
                                0x42, 16, 0x20, 0, 0x37, 3, 0, // i64.store(16, x)
                                0x42, 16, 0x29, 3, 0, 0x42, 7, 0x7c, // i64.load(16) + 7
                            ],
                        ))
                        .collect::<Vec<_>>(),
                ),
            ]);
            let cfg = Config {
                features: FeatureSet::all_phase3_core(),
                ..Config::default()
            };
            let ed = Engine::with_execution_policy(cfg.clone(), ExecutionPolicy::Decoded).unwrap();
            let er = Engine::with_execution_policy(cfg, ExecutionPolicy::Register).unwrap();
            let md = ed.compile(&m64).unwrap();
            let mr = er.compile(&m64).unwrap();
            assert!(is_admitted(&mr, 0));
            let mut sd = Store::new(&ed);
            let mut sr = Store::new(&er);
            let id = Linker::new(&ed).instantiate(&mut sd, &md).unwrap();
            let ir = Linker::new(&er).instantiate(&mut sr, &mr).unwrap();
            let External::Func(fd) = id.export(&sd, "run").unwrap() else {
                panic!()
            };
            let External::Func(fr) = ir.export(&sr, "run").unwrap() else {
                panic!()
            };
            assert_eq!(
                sd.invoke(fd, &[Value::I64(42)]).unwrap(),
                sr.invoke(fr, &[Value::I64(42)]).unwrap()
            );
        }

        // 6. RegOp compact size invariant & Lemire u32 reciprocal exactness!
        assert!(
            std::mem::size_of::<RegOp>() <= 12,
            "RegOp grew to {} bytes (> 12B)",
            std::mem::size_of::<RegOp>()
        );
        for d in [3u32, 5, 7, 10, 17, 255, 65535, 1_000_000_007, u32::MAX] {
            let magic = (u64::MAX / u64::from(d)) + 1;
            for a in [
                0u32,
                1,
                d - 1,
                d,
                d.saturating_add(1),
                u32::MAX / 2,
                u32::MAX - d,
                u32::MAX - 1,
                u32::MAX,
            ] {
                let q = (((u128::from(a)) * u128::from(magic)) >> 64) as u32;
                assert_eq!(q, a / d, "Lemire reciprocal mismatch for {a} / {d}");
            }
        }
    }

    #[test]
    fn fast_register_call_indirect_bulk_memory_and_grow_parity() {
        // Module with:
        // - type 0: (i32) -> i32
        // - type 1: (i32, i32) -> i32
        // - type 2: () -> i32
        // - import env.bump : type 0 (host func, tests call_indirect bail to host!)
        // - table 0: funcref min=5, elem [f1(double), f2(add), host(bump)] at indices 0, 1, 2 (index 3 is null)
        // - memory 0: min=1 max=3
        // - f1 (idx 1): double(x) = x * 2
        // - f2 (idx 2): add(a, b) = a + b
        // - f3 (idx 3): indirect_dispatch(idx, x) -> i32 = call_indirect(type 0, table 0)(x, idx)
        // - f4 (idx 4): bulk_and_grow() -> i32:
        //     memory.fill(0, 0x5a, 32)
        //     memory.copy(16, 0, 32) // overlapping forward copy
        //     memory.copy(8, 16, 16) // overlapping backward copy
        //     old = memory.grow(1)   // 1 -> 2 pages
        //     fail = memory.grow(5)  // > max(3) -> -1
        //     store i32 at 65536 (page 1) = 0x12345678
        //     memory.copy(65540, 0, 4) // cross-page copy page 0 -> page 1!
        //     return old + fail + load(65536) + load8_u(65540) + load8_u(10)
        // - f5 (idx 5): bulk_oob(dst, src, len) -> (): memory.copy(dst, src, len)
        let mut imp = vec![1];
        for s in ["env", "bump"] {
            leb_u32(&mut imp, s.len() as u32);
            imp.extend_from_slice(s.as_bytes());
        }
        imp.extend_from_slice(&[0x00, 0]);

        let types = vec![
            4, 0x60, 1, 0x7f, 1, 0x7f, // 0: (i32) -> i32
            0x60, 2, 0x7f, 0x7f, 1, 0x7f, // 1: (i32, i32) -> i32
            0x60, 0, 1, 0x7f, // 2: () -> i32
            0x60, 3, 0x7f, 0x7f, 0x7f, 0, // 3: (i32, i32, i32) -> ()
        ];
        let funcs = vec![5, 0, 1, 1, 2, 3];
        let tables = vec![1, 0x70, 0x00, 5]; // funcref min=5
        let mem = vec![1, 0x01, 1, 3]; // min=1 max=3
        let mut exps = vec![4];
        for (name, kind, idx) in [
            ("memory", 2u8, 0u32),
            ("indirect_dispatch", 0, 3),
            ("bulk_and_grow", 0, 4),
            ("bulk_oob", 0, 5),
        ] {
            leb_u32(&mut exps, name.len() as u32);
            exps.extend_from_slice(name.as_bytes());
            exps.push(kind);
            leb_u32(&mut exps, idx);
        }
        // elem segment: table 0, offset 0, [func 1, func 2, func 0]
        let elem = vec![1, 0x00, 0x41, 0x00, 0x0b, 3, 1, 2, 0];

        let b1 = vec![0x20, 0, 0x41, 2, 0x6c]; // x * 2
        let b2 = vec![0x20, 0, 0x20, 1, 0x6a]; // a + b
        let b3 = vec![0x20, 1, 0x20, 0, 0x11, 0, 0]; // call_indirect type=0 table=0 (x, idx)
        let mut b4 = Vec::new();
        // memory.fill(0, 0x5a, 32)
        b4.extend_from_slice(&[0x41, 0, 0x41, 0xda, 0x00, 0x41, 32, 0xfc, 0x0b, 0]);
        // memory.copy(16, 0, 32)
        b4.extend_from_slice(&[0x41, 16, 0x41, 0, 0x41, 32, 0xfc, 0x0a, 0, 0]);
        // memory.copy(8, 16, 16)
        b4.extend_from_slice(&[0x41, 8, 0x41, 16, 0x41, 16, 0xfc, 0x0a, 0, 0]);
        // local 0 = memory.grow(1)
        b4.extend_from_slice(&[0x41, 1, 0x40, 0, 0x21, 0]);
        // local 1 = memory.grow(5) -> -1
        b4.extend_from_slice(&[0x41, 5, 0x40, 0, 0x21, 1]);
        // i32.store(65536, 123456)
        b4.push(0x41);
        b4.extend_from_slice(&[0x80, 0x80, 0x04]); // 65536 sleb128
        b4.push(0x41);
        b4.extend_from_slice(&[0xc0, 0xc4, 0x07]); // 123456 sleb128
        b4.extend_from_slice(&[0x36, 2, 0]);
        // memory.copy(65540, 0, 4) -> cross-page!
        b4.push(0x41);
        b4.extend_from_slice(&[0x84, 0x80, 0x04]); // 65540 sleb128
        b4.extend_from_slice(&[0x41, 0, 0x41, 4, 0xfc, 0x0a, 0, 0]);
        // result = local0 + local1 + load(65536) + load8_u(65540) + load8_u(10)
        b4.extend_from_slice(&[0x20, 0, 0x20, 1, 0x6a]);
        b4.push(0x41);
        b4.extend_from_slice(&[0x80, 0x80, 0x04]);
        b4.extend_from_slice(&[0x28, 2, 0, 0x6a]);
        b4.push(0x41);
        b4.extend_from_slice(&[0x84, 0x80, 0x04]);
        b4.extend_from_slice(&[0x2d, 0, 0, 0x6a]);
        b4.extend_from_slice(&[0x41, 10, 0x2d, 0, 0, 0x6a]);

        let b5 = vec![0x20, 0, 0x20, 1, 0x20, 2, 0xfc, 0x0a, 0, 0];

        let mut code = vec![5];
        code.extend_from_slice(&body(&[], &b1));
        code.extend_from_slice(&body(&[], &b2));
        code.extend_from_slice(&body(&[], &b3));
        code.extend_from_slice(&body(&[(2, 0x7f)], &b4));
        code.extend_from_slice(&body(&[], &b5));

        let wasm = wasm_mod(&[
            sec(1, &types),
            sec(2, &imp),
            sec(3, &funcs),
            sec(4, &tables),
            sec(5, &mem),
            sec(7, &exps),
            sec(9, &elem),
            sec(10, &code),
        ]);

        let make_inst = |policy: ExecutionPolicy, fuel: Option<u64>| {
            let mut cfg = Config::default();
            cfg.limits.fuel = fuel;
            let engine = Engine::with_execution_policy(cfg, policy).unwrap();
            let module = engine.compile(&wasm).unwrap();
            if policy == ExecutionPolicy::Register {
                for b in 0..5 {
                    assert!(is_admitted(&module, b), "body {b} must be admitted");
                }
            }
            let mut store = Store::new(&engine);
            let mut linker = Linker::new(&engine);
            let host_fn = store
                .create_host_func(
                    crate::FuncType {
                        params: vec![ValType::I32],
                        results: vec![ValType::I32],
                    },
                    |args| {
                        let Value::I32(v) = args[0] else { panic!() };
                        Ok(vec![Value::I32(v + 100)])
                    },
                )
                .unwrap();
            linker
                .define("env", "bump", External::Func(host_fn))
                .unwrap();
            let inst = linker.instantiate(&mut store, &module).unwrap();
            (store, inst)
        };

        // Test call_indirect: idx=0 (guest double), idx=1 (type mismatch), idx=2 (host bail), idx=3 (null), idx=9 (OOB)
        for idx in [0, 1, 2, 3, 9] {
            for fuel in [None, Some(0), Some(1), Some(3), Some(4), Some(100)] {
                let (mut sd, id) = make_inst(ExecutionPolicy::Decoded, fuel);
                let (mut sr, ir) = make_inst(ExecutionPolicy::Register, fuel);
                let External::Func(fd) = id.export(&sd, "indirect_dispatch").unwrap() else {
                    panic!()
                };
                let External::Func(fr) = ir.export(&sr, "indirect_dispatch").unwrap() else {
                    panic!()
                };
                let od = sd.invoke(fd, &[Value::I32(idx), Value::I32(21)]);
                let or = sr.invoke(fr, &[Value::I32(idx), Value::I32(21)]);
                match (&od, &or) {
                    (Ok(a), Ok(b)) => assert_eq!(a, b, "idx={idx} fuel={fuel:?}"),
                    (Err(a), Err(b)) => {
                        assert_eq!(
                            (a.kind, a.trap_code, &a.message),
                            (b.kind, b.trap_code, &b.message),
                            "idx={idx} fuel={fuel:?}"
                        )
                    }
                    _ => panic!("idx={idx} fuel={fuel:?}: {od:?} vs {or:?}"),
                }
                assert_eq!(
                    sd.current_fuel, sr.current_fuel,
                    "idx={idx} fuel={fuel:?} remaining mismatch"
                );
            }
        }

        // Test bulk_and_grow with exact fuel prefix sweep!
        for fuel in 0..=45u64 {
            let (mut sd, id) = make_inst(ExecutionPolicy::Decoded, Some(fuel));
            let (mut sr, ir) = make_inst(ExecutionPolicy::Register, Some(fuel));
            let External::Func(fd) = id.export(&sd, "bulk_and_grow").unwrap() else {
                panic!()
            };
            let External::Func(fr) = ir.export(&sr, "bulk_and_grow").unwrap() else {
                panic!()
            };
            let od = sd.invoke(fd, &[]);
            let or = sr.invoke(fr, &[]);
            match (&od, &or) {
                (Ok(a), Ok(b)) => assert_eq!(a, b, "bulk fuel={fuel}"),
                (Err(a), Err(b)) => assert_eq!((a.kind, &a.message), (b.kind, &b.message)),
                _ => panic!("bulk fuel={fuel}: {od:?} vs {or:?}"),
            }
            assert_eq!(sd.current_fuel, sr.current_fuel, "bulk fuel={fuel}");
        }

        // Test zero-length copy at exact mem_len (valid) vs mem_len + 1 (trap!)
        for (dst, src, len) in [(65536, 0, 0), (65537, 0, 0), (65530, 0, 10)] {
            let (mut sd, id) = make_inst(ExecutionPolicy::Decoded, Some(50));
            let (mut sr, ir) = make_inst(ExecutionPolicy::Register, Some(50));
            let External::Func(fd) = id.export(&sd, "bulk_oob").unwrap() else {
                panic!()
            };
            let External::Func(fr) = ir.export(&sr, "bulk_oob").unwrap() else {
                panic!()
            };
            let od = sd.invoke(fd, &[Value::I32(dst), Value::I32(src), Value::I32(len)]);
            let or = sr.invoke(fr, &[Value::I32(dst), Value::I32(src), Value::I32(len)]);
            match (&od, &or) {
                (Ok(a), Ok(b)) => assert_eq!(a, b),
                (Err(a), Err(b)) => assert_eq!((a.kind, &a.message), (b.kind, &b.message)),
                _ => panic!("bulk_oob({dst},{src},{len}): {od:?} vs {or:?}"),
            }
            assert_eq!(sd.current_fuel, sr.current_fuel);
        }
    }

    #[test]
    fn fast_register_kernels_parity_speedup_and_large_module_admission() {
        let fixtures_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/wtime-aot/fixtures");
        let ecs_wasm = build_ecs_f64_wasm();
        let mut cases: Vec<(&str, Vec<u8>, i32, usize)> = vec![("ecs_f64", ecs_wasm, 512, 30)];
        for (name, file, n) in [
            ("graph_bfs", "graph_bfs.wasm", 512),
            ("parser_hash", "parser_hash.wasm", 1024),
            ("image_filter", "image_filter.wasm", 256),
        ] {
            if let Ok(bytes) = std::fs::read(fixtures_dir.join(file)) {
                cases.push((name, bytes, n, 30));
            }
        }

        for (name, bytes, n, iters) in cases {
            let mut times_us = [0.0f64; 2];
            let mut outputs = Vec::new();
            for (idx, policy) in [ExecutionPolicy::Decoded, ExecutionPolicy::Register]
                .into_iter()
                .enumerate()
            {
                let engine = Engine::with_execution_policy(Config::default(), policy).unwrap();
                let module = engine.compile(&bytes).unwrap();
                let mut store = Store::new(&engine);
                let inst = Linker::new(&engine)
                    .instantiate(&mut store, &module)
                    .unwrap();
                let External::Func(prep) = inst.export(&store, "prepare").unwrap() else {
                    panic!()
                };
                let External::Func(run_fn) = inst.export(&store, "run").unwrap() else {
                    panic!()
                };
                store.invoke(prep, &[Value::I32(n)]).unwrap();
                // Warm up
                let first = store.invoke(run_fn, &[Value::I32(n)]).unwrap();
                let t0 = std::time::Instant::now();
                let mut last = first;
                for _ in 0..iters {
                    last = store.invoke(run_fn, &[Value::I32(n)]).unwrap();
                }
                let us = t0.elapsed().as_secs_f64() * 1e6 / iters as f64;
                times_us[idx] = us;
                outputs.push(last);
            }
            assert_eq!(outputs[0], outputs[1], "{name}: Register != Decoded output");
            eprintln!(
                "HILL5_BENCH {name}: decoded={:.1} us, register={:.1} us, speedup={:.2}x",
                times_us[0],
                times_us[1],
                times_us[0] / times_us[1].max(0.001)
            );
        }

        // Verify lazy admission on real 5,028-function game.wasm (if present on host):
        let game_path = std::path::Path::new("/home/mewhhaha/src/gdev/.pen/runtime/game.wasm");
        if let Ok(game_bytes) = std::fs::read(game_path) {
            let cfg = Config {
                features: FeatureSet::all_phase3_core(),
                limits: crate::ResourceLimits {
                    max_module_bytes: 32 << 20,
                    max_functions: 32_768,
                    max_instructions: 4_000_000,
                    max_locals: 16_384,
                    max_total_locals: 1_000_000,
                    ..Default::default()
                },
            };
            let engine = Engine::with_execution_policy(cfg, ExecutionPolicy::Register).unwrap();
            let t0 = std::time::Instant::now();
            let module = engine.compile(&game_bytes).unwrap();
            let compile_ms = t0.elapsed().as_secs_f64() * 1e3;
            let total = module.data.inner.decoded().code.len();
            assert!(
                module.data.plans.is_none(),
                "expected game.wasm to exceed eager 100k FunctionPlans cap"
            );
            let t1 = std::time::Instant::now();
            let mut admitted = 0usize;
            for b in 0..total {
                if is_admitted(&module, b) {
                    admitted += 1;
                }
            }
            let lazy_ms = t1.elapsed().as_secs_f64() * 1e3;
            eprintln!(
                "HILL5_GAME_WASM: total_fns={total}, admitted_register_fast={admitted} ({:.1}%), compile_ms={compile_ms:.1}, lazy_compile_all_ms={lazy_ms:.1}",
                100.0 * admitted as f64 / total as f64
            );
            assert!(
                admitted > total / 2,
                "expected majority of game.wasm functions admitted"
            );
        }
    }

    #[test]
    fn fast_register_dense_multipage_parity_and_demotion() {
        // 8-page (512 KiB) memory with 4-page strided scatter/gather across pages 0, 2, 4, 6,
        // cross-64KiB-page unaligned i32 load/store, cross-page memory.copy/fill, OOB trap,
        // and growth past DENSE_MAX_BYTES (16 MiB) verifying clean demotion back to sparse!
        //
        // fn stride_sum(n: i32) -> i32:
        //   for i in 0..n:
        //     addr0 = i * 4 (page 0..1)
        //     addr1 = 131072 + i * 4 (page 2..3)
        //     addr2 = 262144 + i * 4 (page 4..5)
        //     addr3 = 393216 + i * 4 (page 6..7)
        //     v = mem[addr0] + mem[addr1] + mem[addr2]
        //     mem[addr3] = v ^ i
        //     acc += v
        //   // Cross-64KiB-page unaligned write & read at 65534:
        //   mem[65534] = acc
        //   // Cross-page memory.copy (20 bytes from 65526 -> 131062):
        //   memory.copy(131062, 65526, 20)
        //   // Cross-page memory.fill (8 bytes at 196604 = 0x3c):
        //   memory.fill(196604, 0x3c, 8)
        //   return mem[131070] + mem[196606]
        //
        // fn grow_and_probe(delta: i32, probe_addr: i32) -> i32:
        //   let prev = memory.grow(delta);
        //   mem[probe_addr] = prev + 0x1234_5678;
        //   mem[probe_addr]
        let types = {
            let mut p = vec![2];
            // type 0: (i32) -> (i32)
            p.extend_from_slice(&[0x60, 1, 0x7f, 1, 0x7f]);
            // type 1: (i32, i32) -> (i32)
            p.extend_from_slice(&[0x60, 2, 0x7f, 0x7f, 1, 0x7f]);
            p
        };
        let funcs = vec![2, 0, 1];
        // Initial 8 pages (512 KiB), max 512 pages (32 MiB) so grow past 16 MiB works.
        let mut mem = vec![1, 0x01];
        leb_u32(&mut mem, 8);
        leb_u32(&mut mem, 512);
        let mut exps = vec![3];
        for (name, kind, idx) in [
            ("memory", 2u8, 0u32),
            ("stride_sum", 0, 0),
            ("grow_and_probe", 0, 1),
        ] {
            leb_u32(&mut exps, name.len() as u32);
            exps.extend_from_slice(name.as_bytes());
            exps.push(kind);
            leb_u32(&mut exps, idx);
        }

        // Body 0: stride_sum(n: i32) -> i32; locals: 1: i, 2: acc, 3: off, 4: v
        let mut b0 = Vec::new();
        let push_i32 = |out: &mut Vec<u8>, v: i32| {
            out.push(0x41);
            let mut x = v;
            loop {
                let b = (x & 0x7f) as u8;
                x >>= 7;
                if (x == 0 && (b & 0x40) == 0) || (x == -1 && (b & 0x40) != 0) {
                    out.push(b);
                    break;
                }
                out.push(b | 0x80);
            }
        };
        b0.extend_from_slice(&[0x03, 0x40]); // loop
        b0.extend_from_slice(&[0x20, 1, 0x20, 0, 0x49, 0x04, 0x40]); // if i <u n
        b0.extend_from_slice(&[0x20, 1, 0x41, 2, 0x74, 0x21, 3]); // off = i << 2
        // mem[off] = i + 1; mem[off+131072] = i * 3; mem[off+262144] = i * 5
        b0.extend_from_slice(&[0x20, 3, 0x20, 1, 0x41, 1, 0x6a, 0x36, 2, 0]);
        b0.extend_from_slice(&[0x20, 3]);
        push_i32(&mut b0, 131_072);
        b0.extend_from_slice(&[0x6a, 0x20, 1, 0x41, 3, 0x6c, 0x36, 2, 0]);
        b0.extend_from_slice(&[0x20, 3]);
        push_i32(&mut b0, 262_144);
        b0.extend_from_slice(&[0x6a, 0x20, 1, 0x41, 5, 0x6c, 0x36, 2, 0]);
        // v = mem[off] + mem[off+131072] + mem[off+262144]
        b0.extend_from_slice(&[0x20, 3, 0x28, 2, 0]);
        b0.extend_from_slice(&[0x20, 3]);
        push_i32(&mut b0, 131_072);
        b0.extend_from_slice(&[0x6a, 0x28, 2, 0, 0x6a]);
        b0.extend_from_slice(&[0x20, 3]);
        push_i32(&mut b0, 262_144);
        b0.extend_from_slice(&[0x6a, 0x28, 2, 0, 0x6a, 0x21, 4]);
        // mem[off+393216] = v ^ i
        b0.extend_from_slice(&[0x20, 3]);
        push_i32(&mut b0, 393_216);
        b0.extend_from_slice(&[0x6a, 0x20, 4, 0x20, 1, 0x73, 0x36, 2, 0]);
        // acc += mem[off+393216]
        b0.extend_from_slice(&[0x20, 2, 0x20, 3]);
        push_i32(&mut b0, 393_216);
        b0.extend_from_slice(&[0x6a, 0x28, 2, 0, 0x6a, 0x21, 2]);
        // i += 1; br 1
        b0.extend_from_slice(&[0x20, 1, 0x41, 1, 0x6a, 0x21, 1, 0x0c, 1, 0x0b, 0x0b]);
        // Cross-page unaligned store at 65534:
        push_i32(&mut b0, 65_534);
        b0.extend_from_slice(&[0x20, 2, 0x36, 0, 0]);
        // Cross-page memory.copy(131062, 65526, 20):
        push_i32(&mut b0, 131_062);
        push_i32(&mut b0, 65_526);
        push_i32(&mut b0, 20);
        b0.extend_from_slice(&[0xfc, 10, 0, 0]);
        // Cross-page memory.fill(196604, 0x3c, 8):
        push_i32(&mut b0, 196_604);
        push_i32(&mut b0, 0x3c);
        push_i32(&mut b0, 8);
        b0.extend_from_slice(&[0xfc, 11, 0]);
        // return mem[131070] + mem[196606]
        push_i32(&mut b0, 131_070);
        b0.extend_from_slice(&[0x28, 0, 0]);
        push_i32(&mut b0, 196_606);
        b0.extend_from_slice(&[0x28, 0, 0, 0x6a]);

        // Body 1: grow_and_probe(delta: i32, probe_addr: i32) -> i32
        let mut b1 = Vec::new();
        b1.extend_from_slice(&[0x20, 1, 0x20, 0, 0x40, 0]); // probe_addr, memory.grow(delta)
        push_i32(&mut b1, 0x1234_5678);
        b1.extend_from_slice(&[0x6a, 0x36, 2, 0, 0x20, 1, 0x28, 2, 0]);

        let mut code = vec![2];
        code.extend_from_slice(&body(&[(4, 0x7f)], &b0));
        code.extend_from_slice(&body(&[], &b1));

        let mut wasm = b"\0asm\x01\0\0\0".to_vec();
        for s in [
            sec(1, &types),
            sec(3, &funcs),
            sec(5, &mem),
            sec(7, &exps),
            sec(10, &code),
        ] {
            wasm.extend_from_slice(&s);
        }

        // 1. Parity + speedup on multi-page workload (n = 4096 -> spans all 8 pages / 512 KiB!)
        let mut results = Vec::new();
        let mut times_us = [0.0f64; 2];
        for (idx, policy) in [ExecutionPolicy::Decoded, ExecutionPolicy::Register]
            .into_iter()
            .enumerate()
        {
            let mut cfg = Config::default();
            cfg.limits.max_memory_pages = 512;
            cfg.limits.max_memory_bytes = 32 << 20;
            cfg.limits.max_store_memory_bytes = 32 << 20;
            let engine = Engine::with_execution_policy(cfg, policy).unwrap();
            let module = engine.compile(&wasm).unwrap();
            let mut store = Store::new(&engine);
            let inst = Linker::new(&engine)
                .instantiate(&mut store, &module)
                .unwrap();
            let External::Func(f_sum) = inst.export(&store, "stride_sum").unwrap() else {
                panic!()
            };
            let External::Func(f_grow) = inst.export(&store, "grow_and_probe").unwrap() else {
                panic!()
            };
            let first = store.invoke(f_sum, &[Value::I32(4096)]).unwrap();
            if policy == ExecutionPolicy::Register {
                let mem_h = store.instances[0].mems[0];
                assert!(
                    store.mems[mem_h.0.index].bytes.is_dense(),
                    "512 KiB active memory must be promoted to dense backing"
                );
            }
            let t0 = std::time::Instant::now();
            for _ in 0..20 {
                let r = store.invoke(f_sum, &[Value::I32(4096)]).unwrap();
                assert_eq!(r, first);
            }
            times_us[idx] = t0.elapsed().as_secs_f64() * 1e6 / 20.0;

            // OOB trap check at exact boundary (512 KiB - 2 -> 4-byte store traps!)
            let oob_err = store
                .invoke(f_grow, &[Value::I32(0), Value::I32(8 * 65536 - 2)])
                .unwrap_err();
            assert_eq!(oob_err.kind, ErrorKind::Trap);

            // Grow past DENSE_MAX_BYTES (8 + 256 = 264 pages = 16.5 MiB) -> demotes cleanly to sparse!
            let probe = store
                .invoke(f_grow, &[Value::I32(256), Value::I32(260 * 65536 + 12)])
                .unwrap();
            if policy == ExecutionPolicy::Register {
                let mem_h = store.instances[0].mems[0];
                assert!(
                    !store.mems[mem_h.0.index].bytes.is_dense(),
                    "16.5 MiB memory (> DENSE_MAX_BYTES) must demote back to sparse"
                );
            }
            // Subsequent multi-page run still matches after demotion!
            let after_demote = store.invoke(f_sum, &[Value::I32(64)]).unwrap();
            results.push((first, probe, after_demote));
        }
        assert_eq!(results[0], results[1], "Register != Decoded on multipage");
        eprintln!(
            "STEP5_DENSE_MULTIPAGE (8 pages / 512 KiB): decoded={:.1} us, register={:.1} us, speedup={:.2}x",
            times_us[0],
            times_us[1],
            times_us[0] / times_us[1].max(0.001)
        );

        // 2. Fuel prefix parity on dense promotion + cross-page ops.
        for fuel in [0u64, 1, 5, 25, 60, 120, 500] {
            let mut res_pair = Vec::new();
            for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                let mut cfg = Config::default();
                cfg.limits.fuel = Some(fuel);
                let engine = Engine::with_execution_policy(cfg, policy).unwrap();
                let module = engine.compile(&wasm).unwrap();
                let mut store = Store::new(&engine);
                let inst = Linker::new(&engine)
                    .instantiate(&mut store, &module)
                    .unwrap();
                let External::Func(f_sum) = inst.export(&store, "stride_sum").unwrap() else {
                    panic!()
                };
                let out = store.invoke(f_sum, &[Value::I32(4)]);
                res_pair.push((out.map_err(|e| (e.kind, e.message)), store.current_fuel));
            }
            assert_eq!(res_pair[0], res_pair[1], "dense fuel={fuel} mismatch");
        }
    }

    #[test]
    fn fast_register_tail_call_and_wide_arith_full_parity() {
        let mut features = FeatureSet::stable_core();
        features.tail_call = true;
        features.wide_arithmetic = true;

        // Module with:
        // - f0: import env::host_add (i32) -> (i32)
        // - f1: even(n) -> if n == 0 { 1 } else { return_call odd(n - 1) }  [3 extra locals]
        // - f2: odd(n)  -> if n == 0 { 0 } else { return_call even(n - 1) } [0 extra locals]
        // - f3: even_ind(n) -> if n == 0 { 1 } else { return_call_indirect type 0 table 0 (n-1, 1) }
        // - f4: odd_ind(n)  -> if n == 0 { 0 } else { return_call_indirect type 0 table 0 (n-1, 0) }
        // - f5: tail_host(x) -> return_call host_add(x + 10) [bails cleanly to decoded host tail call!]
        // - f6: inf_tail(n) -> return_call inf_tail(n + 1) [cancellation & fuel test]
        // - f7: wide_test(a: i64, b: i64) -> i64 (exercises i64.add128, sub128, mul_wide_s, mul_wide_u)
        let types = vec![
            2, // 2 types
            0x60, 1, 0x7f, 1, 0x7f, // type 0: (i32) -> (i32)
            0x60, 2, 0x7e, 0x7e, 1, 0x7e, // type 1: (i64, i64) -> (i64)
        ];
        let mut imps = vec![1];
        imps.push(3);
        imps.extend_from_slice(b"env");
        imps.push(8);
        imps.extend_from_slice(b"host_add");
        imps.push(0x00);
        imps.push(0);
        let funcs = vec![7, 0, 0, 0, 0, 0, 0, 1];
        let tbl = vec![1, 0x70, 0x00, 3]; // funcref len=3 (slot 2 null!)
        let elem = vec![1, 0x00, 0x41, 0x00, 0x0b, 2, 3, 4]; // [even_ind, odd_ind]
        let mut exps = vec![5];
        for (name, idx) in [
            ("even", 1u32),
            ("even_ind", 3),
            ("tail_host", 5),
            ("inf_tail", 6),
            ("wide_test", 7),
        ] {
            exps.push(name.len() as u8);
            exps.extend_from_slice(name.as_bytes());
            exps.push(0x00);
            leb_u32(&mut exps, idx);
        }

        let b_even = vec![
            0x20, 0, 0x45, 0x04, 0x7f, 0x41, 1, 0x05, 0x20, 0, 0x41, 1, 0x6b, 0x12, 2, 0x0b,
        ];
        let b_odd = vec![
            0x20, 0, 0x45, 0x04, 0x7f, 0x41, 0, 0x05, 0x20, 0, 0x41, 1, 0x6b, 0x12, 1, 0x0b,
        ];
        let b_even_ind = vec![
            0x20, 0, 0x45, 0x04, 0x7f, 0x41, 1, 0x05, 0x20, 0, 0x41, 1, 0x6b, 0x41, 1, 0x13, 0, 0,
            0x0b,
        ];
        let b_odd_ind = vec![
            0x20, 0, 0x45, 0x04, 0x7f, 0x41, 0, 0x05, 0x20, 0, 0x41, 1, 0x6b, 0x41, 0, 0x13, 0, 0,
            0x0b,
        ];
        let b_tail_host = vec![0x20, 0, 0x41, 10, 0x6a, 0x12, 0];
        let b_inf_tail = vec![0x20, 0, 0x41, 1, 0x6a, 0x12, 6];
        // wide_test(a, b):
        // (lo1, hi1) = mul_wide_s(a, b)
        // (lo2, hi2) = mul_wide_u(a, b)
        // (lo3, hi3) = add128(lo1, hi1, lo2, hi2)
        // (lo4, hi4) = sub128(lo3, hi3, a, b)
        // return lo4 ^ (hi4 * 31)
        let b_wide = vec![
            0x20, 0, 0x20, 1, 0xfc, 0x15, // (lo1, hi1)
            0x20, 0, 0x20, 1, 0xfc, 0x16, // (lo2, hi2)
            0xfc, 0x13, // add128 -> (lo3, hi3)
            0x20, 0, 0x20, 1, 0xfc, 0x14, // sub128 -> (lo4, hi4)
            0x42, 31, 0x7e, 0x85, // hi4 * 31 ^ lo4
        ];
        let mut code = vec![7];
        code.extend_from_slice(&body(&[(3, 0x7f)], &b_even));
        code.extend_from_slice(&body(&[], &b_odd));
        code.extend_from_slice(&body(&[], &b_even_ind));
        code.extend_from_slice(&body(&[], &b_odd_ind));
        code.extend_from_slice(&body(&[], &b_tail_host));
        code.extend_from_slice(&body(&[], &b_inf_tail));
        code.extend_from_slice(&body(&[], &b_wide));

        let wasm = wasm_mod(&[
            sec(1, &types),
            sec(2, &imps),
            sec(3, &funcs),
            sec(4, &tbl),
            sec(7, &exps),
            sec(9, &elem),
            sec(10, &code),
        ]);

        // 1. Deep O(1) tail recursion (50,000 steps with max_call_depth = 2!), host tail call, and wide-arith parity.
        let mut results = Vec::new();
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
            let mut cfg = Config {
                features,
                ..Config::default()
            };
            cfg.limits.max_call_depth = 2;
            let engine = Engine::with_execution_policy(cfg, policy).unwrap();
            let module = engine.compile(&wasm).unwrap();
            if policy == ExecutionPolicy::Register {
                for bi in 0..7 {
                    assert!(
                        is_admitted(&module, bi),
                        "body {bi} must be admitted by fast register tier"
                    );
                }
            }
            let mut store = Store::new(&engine);
            let mut linker = Linker::new(&engine);
            let host_fn = store
                .create_host_func(
                    crate::FuncType {
                        params: vec![ValType::I32],
                        results: vec![ValType::I32],
                    },
                    |args| {
                        let Value::I32(x) = args[0] else { panic!() };
                        Ok(vec![Value::I32(x.wrapping_mul(3).wrapping_add(1))])
                    },
                )
                .unwrap();
            linker
                .define("env", "host_add", External::Func(host_fn))
                .unwrap();
            let inst = linker.instantiate(&mut store, &module).unwrap();
            let get = |s: &Store, name: &str| match inst.export(s, name).unwrap() {
                External::Func(f) => f,
                _ => panic!(),
            };
            let f_even = get(&store, "even");
            let f_ind = get(&store, "even_ind");
            let f_host = get(&store, "tail_host");
            let f_wide = get(&store, "wide_test");
            let r_even = store.invoke(f_even, &[Value::I32(50_000)]).unwrap();
            let r_odd = store.invoke(f_even, &[Value::I32(50_001)]).unwrap();
            let r_ind = store.invoke(f_ind, &[Value::I32(10_000)]).unwrap();
            let r_host = store.invoke(f_host, &[Value::I32(7)]).unwrap();
            let r_w1 = store
                .invoke(f_wide, &[Value::I64(i64::MIN), Value::I64(-1)])
                .unwrap();
            let r_w2 = store
                .invoke(
                    f_wide,
                    &[Value::I64(-123_456_789_012_345), Value::I64(987_654_321)],
                )
                .unwrap();
            assert_eq!(store.active_local_slots, 0, "local slots leaked");
            results.push((r_even, r_odd, r_ind, r_host, r_w1, r_w2));
        }
        assert_eq!(
            results[0], results[1],
            "Register != Decoded on tail_call/wide"
        );
        assert_eq!(
            results[1].0,
            crate::RunOutcome::Returned(vec![Value::I32(1)])
        );
        assert_eq!(
            results[1].1,
            crate::RunOutcome::Returned(vec![Value::I32(0)])
        );
        assert_eq!(
            results[1].2,
            crate::RunOutcome::Returned(vec![Value::I32(1)])
        );
        assert_eq!(
            results[1].3,
            crate::RunOutcome::Returned(vec![Value::I32(52)])
        );

        // 2. Instruction-exact fuel parity across tail calls (including host tail call & infinite tail loop).
        for fuel in [0u64, 1, 2, 3, 4, 5, 7, 10, 15, 25, 40, 60] {
            let mut pair = Vec::new();
            for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                let mut cfg = Config {
                    features,
                    ..Config::default()
                };
                cfg.limits.fuel = Some(fuel);
                let engine = Engine::with_execution_policy(cfg, policy).unwrap();
                let module = engine.compile(&wasm).unwrap();
                let mut store = Store::new(&engine);
                let mut linker = Linker::new(&engine);
                let host_fn = store
                    .create_host_func(
                        crate::FuncType {
                            params: vec![ValType::I32],
                            results: vec![ValType::I32],
                        },
                        |args| Ok(args.to_vec()),
                    )
                    .unwrap();
                linker
                    .define("env", "host_add", External::Func(host_fn))
                    .unwrap();
                let inst = linker.instantiate(&mut store, &module).unwrap();
                let External::Func(f_even) = inst.export(&store, "even").unwrap() else {
                    panic!()
                };
                let out = store.invoke(f_even, &[Value::I32(5)]);
                assert_eq!(store.active_local_slots, 0);
                pair.push((out.map_err(|e| (e.kind, e.message)), store.current_fuel));
            }
            assert_eq!(pair[0], pair[1], "tail_call fuel={fuel} mismatch");
        }

        // 3. Cooperative cancellation terminates infinite tail recursion cleanly.
        let cfg = Config {
            features,
            ..Config::default()
        };
        let engine = Engine::with_execution_policy(cfg, ExecutionPolicy::Register).unwrap();
        let module = engine.compile(&wasm).unwrap();
        let mut store = Store::new(&engine);
        let mut linker = Linker::new(&engine);
        let host_fn = store
            .create_host_func(
                crate::FuncType {
                    params: vec![ValType::I32],
                    results: vec![ValType::I32],
                },
                |args| Ok(args.to_vec()),
            )
            .unwrap();
        linker
            .define("env", "host_add", External::Func(host_fn))
            .unwrap();
        let inst = linker.instantiate(&mut store, &module).unwrap();
        let External::Func(f_inf) = inst.export(&store, "inf_tail").unwrap() else {
            panic!()
        };
        let cancel = store.cancellation_token();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(5));
            cancel.cancel();
        });
        let err = store.invoke(f_inf, &[Value::I32(0)]).unwrap_err();
        canceller.join().unwrap();
        assert_eq!(err.kind, ErrorKind::ResourceLimit);
        assert_eq!(store.active_local_slots, 0);
    }

    #[test]
    fn fast_register_all_236_simd_and_20_relaxed_simd_full_parity() {
        use std::collections::BTreeSet;

        fn push_sleb_i32(out: &mut Vec<u8>, mut v: i32) {
            loop {
                let b = (v & 0x7f) as u8;
                v >>= 7;
                if (v == 0 && (b & 0x40) == 0) || (v == -1 && (b & 0x40) != 0) {
                    out.push(b);
                    break;
                }
                out.push(b | 0x80);
            }
        }
        fn emit_simd_op(out: &mut Vec<u8>, op: u16, covered: &mut BTreeSet<u16>) {
            covered.insert(op);
            let sub = if op >= 0xfd00 {
                u32::from(op - 0xfd00)
            } else {
                u32::from(op - 0xfa00 + 0x100)
            };
            out.push(0xfd);
            leb_u32(out, sub);
        }
        fn push_v128(out: &mut Vec<u8>, bytes: [u8; 16], covered: &mut BTreeSet<u16>) {
            emit_simd_op(out, 0xfd0c, covered);
            out.extend_from_slice(&bytes);
        }
        fn load_v(out: &mut Vec<u8>, off: i32, covered: &mut BTreeSet<u16>) {
            out.push(0x41);
            push_sleb_i32(out, off);
            emit_simd_op(out, 0xfd00, covered);
            out.extend_from_slice(&[0, 0]);
        }
        fn store_v(out: &mut Vec<u8>, out_off: &mut i32, covered: &mut BTreeSet<u16>) {
            // Stack has [v128]; stash in local 1, store at *out_off, advance *out_off += 16.
            out.extend_from_slice(&[0x21, 1, 0x41]);
            push_sleb_i32(out, *out_off);
            out.extend_from_slice(&[0x20, 1]);
            emit_simd_op(out, 0xfd0b, covered);
            out.extend_from_slice(&[0, 0]);
            *out_off += 16;
        }

        let mut covered: BTreeSet<u16> = BTreeSet::new();
        let mut out_off: i32 = 512;

        // Types:
        // 0: (i32) -> (i32)
        // 1: (i32, v128) -> (v128, i32)
        // 2: (v128, i32) -> (v128)
        let types = vec![
            3, 0x60, 1, 0x7f, 1, 0x7f, // 0
            0x60, 2, 0x7f, 0x7b, 2, 0x7b, 0x7f, // 1
            0x60, 2, 0x7b, 0x7f, 1, 0x7b, // 2
        ];
        // Import 0: env::host_vec (v128, i32) -> (v128) (exercises live v128 bail + resume!)
        let mut imps = vec![1];
        for s in ["env", "host_vec"] {
            leb_u32(&mut imps, s.len() as u32);
            imps.extend_from_slice(s.as_bytes());
        }
        imps.extend_from_slice(&[0x00, 2]);

        // Defined functions (indices 1..=7):
        // 1 (type 0): f_mem_lanes_unary(seed) -> acc
        // 2 (type 0): f_bin_tern_relaxed(shift) -> end_off
        // 3 (type 1): vec_callee(tag, v) -> (v ^ const, tag + 1)
        // 4 (type 1): vec_tail_step(n, v) -> (v', n') (tail calls self & indirect!)
        // 5 (type 0): cfg_calls_globals_bail(x) -> i32 (v128 globals, select, multivalue block, call, return_call, host bail!)
        // 6 (type 0): simd_oob_trap(addr) -> i32
        // 7 (type 0): multi_mem64_simd(addr) -> i32
        let funcs = vec![7, 0, 0, 1, 1, 0, 0, 0];
        let tables = vec![1, 0x70, 0x00, 2]; // table 0: funcref min=2
        let elem = vec![1, 0x00, 0x41, 0x00, 0x0b, 2, 3, 4];
        // 3 memories: mem0 (32-bit, 2 pages), mem1 (32-bit, 1 page), mem2 (64-bit, 1 page)
        let mems = vec![3, 0x00, 2, 0x00, 1, 0x04, 1];
        // 1 mutable v128 global initialized to [1..=16]
        let mut globals = vec![1, 0x7b, 0x01, 0xfd, 0x0c];
        for i in 1u8..=16 {
            globals.push(i);
        }
        globals.push(0x0b);

        let mut exps = vec![6];
        for (name, kind, idx) in [
            ("memory", 2u8, 0u32),
            ("f_mem_lanes_unary", 0, 1),
            ("f_bin_tern_relaxed", 0, 2),
            ("cfg_calls_globals_bail", 0, 5),
            ("simd_oob_trap", 0, 6),
            ("multi_mem64_simd", 0, 7),
        ] {
            leb_u32(&mut exps, name.len() as u32);
            exps.extend_from_slice(name.as_bytes());
            exps.push(kind);
            leb_u32(&mut exps, idx);
        }

        // Active data segment initializing mem0[0..256] with rich int/float/signed/NaN patterns.
        let mut init_bytes = Vec::new();
        // 0: 16 bytes mixed signs
        init_bytes.extend_from_slice(&[
            0x00, 0x01, 0x7f, 0x80, 0xff, 0xfe, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
            0x42, 0xa5,
        ]);
        // 16: second integer pattern
        init_bytes.extend_from_slice(&[
            0x01, 0xff, 0x80, 0x7f, 0x00, 0x02, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
            0x99, 0xaa,
        ]);
        // 32: 4 f32 values [1.5, -2.25, 0.0, -0.0]
        for f in [1.5f32, -2.25, 0.0, -0.0] {
            init_bytes.extend_from_slice(&f.to_le_bytes());
        }
        // 48: 4 f32 values [3.25, 0.5, f32::INFINITY, 1e10]
        for f in [3.25f32, 0.5, f32::INFINITY, 1e10] {
            init_bytes.extend_from_slice(&f.to_le_bytes());
        }
        // 64: 2 f64 values [1.5, -3.75]
        for f in [1.5f64, -3.75] {
            init_bytes.extend_from_slice(&f.to_le_bytes());
        }
        // 80: 2 f64 values [0.25, -0.0]
        for f in [0.25f64, -0.0] {
            init_bytes.extend_from_slice(&f.to_le_bytes());
        }
        // 96: all-nonzero vector for all_true
        init_bytes.extend_from_slice(&[0x03; 16]);
        let mut data_sec = vec![1, 0x00, 0x41, 0x00, 0x0b];
        leb_u32(&mut data_sec, init_bytes.len() as u32);
        data_sec.extend_from_slice(&init_bytes);

        // ---- Body 1: f_mem_lanes_unary(seed: i32) -> i32 (locals: 1:v128, 2:i32 acc) ----
        let mut b1 = Vec::new();
        let mix_i32 = |out: &mut Vec<u8>| {
            out.extend_from_slice(&[0x20, 2, 0x41, 31, 0x6c, 0x6a, 0x21, 2]);
        };
        // All 10 non-lane SIMD loads: 0xfd00..=0xfd0a, 0xfd5c, 0xfd5d
        for op in [
            0xfd00u16, 0xfd01, 0xfd02, 0xfd03, 0xfd04, 0xfd05, 0xfd06, 0xfd07, 0xfd08, 0xfd09,
            0xfd0a, 0xfd5c, 0xfd5d,
        ] {
            b1.extend_from_slice(&[0x41, 0]);
            emit_simd_op(&mut b1, op, &mut covered);
            b1.extend_from_slice(&[0, 0]);
            store_v(&mut b1, &mut out_off, &mut covered);
        }
        // All 4 load*_lane (0xfd54..=0xfd57) and 4 store*_lane (0xfd58..=0xfd5b)
        for (op, lane) in [(0xfd54u16, 13u8), (0xfd55, 5), (0xfd56, 2), (0xfd57, 1)] {
            b1.extend_from_slice(&[0x41, 0]);
            load_v(&mut b1, 16, &mut covered);
            emit_simd_op(&mut b1, op, &mut covered);
            b1.extend_from_slice(&[0, 0, lane]);
            store_v(&mut b1, &mut out_off, &mut covered);
        }
        for (op, lane) in [(0xfd58u16, 7u8), (0xfd59, 3), (0xfd5a, 1), (0xfd5b, 0)] {
            b1.push(0x41);
            push_sleb_i32(&mut b1, out_off);
            load_v(&mut b1, 0, &mut covered);
            emit_simd_op(&mut b1, op, &mut covered);
            b1.extend_from_slice(&[0, 0, lane]);
            out_off += 16;
        }
        // Shuffle (0xfd0d) & Swizzle (0xfd0e)
        load_v(&mut b1, 0, &mut covered);
        load_v(&mut b1, 16, &mut covered);
        emit_simd_op(&mut b1, 0xfd0d, &mut covered);
        b1.extend_from_slice(&[0, 17, 2, 19, 4, 21, 6, 23, 8, 25, 10, 27, 12, 29, 14, 31]);
        store_v(&mut b1, &mut out_off, &mut covered);
        load_v(&mut b1, 0, &mut covered);
        load_v(&mut b1, 16, &mut covered);
        emit_simd_op(&mut b1, 0xfd0e, &mut covered);
        store_v(&mut b1, &mut out_off, &mut covered);
        // All 6 splats (0xfd0f..=0xfd14)
        for op in [0xfd0fu16, 0xfd10, 0xfd11] {
            b1.extend_from_slice(&[0x20, 0]);
            emit_simd_op(&mut b1, op, &mut covered);
            store_v(&mut b1, &mut out_off, &mut covered);
        }
        b1.extend_from_slice(&[0x20, 0, 0xac]); // i64.extend_i32_s
        emit_simd_op(&mut b1, 0xfd12, &mut covered);
        store_v(&mut b1, &mut out_off, &mut covered);
        b1.extend_from_slice(&[0x20, 0, 0xb2]); // f32.convert_i32_s
        emit_simd_op(&mut b1, 0xfd13, &mut covered);
        store_v(&mut b1, &mut out_off, &mut covered);
        b1.extend_from_slice(&[0x20, 0, 0xb7]); // f64.convert_i32_s
        emit_simd_op(&mut b1, 0xfd14, &mut covered);
        store_v(&mut b1, &mut out_off, &mut covered);
        // All 8 extract_lane (0xfd15,16,18,19,1b,1d,1f,21)
        for (op, lane, cast) in [
            (0xfd15u16, 3u8, 0u8),
            (0xfd16, 4, 0),
            (0xfd18, 2, 0),
            (0xfd19, 5, 0),
            (0xfd1b, 3, 0),
            (0xfd1d, 1, 1), // i64 -> i32.wrap_i64
            (0xfd1f, 2, 2), // f32 -> i32.reinterpret_f32
            (0xfd21, 0, 3), // f64 -> i64.reinterpret_f64 + wrap
        ] {
            load_v(&mut b1, 0, &mut covered);
            emit_simd_op(&mut b1, op, &mut covered);
            b1.push(lane);
            match cast {
                1 => b1.push(0xa7),
                2 => b1.push(0xbc),
                3 => b1.extend_from_slice(&[0xbd, 0xa7]),
                _ => {}
            }
            mix_i32(&mut b1);
        }
        // All 6 replace_lane (0xfd17,1a,1c,1e,20,22)
        for (op, lane, kind) in [
            (0xfd17u16, 9u8, 0u8),
            (0xfd1a, 4, 0),
            (0xfd1c, 2, 0),
            (0xfd1e, 1, 1),
            (0xfd20, 3, 2),
            (0xfd22, 0, 3),
        ] {
            load_v(&mut b1, 0, &mut covered);
            match kind {
                0 => b1.extend_from_slice(&[0x20, 0]),
                1 => b1.extend_from_slice(&[0x20, 0, 0xac]),
                2 => b1.extend_from_slice(&[0x20, 0, 0xb2]),
                _ => b1.extend_from_slice(&[0x20, 0, 0xb7]),
            }
            emit_simd_op(&mut b1, op, &mut covered);
            b1.push(lane);
            store_v(&mut b1, &mut out_off, &mut covered);
        }
        // All 9 V128 -> I32 ops
        for op in [
            0xfd53u16, 0xfd63, 0xfd64, 0xfd83, 0xfd84, 0xfda3, 0xfda4, 0xfdc3, 0xfdc4,
        ] {
            for off in [0, 96] {
                load_v(&mut b1, off, &mut covered);
                emit_simd_op(&mut b1, op, &mut covered);
                mix_i32(&mut b1);
            }
        }
        // All 50 unary V128 -> V128 + 4 relaxed unary (0xfa01..=0xfa04)
        let unary_ops: &[u16] = &[
            0xfd4d, 0xfd5e, 0xfd5f, 0xfd60, 0xfd61, 0xfd62, 0xfd67, 0xfd68, 0xfd69, 0xfd6a, 0xfd74,
            0xfd75, 0xfd7a, 0xfd7c, 0xfd7d, 0xfd7e, 0xfd7f, 0xfd80, 0xfd81, 0xfd87, 0xfd88, 0xfd89,
            0xfd8a, 0xfd94, 0xfda0, 0xfda1, 0xfda7, 0xfda8, 0xfda9, 0xfdaa, 0xfdc0, 0xfdc1, 0xfdc7,
            0xfdc8, 0xfdc9, 0xfdca, 0xfde0, 0xfde1, 0xfde3, 0xfdec, 0xfded, 0xfdef, 0xfdf8, 0xfdf9,
            0xfdfa, 0xfdfb, 0xfdfc, 0xfdfd, 0xfdfe, 0xfdff, 0xfa01, 0xfa02, 0xfa03, 0xfa04,
        ];
        for &op in unary_ops {
            let src_off_mem = match op {
                0xfd5e | 0xfd74 | 0xfd75 | 0xfd7a | 0xfd94 | 0xfdec | 0xfded | 0xfdef | 0xfdfc
                | 0xfdfd | 0xfa03 | 0xfa04 => 64,
                0xfd5f
                | 0xfd67..=0xfd6a
                | 0xfde0
                | 0xfde1
                | 0xfde3
                | 0xfdf8
                | 0xfdf9
                | 0xfa01
                | 0xfa02 => 32,
                _ => 0,
            };
            load_v(&mut b1, src_off_mem, &mut covered);
            emit_simd_op(&mut b1, op, &mut covered);
            store_v(&mut b1, &mut out_off, &mut covered);
        }
        b1.extend_from_slice(&[0x20, 2]);

        // ---- Body 2: f_bin_tern_relaxed(shift: i32) -> i32 (locals: 1:v128) ----
        let mut b2 = Vec::new();
        // All 12 shifts
        for op in [
            0xfd6bu16, 0xfd6c, 0xfd6d, 0xfd8b, 0xfd8c, 0xfd8d, 0xfdab, 0xfdac, 0xfdad, 0xfdcb,
            0xfdcc, 0xfdcd,
        ] {
            load_v(&mut b2, 0, &mut covered);
            b2.extend_from_slice(&[0x20, 0]);
            emit_simd_op(&mut b2, op, &mut covered);
            store_v(&mut b2, &mut out_off, &mut covered);
        }
        // All 119 binary V128xV128 + 7 relaxed binary (0xfa00, 0xfa0d..=0xfa12)
        let bin_ops: &[u16] = &[
            0xfd23, 0xfd24, 0xfd25, 0xfd26, 0xfd27, 0xfd28, 0xfd29, 0xfd2a, 0xfd2b, 0xfd2c, 0xfd2d,
            0xfd2e, 0xfd2f, 0xfd30, 0xfd31, 0xfd32, 0xfd33, 0xfd34, 0xfd35, 0xfd36, 0xfd37, 0xfd38,
            0xfd39, 0xfd3a, 0xfd3b, 0xfd3c, 0xfd3d, 0xfd3e, 0xfd3f, 0xfd40, 0xfd41, 0xfd42, 0xfd43,
            0xfd44, 0xfd45, 0xfd46, 0xfd47, 0xfd48, 0xfd49, 0xfd4a, 0xfd4b, 0xfd4c, 0xfd4e, 0xfd4f,
            0xfd50, 0xfd51, 0xfd65, 0xfd66, 0xfd6e, 0xfd6f, 0xfd70, 0xfd71, 0xfd72, 0xfd73, 0xfd76,
            0xfd77, 0xfd78, 0xfd79, 0xfd7b, 0xfd82, 0xfd85, 0xfd86, 0xfd8e, 0xfd8f, 0xfd90, 0xfd91,
            0xfd92, 0xfd93, 0xfd95, 0xfd96, 0xfd97, 0xfd98, 0xfd99, 0xfd9b, 0xfd9c, 0xfd9d, 0xfd9e,
            0xfd9f, 0xfdae, 0xfdb1, 0xfdb5, 0xfdb6, 0xfdb7, 0xfdb8, 0xfdb9, 0xfdba, 0xfdbc, 0xfdbd,
            0xfdbe, 0xfdbf, 0xfdce, 0xfdd1, 0xfdd5, 0xfdd6, 0xfdd7, 0xfdd8, 0xfdd9, 0xfdda, 0xfddb,
            0xfddc, 0xfddd, 0xfdde, 0xfddf, 0xfde4, 0xfde5, 0xfde6, 0xfde7, 0xfde8, 0xfde9, 0xfdea,
            0xfdeb, 0xfdf0, 0xfdf1, 0xfdf2, 0xfdf3, 0xfdf4, 0xfdf5, 0xfdf6, 0xfdf7, 0xfa00, 0xfa0d,
            0xfa0e, 0xfa0f, 0xfa10, 0xfa11, 0xfa12,
        ];
        for &op in bin_ops {
            let (a_off, b_off) = match op {
                0xfd41..=0xfd46 | 0xfde4..=0xfdeb | 0xfa0d | 0xfa0e => (32, 48),
                0xfd47..=0xfd4c | 0xfdf0..=0xfdf7 | 0xfa0f | 0xfa10 => (64, 80),
                _ => (0, 16),
            };
            load_v(&mut b2, a_off, &mut covered);
            load_v(&mut b2, b_off, &mut covered);
            emit_simd_op(&mut b2, op, &mut covered);
            store_v(&mut b2, &mut out_off, &mut covered);
        }
        // All 10 ternary ops: 0xfd52 + 0xfa05..=0xfa0c + 0xfa13
        for op in [
            0xfd52u16, 0xfa05, 0xfa06, 0xfa07, 0xfa08, 0xfa09, 0xfa0a, 0xfa0b, 0xfa0c, 0xfa13,
        ] {
            let (a_off, b_off, c_off) = match op {
                0xfa05 | 0xfa06 => (32, 48, 32),
                0xfa07 | 0xfa08 => (64, 80, 64),
                _ => (0, 16, 96),
            };
            load_v(&mut b2, a_off, &mut covered);
            load_v(&mut b2, b_off, &mut covered);
            load_v(&mut b2, c_off, &mut covered);
            emit_simd_op(&mut b2, op, &mut covered);
            store_v(&mut b2, &mut out_off, &mut covered);
        }
        b2.push(0x41);
        push_sleb_i32(&mut b2, out_off);

        // ---- Body 3 (idx 3): vec_callee(tag: i32, v: v128) -> (v128, i32) ----
        let mut b3 = Vec::new();
        b3.extend_from_slice(&[0x20, 1]); // v
        push_v128(&mut b3, [0x5a; 16], &mut covered);
        emit_simd_op(&mut b3, 0xfd51, &mut covered); // v128.xor
        b3.extend_from_slice(&[0x20, 0, 0x41, 1, 0x6a]); // tag + 1

        // Verify ALL 236 SIMD + 20 Relaxed SIMD opcodes = 256 total!
        assert_eq!(
            covered.len(),
            256,
            "expected all 236 SIMD + 20 relaxed SIMD opcodes covered, got {}",
            covered.len()
        );

        // ---- Body 4 (idx 4): vec_tail_step(n: i32, v: v128) -> (v128, i32) ----
        // if n == 0 { return (v, 77) } else { return_call vec_tail_step(n - 1, v + splat(n)) }
        let mut b4 = Vec::new();
        b4.extend_from_slice(&[0x20, 0, 0x45, 0x04, 0x40]);
        b4.extend_from_slice(&[0x20, 1, 0x41, 0xcd, 0x00, 0x0f, 0x0b]); // return (v, 77)
        b4.extend_from_slice(&[0x20, 0, 0x41, 1, 0x6b]); // n - 1
        b4.extend_from_slice(&[0x20, 1, 0x20, 0]);
        emit_simd_op(&mut b4, 0xfd11, &mut covered); // i32x4.splat(n)
        emit_simd_op(&mut b4, 0xfdae, &mut covered); // i32x4.add
        b4.extend_from_slice(&[0x12, 4]); // return_call vec_tail_step

        // ---- Body 5 (idx 5): cfg_calls_globals_bail(x: i32) -> i32 ----
        // Mixes v128 global.get/set, v128 select (0x1b & 0x1c), multivalue block (i32, v128)->(v128, i32),
        // direct call, call_indirect, tail-call callee, AND host_vec bail with live v128 local + live v128 stack value!
        let mut b5 = Vec::new();
        // local 1 (v128) = global.get 0
        b5.extend_from_slice(&[0x23, 0, 0x22, 1]);
        // push [x, v1] into multivalue block type 1: (i32, v128) -> (v128, i32)
        b5.extend_from_slice(&[0x20, 0, 0x20, 1, 0x02, 1]);
        // Inside block: call_indirect type 1 table 0 (slot 0 = vec_callee)
        b5.extend_from_slice(&[0x41, 0, 0x11, 1, 0]);
        b5.push(0x0b); // end block -> stack has [v1, v_out, tag_out]
        b5.extend_from_slice(&[0x21, 2, 0x21, 3]); // local2(i32)=tag_out, local3(v128)=v_out; stack still has [v1]!
        // Call vec_tail_step(4, v_out) -> (v_tail, 77)
        b5.extend_from_slice(&[0x41, 4, 0x20, 3, 0x10, 4]);
        b5.extend_from_slice(&[0x20, 2, 0x6a, 0x21, 2]); // local2 += 77; stack has [v1, v_tail]
        // Typed select (0x1c [v128]) choosing between v1 and v_tail based on (x & 1)
        b5.extend_from_slice(&[0x20, 0, 0x41, 1, 0x71, 0x1c, 1, 0x7b]);
        // Keep a live v128 on stack AND call host_vec(selected, x) -> forces bail with live v128 local + stack!
        b5.extend_from_slice(&[0x22, 3, 0x20, 3, 0x20, 0, 0x10, 0]); // stack: [selected, host_res]
        // Untyped select (0x1b) choosing with (x & 2)
        b5.extend_from_slice(&[0x20, 0, 0x41, 2, 0x71, 0x1b]);
        // Store back into global 0 and extract lane 0 + lane 3 + local2
        b5.extend_from_slice(&[0x22, 1, 0x24, 0]);
        b5.extend_from_slice(&[0x20, 1, 0xfd, 0x1b, 0]); // i32x4.extract_lane 0
        b5.extend_from_slice(&[0x20, 1, 0xfd, 0x1b, 3, 0x6a]); // + lane 3
        b5.extend_from_slice(&[0x20, 2, 0x6a]); // + local2

        // ---- Body 6 (idx 6): simd_oob_trap(addr: i32) -> i32 ----
        let mut b6 = Vec::new();
        // Write 1 word at mem0[0] before potentially trapping on v128.load(addr)
        b6.extend_from_slice(&[0x41, 0, 0x41, 42, 0x36, 2, 0]);
        b6.extend_from_slice(&[0x20, 0, 0xfd, 0x00, 0, 0, 0xfd, 0x1b, 0]);

        // ---- Body 7 (idx 7): multi_mem64_simd(addr: i32) -> i32 ----
        // Store v128 to mem1[addr], load back, store to mem2[addr as i64], load back!
        let mut b7 = Vec::new();
        b7.extend_from_slice(&[0x20, 0]);
        load_v(&mut b7, 0, &mut covered);
        // v128.store align=0 | 0x40 (has memidx), memidx=1, offset=0
        b7.extend_from_slice(&[0xfd, 0x0b, 0x40, 1, 0]);
        // mem2 (i64 addr): store what we load from mem1
        b7.extend_from_slice(&[0x20, 0, 0xad]); // i64.extend_i32_u
        b7.extend_from_slice(&[0x20, 0, 0xfd, 0x00, 0x40, 1, 0]);
        b7.extend_from_slice(&[0xfd, 0x0b, 0x40, 2, 0]);
        // Load back from mem2 and extract lane 2
        b7.extend_from_slice(&[0x20, 0, 0xad, 0xfd, 0x00, 0x40, 2, 0, 0xfd, 0x1b, 2]);

        let mut code = vec![7];
        code.extend_from_slice(&body(&[(1, 0x7b), (1, 0x7f)], &b1));
        code.extend_from_slice(&body(&[(1, 0x7b)], &b2));
        code.extend_from_slice(&body(&[], &b3));
        code.extend_from_slice(&body(&[], &b4));
        code.extend_from_slice(&body(&[(1, 0x7b), (1, 0x7f), (1, 0x7b)], &b5));
        code.extend_from_slice(&body(&[], &b6));
        code.extend_from_slice(&body(&[], &b7));

        let wasm = wasm_mod(&[
            sec(1, &types),
            sec(2, &imps),
            sec(3, &funcs),
            sec(4, &tables),
            sec(5, &mems),
            sec(6, &globals),
            sec(7, &exps),
            sec(9, &elem),
            sec(10, &code),
            sec(11, &data_sec),
        ]);

        let make_inst = |policy: ExecutionPolicy, fuel: Option<u64>| {
            let mut features = FeatureSet::all_phase3_core();
            features.tail_call = true;
            features.relaxed_simd = true;
            features.multi_memory = true;
            features.memory64 = true;
            let mut cfg = Config {
                features,
                ..Config::default()
            };
            cfg.limits.fuel = fuel;
            let engine = Engine::with_execution_policy(cfg, policy).unwrap();
            let module = engine.compile(&wasm).unwrap();
            if policy == ExecutionPolicy::Register {
                for bi in 0..7 {
                    assert!(
                        is_admitted(&module, bi),
                        "SIMD body {bi} must be natively admitted by register-executor"
                    );
                }
            }
            let mut store = Store::new(&engine);
            let mut linker = Linker::new(&engine);
            let host_fn = store
                .create_host_func(
                    crate::FuncType {
                        params: vec![ValType::V128, ValType::I32],
                        results: vec![ValType::V128],
                    },
                    |args| {
                        let (Value::V128(mut v), Value::I32(k)) = (args[0], args[1]) else {
                            panic!()
                        };
                        for b in &mut v.0 {
                            *b = b.wrapping_add(k as u8);
                        }
                        Ok(vec![Value::V128(v)])
                    },
                )
                .unwrap();
            linker
                .define("env", "host_vec", External::Func(host_fn))
                .unwrap();
            let inst = linker.instantiate(&mut store, &module).unwrap();
            (store, inst)
        };

        // 1. Run ALL 256 SIMD + relaxed SIMD ops, control flow, host bail, multi-memory, memory64, and OOB traps!
        let (mut sd, id) = make_inst(ExecutionPolicy::Decoded, None);
        let (mut sr, ir) = make_inst(ExecutionPolicy::Register, None);
        for (name, arg) in [
            ("f_mem_lanes_unary", 0x1234_5678i32),
            ("f_bin_tern_relaxed", 5),
            ("cfg_calls_globals_bail", 3),
            ("cfg_calls_globals_bail", 2),
            ("multi_mem64_simd", 64),
            ("simd_oob_trap", 16),
            ("simd_oob_trap", 2 * 65536 - 8), // traps OOB!
        ] {
            let External::Func(fd) = id.export(&sd, name).unwrap() else {
                panic!()
            };
            let External::Func(fr) = ir.export(&sr, name).unwrap() else {
                panic!()
            };
            let od = sd.invoke(fd, &[Value::I32(arg)]);
            let or = sr.invoke(fr, &[Value::I32(arg)]);
            match (&od, &or) {
                (Ok(a), Ok(b)) => assert_eq!(a, b, "{name}({arg}) mismatch"),
                (Err(a), Err(b)) => {
                    assert_eq!(
                        (a.kind, a.trap_code, &a.message),
                        (b.kind, b.trap_code, &b.message)
                    )
                }
                _ => panic!("{name}({arg}): Decoded={od:?} != Register={or:?}"),
            }
            assert_eq!(sr.active_local_slots, 0);
        }
        let External::Memory(md) = id.export(&sd, "memory").unwrap() else {
            panic!()
        };
        let External::Memory(mr) = ir.export(&sr, "memory").unwrap() else {
            panic!()
        };
        assert_eq!(
            sd.read_memory(md, 0, out_off as usize).unwrap(),
            sr.read_memory(mr, 0, out_off as usize).unwrap(),
            "all-256 SIMD output memory buffer must match Decoded byte-for-byte!"
        );

        // 2. Exact source-level fuel prefix sweep across cfg_calls_globals_bail & simd_oob_trap!
        for fuel in 0..=75u64 {
            let (mut sd, id) = make_inst(ExecutionPolicy::Decoded, Some(fuel));
            let (mut sr, ir) = make_inst(ExecutionPolicy::Register, Some(fuel));
            let External::Func(fd) = id.export(&sd, "cfg_calls_globals_bail").unwrap() else {
                panic!()
            };
            let External::Func(fr) = ir.export(&sr, "cfg_calls_globals_bail").unwrap() else {
                panic!()
            };
            let od = sd.invoke(fd, &[Value::I32(5)]);
            let or = sr.invoke(fr, &[Value::I32(5)]);
            match (&od, &or) {
                (Ok(a), Ok(b)) => assert_eq!(a, b, "simd fuel={fuel}"),
                (Err(a), Err(b)) => {
                    assert_eq!(
                        (a.kind, &a.message),
                        (b.kind, &b.message),
                        "simd fuel={fuel}"
                    )
                }
                _ => panic!("simd fuel={fuel}: Decoded={od:?} != Register={or:?}"),
            }
            assert_eq!(
                sd.current_fuel, sr.current_fuel,
                "simd fuel={fuel} rem mismatch"
            );
            assert_eq!(sr.active_local_slots, 0);
        }
        // OOB SIMD load mid-block trap must leave exact earlier memory write and exact fuel!
        {
            let (mut sd, id) = make_inst(ExecutionPolicy::Decoded, Some(40));
            let (mut sr, ir) = make_inst(ExecutionPolicy::Register, Some(40));
            let External::Func(fd) = id.export(&sd, "simd_oob_trap").unwrap() else {
                panic!()
            };
            let External::Func(fr) = ir.export(&sr, "simd_oob_trap").unwrap() else {
                panic!()
            };
            let External::Memory(md) = id.export(&sd, "memory").unwrap() else {
                panic!()
            };
            let External::Memory(mr) = ir.export(&sr, "memory").unwrap() else {
                panic!()
            };
            let ed = sd.invoke(fd, &[Value::I32(2 * 65536 - 4)]).unwrap_err();
            let er = sr.invoke(fr, &[Value::I32(2 * 65536 - 4)]).unwrap_err();
            assert_eq!(
                (ed.kind, ed.trap_code, ed.message),
                (er.kind, er.trap_code, er.message)
            );
            assert_eq!(sd.current_fuel, sr.current_fuel);
            assert_eq!(
                sd.read_memory(md, 0, 16).unwrap(),
                sr.read_memory(mr, 0, 16).unwrap()
            );
        }
    }

    #[test]
    fn fast_register_deterministic_differential_fuzzer() {
        // Fixed-seed deterministic PRNG generating 80 valid random Wasm modules mixing:
        // scalar + v128 locals, nested block/if/else/loop/br/br_if/br_table with multivalue
        // (i32, v128, i64) results, memory loads/stores/copy/fill, SIMD + relaxed SIMD,
        // wide-arithmetic, direct calls, call_indirect, return_call, select, and traps.
        struct Rng(u64);
        impl Rng {
            fn next(&mut self) -> u64 {
                let mut x = self.0;
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                self.0 = x;
                x
            }
            fn pick(&mut self, n: usize) -> usize {
                (self.next() as usize) % n
            }
        }
        fn push_sleb(out: &mut Vec<u8>, mut v: i32) {
            loop {
                let b = (v & 0x7f) as u8;
                v >>= 7;
                if (v == 0 && (b & 0x40) == 0) || (v == -1 && (b & 0x40) != 0) {
                    out.push(b);
                    break;
                }
                out.push(b | 0x80);
            }
        }
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let mut features = FeatureSet::all_phase3_core();
        features.tail_call = true;
        features.relaxed_simd = true;
        features.wide_arithmetic = true;

        for seed_idx in 0..80usize {
            // Fixed module shape with random instruction sequences:
            // - type 0: (i32, i64) -> (i32)
            // - type 1: (i32, v128) -> (v128, i32)
            // - type 2: (i32) -> (i32)
            // - import 0: env::host_mix (i32) -> (i32)
            // - f1 (type 1): helper(a: i32, v: v128) -> (v128, i32)
            // - f2 (type 1): tail_helper(k: i32, v: v128) -> (v128, i32)
            // - f3 (type 0): main(x: i32, y: i64) -> i32  [locals: 2:i32, 3:i64, 4:v128, 5:v128, 6:f64]
            let types = vec![
                3, 0x60, 2, 0x7f, 0x7e, 1, 0x7f, // 0
                0x60, 2, 0x7f, 0x7b, 2, 0x7b, 0x7f, // 1
                0x60, 1, 0x7f, 1, 0x7f, // 2
            ];
            let mut imps = vec![1];
            for s in ["env", "host_mix"] {
                leb_u32(&mut imps, s.len() as u32);
                imps.extend_from_slice(s.as_bytes());
            }
            imps.extend_from_slice(&[0x00, 2]);
            let funcs = vec![3, 1, 1, 0];
            let tbl = vec![1, 0x70, 0x00, 2];
            let elem = vec![1, 0x00, 0x41, 0x00, 0x0b, 2, 1, 2];
            let mem = vec![1, 0x01, 2, 4];
            let mut globals = vec![2, 0x7f, 0x01, 0x41, 7, 0x0b, 0x7b, 0x01, 0xfd, 0x0c];
            for _ in 0..16 {
                globals.push(rng.next() as u8);
            }
            globals.push(0x0b);
            let mut exps = vec![2];
            for (name, kind, idx) in [("memory", 2u8, 0u32), ("main", 0, 3)] {
                leb_u32(&mut exps, name.len() as u32);
                exps.extend_from_slice(name.as_bytes());
                exps.push(kind);
                leb_u32(&mut exps, idx);
            }

            // f1: helper(a, v) -> (v op splat(a), a ^ lane0)
            let mut f1 = Vec::new();
            f1.extend_from_slice(&[0x20, 1, 0x20, 0, 0xfd, 0x11]); // v, splat(a)
            let vop = [0xfdae_u16, 0xfdb1, 0xfdb5, 0xfd4e, 0xfd50, 0xfd51][rng.pick(6)];
            f1.push(0xfd);
            leb_u32(&mut f1, u32::from(vop - 0xfd00));
            f1.extend_from_slice(&[0x22, 1, 0x20, 0, 0x20, 1, 0xfd, 0x1b, 0, 0x73]);

            // f2: tail_helper(k, v) -> if (k & 3) == 0 { return (v, k) } else { return_call f1(k - 1, v) }
            let mut f2 = Vec::new();
            f2.extend_from_slice(&[
                0x20, 0, 0x41, 3, 0x71, 0x45, 0x04, 0x40, 0x20, 1, 0x20, 0, 0x0f, 0x0b, 0x20, 0,
                0x41, 1, 0x6b, 0x20, 1, 0x12, 1,
            ]);

            // f3: main(x:0, y:1; locals: i32:2, i64:3, v128:4, v128:5, f64:6) -> i32
            let mut f3 = Vec::new();
            // Seed v128 locals 4 & 5 from params
            f3.extend_from_slice(&[0x20, 0, 0xfd, 0x11, 0x21, 4]);
            f3.extend_from_slice(&[0x20, 1, 0xfd, 0x12, 0x21, 5]);
            f3.extend_from_slice(&[0x20, 0, 0x21, 2, 0x20, 1, 0x21, 3]);
            f3.extend_from_slice(&[0x20, 0, 0xb7, 0x21, 6]);

            // Emit 14..24 random well-typed operations
            let steps = 14 + rng.pick(11);
            for _ in 0..steps {
                match rng.pick(15) {
                    0 => {
                        // i32 ALU (including occasional div/rem that might trap if rhs==0)
                        f3.extend_from_slice(&[0x20, 2, 0x20, 0]);
                        let op = [
                            0x6a, 0x6b, 0x6c, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x6d,
                            0x70,
                        ][rng.pick(13)];
                        f3.push(op);
                        f3.extend_from_slice(&[0x21, 2]);
                    }
                    1 => {
                        // i64 wide-arithmetic (add128/sub128/mul_wide_s/mul_wide_u)
                        match rng.pick(4) {
                            0 | 1 => {
                                f3.extend_from_slice(&[0x20, 3, 0x20, 1, 0x20, 1, 0x20, 3]);
                                f3.extend_from_slice(&[
                                    0xfc,
                                    if rng.pick(2) == 0 { 0x13 } else { 0x14 },
                                ]);
                                f3.extend_from_slice(&[0x85, 0x21, 3]);
                            }
                            k => {
                                f3.extend_from_slice(&[
                                    0x20,
                                    3,
                                    0x20,
                                    1,
                                    0xfc,
                                    (0x15 + (k - 2)) as u8,
                                ]);
                                f3.extend_from_slice(&[0x85, 0x21, 3]);
                            }
                        }
                    }
                    2 => {
                        // v128 binary / relaxed op into local 4 or 5
                        f3.extend_from_slice(&[0x20, 4, 0x20, 5]);
                        let ops = [
                            0xfd0eu16, 0xfd4e, 0xfd4f, 0xfd50, 0xfd51, 0xfd6e, 0xfd70, 0xfd82,
                            0xfdae, 0xfdb1, 0xfdb5, 0xfdce, 0xfde4, 0xfdf0, 0xfa00, 0xfa11, 0xfa12,
                        ];
                        let op = ops[rng.pick(ops.len())];
                        let sub = if op >= 0xfd00 {
                            u32::from(op - 0xfd00)
                        } else {
                            u32::from(op - 0xfa00 + 0x100)
                        };
                        f3.push(0xfd);
                        leb_u32(&mut f3, sub);
                        f3.extend_from_slice(&[0x21, if rng.pick(2) == 0 { 4 } else { 5 }]);
                    }
                    3 => {
                        // v128 ternary (bitselect / relaxed_madd / relaxed_dot_add)
                        f3.extend_from_slice(&[0x20, 4, 0x20, 5, 0x23, 1]);
                        let op = [0xfd52u16, 0xfa05, 0xfa07, 0xfa0a, 0xfa13][rng.pick(5)];
                        let sub = if op >= 0xfd00 {
                            u32::from(op - 0xfd00)
                        } else {
                            u32::from(op - 0xfa00 + 0x100)
                        };
                        f3.push(0xfd);
                        leb_u32(&mut f3, sub);
                        f3.extend_from_slice(&[0x21, 4]);
                    }
                    4 => {
                        // v128 shuffle with random 32-entry lane mask
                        f3.extend_from_slice(&[0x20, 4, 0x20, 5, 0xfd, 0x0d]);
                        for _ in 0..16 {
                            f3.push((rng.next() & 31) as u8);
                        }
                        f3.extend_from_slice(&[0x21, 5]);
                    }
                    5 => {
                        // v128.store + v128.load at bounded (or occasionally cross-page/OOB!) address
                        let base = if rng.pick(12) == 0 {
                            65530 // cross-page!
                        } else {
                            (rng.pick(64) * 16) as i32
                        };
                        f3.push(0x41);
                        push_sleb(&mut f3, base);
                        f3.extend_from_slice(&[0x20, 4, 0xfd, 0x0b, 0, 0]);
                        f3.push(0x41);
                        push_sleb(&mut f3, base);
                        f3.extend_from_slice(&[0xfd, 0x00, 0, 0, 0x21, 5]);
                    }
                    6 => {
                        // v128.load32_lane + v128.store32_lane
                        let off = (rng.pick(32) * 4) as i32;
                        let lane = rng.pick(4) as u8;
                        f3.push(0x41);
                        push_sleb(&mut f3, off);
                        f3.extend_from_slice(&[0x20, 4, 0xfd, 0x5a, 0, 0, lane]);
                        f3.push(0x41);
                        push_sleb(&mut f3, off);
                        f3.extend_from_slice(&[0x20, 5, 0xfd, 0x56, 0, 0, lane, 0x21, 5]);
                    }
                    7 => {
                        // Multivalue if/else producing (v128, i32) with mixed stack prefix!
                        f3.extend_from_slice(&[0x20, 2, 0x20, 4, 0x20, 2, 0x41, 1, 0x71]);
                        f3.extend_from_slice(&[0x04, 1]); // if (i32, v128)->(v128, i32)
                        f3.extend_from_slice(&[0x10, 1]); // direct call helper
                        f3.push(0x05);
                        f3.extend_from_slice(&[0x41, (rng.pick(2)) as u8, 0x11, 1, 0]); // call_indirect helper or tail_helper
                        f3.push(0x0b);
                        f3.extend_from_slice(&[0x21, 2, 0x21, 4]);
                    }
                    8 => {
                        // Counted loop mutating local 4 (v128) via global 0 trip counter
                        let trips = (1 + rng.pick(4)) as i32;
                        f3.push(0x41);
                        push_sleb(&mut f3, trips);
                        f3.extend_from_slice(&[
                            0x24, 0, // global 0 = trips
                            0x03, 0x40, 0x20, 4, 0x20, 5, 0xfd, 0xae, 0x01, 0x21, 4, 0x23, 0, 0x41,
                            1, 0x6b, 0x24, 0, 0x23, 0, 0x0d, 0, 0x0b,
                        ]);
                    }
                    9 => {
                        // br_table over 3 nested blocks returning v128!
                        f3.extend_from_slice(&[
                            0x02, 0x7b, 0x02, 0x7b, 0x02, 0x7b, 0x20, 4, 0x20, 2, 0x41, 3, 0x71,
                            0x0e, 2, 0, 1, 2, 0x0b, 0x20, 5, 0xfd, 0x51, 0x0b, 0x23, 1, 0xfd, 0x50,
                            0x0b, 0x21, 4,
                        ]);
                    }
                    10 => {
                        // select on v128 & i32 + global v128 update
                        f3.extend_from_slice(&[
                            0x20, 4, 0x20, 5, 0x20, 2, 0x41, 2, 0x71, 0x1b, 0x22, 4, 0x24, 1,
                        ]);
                    }
                    11 => {
                        // scalar memory load/store + memory.fill / memory.copy
                        let off = (rng.pick(32) * 4) as i32;
                        f3.push(0x41);
                        push_sleb(&mut f3, off);
                        f3.extend_from_slice(&[0x20, 2, 0x36, 2, 0]);
                        f3.push(0x41);
                        push_sleb(&mut f3, off + 64);
                        f3.push(0x41);
                        push_sleb(&mut f3, off);
                        f3.extend_from_slice(&[0x41, 8, 0xfc, 0x0a, 0, 0]);
                        f3.push(0x41);
                        push_sleb(&mut f3, off + 64);
                        f3.extend_from_slice(&[0x28, 2, 0, 0x20, 2, 0x73, 0x21, 2]);
                    }
                    12 => {
                        // Host call bail with live v128 on stack AND in locals!
                        f3.extend_from_slice(&[
                            0x20, 4, // live v128 on stack across host call!
                            0x20, 2, 0x10, 0, // call host_mix(l2) -> bail & resume!
                            0x21, 2, // l2 = res
                            0x21, 5, // l5 = live v128 from stack!
                        ]);
                    }
                    13 => {
                        // f64 arithmetic + compare
                        f3.extend_from_slice(&[
                            0x20, 6, 0x20, 3, 0xb9, 0xa0, 0x22, 6, 0x44, 0, 0, 0, 0, 0, 0, 0xf0,
                            0x3f, 0x63, 0x20, 2, 0x6a, 0x21, 2,
                        ]);
                    }
                    _ => {
                        // v128 unary + extract_lane -> mix into l2
                        f3.extend_from_slice(&[
                            0x20,
                            4,
                            0xfd,
                            0x4d,
                            0x22,
                            4,
                            0xfd,
                            0x1b,
                            (rng.pick(4)) as u8,
                            0x20,
                            2,
                            0x73,
                            0x21,
                            2,
                        ]);
                    }
                }
            }
            // Final return: l2 ^ wrap(l3) ^ lane0(v4) ^ lane3(v5)
            f3.extend_from_slice(&[
                0x20, 2, 0x20, 3, 0xa7, 0x73, 0x20, 4, 0xfd, 0x1b, 0, 0x73, 0x20, 5, 0xfd, 0x1b, 3,
                0x73,
            ]);

            let mut code = vec![3];
            code.extend_from_slice(&body(&[], &f1));
            code.extend_from_slice(&body(&[], &f2));
            code.extend_from_slice(&body(&[(1, 0x7f), (1, 0x7e), (2, 0x7b), (1, 0x7c)], &f3));

            let wasm = wasm_mod(&[
                sec(1, &types),
                sec(2, &imps),
                sec(3, &funcs),
                sec(4, &tbl),
                sec(5, &mem),
                sec(6, &globals),
                sec(7, &exps),
                sec(9, &elem),
                sec(10, &code),
            ]);

            let x_in = rng.next() as i32;
            let y_in = rng.next() as i64;
            for fuel in [None, Some(1), Some(12), Some(45)] {
                let mut res = Vec::new();
                for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                    let mut cfg = Config {
                        features,
                        ..Config::default()
                    };
                    cfg.limits.fuel = fuel;
                    let engine = Engine::with_execution_policy(cfg, policy).unwrap();
                    let module = engine.compile(&wasm).unwrap();
                    if policy == ExecutionPolicy::Register {
                        for bi in 0..3 {
                            assert!(is_admitted(&module, bi), "fuzz seed {seed_idx} body {bi}");
                        }
                    }
                    let mut store = Store::new(&engine);
                    let mut linker = Linker::new(&engine);
                    let hf = store
                        .create_host_func(
                            crate::FuncType {
                                params: vec![ValType::I32],
                                results: vec![ValType::I32],
                            },
                            |args| {
                                let Value::I32(v) = args[0] else { panic!() };
                                Ok(vec![Value::I32(v.wrapping_mul(17).wrapping_add(3))])
                            },
                        )
                        .unwrap();
                    linker
                        .define("env", "host_mix", External::Func(hf))
                        .unwrap();
                    let inst = linker.instantiate(&mut store, &module).unwrap();
                    let External::Func(fm) = inst.export(&store, "main").unwrap() else {
                        panic!()
                    };
                    let External::Memory(mm) = inst.export(&store, "memory").unwrap() else {
                        panic!()
                    };
                    let out = store.invoke(fm, &[Value::I32(x_in), Value::I64(y_in)]);
                    assert_eq!(store.active_local_slots, 0);
                    let mem_snap = store.read_memory(mm, 0, 256).unwrap();
                    let mem_cross = store.read_memory(mm, 65520, 32).unwrap();
                    res.push((
                        out.map_err(|e| (e.kind, e.trap_code, e.message)),
                        store.current_fuel,
                        mem_snap,
                        mem_cross,
                    ));
                }
                assert_eq!(
                    res[0], res[1],
                    "differential fuzz mismatch on seed {seed_idx} (fuel={fuel:?})"
                );
            }
        }
    }

    #[test]
    fn fast_register_hardened_features_differential_fuzzer() {
        // Fixed-seed deterministic differential fuzzer over exception handling (`try_table`,
        // `throw`, `catch`, `catch_all`), multi-memory (mem0 32-bit, mem1 32-bit, mem2 64-bit),
        // passive `memory.init` / `data.drop`, tail calls (`return_call` / `return_call_indirect`),
        // bulk table ops (`table.size`, `table.init`, `elem.drop`, `table.copy`, `table.grow`,
        // `table.fill`), and large-slot (> 4,096 locals) functions against `Decoded`.
        struct Rng(u64);
        impl Rng {
            fn next(&mut self) -> u64 {
                let mut x = self.0;
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                self.0 = x;
                x
            }
            fn pick(&mut self, n: usize) -> usize {
                (self.next() as usize) % n
            }
        }
        fn push_sleb(out: &mut Vec<u8>, mut v: i32) {
            loop {
                let b = (v & 0x7f) as u8;
                v >>= 7;
                if (v == 0 && (b & 0x40) == 0) || (v == -1 && (b & 0x40) != 0) {
                    out.push(b);
                    break;
                }
                out.push(b | 0x80);
            }
        }
        let mut features = FeatureSet::all_phase3_core();
        features.tail_call = true;
        features.multi_memory = true;
        features.memory64 = true;
        features.exception_handling = true;

        // 1. Verify large-slot (> 4,096 locals) admission + exact `max_local_slots` enforcement!
        {
            let types = vec![1, 0x60, 1, 0x7f, 1, 0x7f];
            let funcs = vec![1, 0];
            let mut exps = vec![1];
            leb_u32(&mut exps, 4);
            exps.extend_from_slice(b"bigf");
            exps.extend_from_slice(&[0x00, 0]);
            // 5,000 i32 locals + optional recursive call:
            //   if p0 > 0 { return bigf(p0 - 1) + 1 } else { local[4999] = 18; return local[4999] * 3 }
            let mut bf = Vec::new();
            bf.extend_from_slice(&[
                0x20, 0, 0x41, 0, 0x4a, 0x04, 0x40, // if p0 > 0
                0x20, 0, 0x41, 1, 0x6b, 0x10, 0, 0x41, 1, 0x6a,
                0x0f, // return bigf(p0 - 1) + 1
                0x0b, // end if
                0x41, 18, 0x22,
            ]);
            leb_u32(&mut bf, 4999);
            bf.extend_from_slice(&[0x41, 3, 0x6c]);
            let mut code = vec![1];
            code.extend_from_slice(&body(&[(5000, 0x7f)], &bf));
            let big_wasm = wasm_mod(&[
                sec(1, &types),
                sec(3, &funcs),
                sec(7, &exps),
                sec(10, &code),
            ]);
            // Call with depth=0 (5,001 slots -> succeeds) and depth=1 under max_total_locals=8000 (10,002 slots -> traps!).
            for (depth, max_slots_opt) in [(0i32, None), (1i32, Some(8_000usize))] {
                let mut outs = Vec::new();
                for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                    let mut cfg = Config {
                        features,
                        ..Config::default()
                    };
                    if let Some(lim) = max_slots_opt {
                        cfg.limits.max_locals = 6_000;
                        cfg.limits.max_total_locals = lim;
                    }
                    let engine = Engine::with_execution_policy(cfg, policy).unwrap();
                    let module = engine.compile(&big_wasm).unwrap();
                    if policy == ExecutionPolicy::Register && max_slots_opt.is_none() {
                        assert!(
                            is_admitted(&module, 0),
                            "5,000-local function must be admitted by raised MAX_FN_SLOTS"
                        );
                    }
                    let mut store = Store::new(&engine);
                    let inst = Linker::new(&engine)
                        .instantiate(&mut store, &module)
                        .unwrap();
                    let External::Func(f) = inst.export(&store, "bigf").unwrap() else {
                        panic!()
                    };
                    let r = store.invoke(f, &[Value::I32(depth)]);
                    assert_eq!(store.active_local_slots, 0);
                    outs.push(r.map_err(|e| (e.kind, e.trap_code, e.message)));
                }
                assert_eq!(outs[0], outs[1], "bigf mismatch (limit={max_slots_opt:?})");
                if max_slots_opt.is_none() {
                    assert_eq!(
                        outs[0],
                        Ok(crate::RunOutcome::Returned(vec![Value::I32(54)]))
                    );
                } else {
                    assert!(outs[0].is_err());
                }
            }
        }

        // 2. Deterministic 60-seed differential fuzzing over EH, multi-memory+mem64, passive data,
        //    tail calls, and bulk table ops.
        let mut rng = Rng(0x243f_6a88_85a3_08d3);
        for seed_idx in 0..60usize {
            // Types:
            //   0: (i32) -> (i32)
            //   1: (i32) -> ()        (tag 0 payload: i32)
            //   2: (i32, i32) -> (i32)
            let types = vec![
                3, 0x60, 1, 0x7f, 1, 0x7f, // 0
                0x60, 1, 0x7f, 0, // 1 (tag 0)
                0x60, 2, 0x7f, 0x7f, 1, 0x7f, // 2
            ];
            let funcs = vec![6, 0, 0, 2, 0, 0, 0];
            // Tables: t0 funcref min=4 max=8, t1 funcref min=4 max=8
            let tbls = vec![2, 0x70, 0x01, 4, 8, 0x70, 0x01, 4, 8];
            // Memories: m0 (32-bit, min=1, max=2), m1 (32-bit, min=1, max=2), m2 (64-bit, min=1, max=2)
            let mems = vec![3, 0x01, 1, 2, 0x01, 1, 2, 0x05, 1, 2];
            // Tag 0: type 1 (i32)
            let tags = vec![1, 0x00, 1];
            let mut exps = vec![5];
            for (name, kind, idx) in [
                ("main", 0u8, 3u32),
                ("bulk_fast", 0, 4),
                ("m0", 2, 0),
                ("m1", 2, 1),
                ("m2", 2, 2),
            ] {
                leb_u32(&mut exps, name.len() as u32);
                exps.extend_from_slice(name.as_bytes());
                exps.push(kind);
                leb_u32(&mut exps, idx);
            }
            // Elem 0 (active on t0 @ 0): [f0, f5], Elem 1 (passive funcref): [f5, f0, f0, f5]
            // (Both f0 and f5 are non-recursive leaves so arbitrary table.init/copy/call_indirect never cycle!)
            let elems = vec![
                2, 0x00, 0x41, 0x00, 0x0b, 2, 0, 5, // active elem 0
                0x01, 0x00, 4, 5, 0, 0, 5, // passive elem 1
            ];
            let data_count = vec![1];
            // Passive data seg 0: 32 deterministic bytes
            let mut data_sec = vec![1, 0x01, 32];
            for _ in 0..32 {
                data_sec.push(rng.next() as u8);
            }

            // f0(x): if (x & 7) == 5 { throw 0 (x ^ 0x55) } else { return x * 3 + 1 }
            let f0 = vec![
                0x20, 0, 0x41, 7, 0x71, 0x41, 5, 0x46, 0x04, 0x40, 0x20, 0, 0x41, 0xd5, 0x00, 0x08,
                0, 0x0b, 0x20, 0, 0x41, 3, 0x6c, 0x41, 1, 0x6a,
            ];
            // f5(x): pure non-recursive leaf returning (x ^ 0x2a) + 5
            let f5 = vec![0x20, 0, 0x41, 0x2a, 0x73, 0x41, 5, 0x6a];
            // f1(x): return_call f2(x, 3)
            let f1 = vec![0x20, 0, 0x41, 3, 0x12, 2];
            // f2(x, k): if k == 0 { return_call_indirect t0 type 0 (x, slot = x & 1) }
            //           else { return_call f2(x + k, k - 1) }
            let f2 = vec![
                0x20, 1, 0x45, 0x04, 0x40, 0x20, 0, 0x20, 0, 0x41, 1, 0x71, 0x13, 0, 0, 0x0b, 0x20,
                0, 0x20, 1, 0x6a, 0x20, 1, 0x41, 1, 0x6b, 0x12, 2,
            ];

            // f4(x): 100% natively admitted fast-register function exercising memory.init, data.drop,
            //        multi-memory copy/fill/mem64, table.size, table.init, elem.drop, table.copy!
            let mut f4 = Vec::new();
            for _ in 0..(6 + rng.pick(5)) {
                match rng.pick(7) {
                    0 => {
                        let mi = rng.pick(3) as u8;
                        let dst = (rng.pick(16) * 8) as i32;
                        let src = rng.pick(20) as i32;
                        let len = rng.pick(12) as i32;
                        if mi == 2 {
                            f4.push(0x42);
                            push_sleb(&mut f4, dst);
                        } else {
                            f4.push(0x41);
                            push_sleb(&mut f4, dst);
                        }
                        f4.push(0x41);
                        push_sleb(&mut f4, src);
                        f4.push(0x41);
                        push_sleb(&mut f4, len);
                        f4.extend_from_slice(&[0xfc, 0x08, 0, mi]);
                    }
                    1 => {
                        let dm = rng.pick(2) as u8;
                        let sm = 1 - dm;
                        let dst = (rng.pick(16) * 4) as i32;
                        let src = (rng.pick(16) * 4) as i32;
                        f4.push(0x41);
                        push_sleb(&mut f4, dst);
                        f4.push(0x41);
                        push_sleb(&mut f4, src);
                        f4.extend_from_slice(&[0x41, 8, 0xfc, 0x0a, dm, sm]);
                        f4.push(0x41);
                        push_sleb(&mut f4, dst);
                        f4.extend_from_slice(&[0x28, 0x42, dm, 0, 0x20, 0, 0x73, 0x21, 0]);
                    }
                    2 => {
                        let dst = (rng.pick(16) * 8) as i32;
                        f4.push(0x42);
                        push_sleb(&mut f4, dst);
                        f4.extend_from_slice(&[0x20, 0, 0x42, 8, 0xfc, 0x0b, 2]);
                        f4.push(0x42);
                        push_sleb(&mut f4, dst);
                        f4.extend_from_slice(&[0x28, 0x42, 2, 0, 0x20, 0, 0x6a, 0x21, 0]);
                    }
                    3 => {
                        if rng.pick(4) == 0 {
                            f4.extend_from_slice(&[0xfc, 0x09, 0]);
                        }
                    }
                    4 => {
                        f4.extend_from_slice(&[
                            0xfc, 0x10, 0, 0xfc, 0x10, 1, 0x6a, 0x20, 0, 0x6a, 0x21, 0,
                        ]);
                    }
                    5 => {
                        let ti = rng.pick(2) as u8;
                        let dst = rng.pick(3) as i32;
                        let src = rng.pick(2) as i32;
                        let len = rng.pick(3) as i32;
                        f4.push(0x41);
                        push_sleb(&mut f4, dst);
                        f4.push(0x41);
                        push_sleb(&mut f4, src);
                        f4.push(0x41);
                        push_sleb(&mut f4, len);
                        f4.extend_from_slice(&[0xfc, 0x0c, 1, ti]);
                    }
                    _ => {
                        let dt = rng.pick(2) as u8;
                        let st = rng.pick(2) as u8;
                        let dst = rng.pick(3) as i32;
                        let src = rng.pick(3) as i32;
                        let len = rng.pick(3) as i32;
                        f4.push(0x41);
                        push_sleb(&mut f4, dst);
                        f4.push(0x41);
                        push_sleb(&mut f4, src);
                        f4.push(0x41);
                        push_sleb(&mut f4, len);
                        f4.extend_from_slice(&[0xfc, 0x0e, dt, st]);
                        if rng.pick(5) == 0 {
                            f4.extend_from_slice(&[0xfc, 0x0d, 1]);
                        }
                    }
                }
            }
            f4.extend_from_slice(&[0x20, 0]);

            // f3(x): mixes calls into fast f4, table.grow/fill, and try_table catch/catch_all across
            //        fast tail-call chains (f1 -> return_call f2 -> return_call_indirect f0 -> throw 0)!
            let mut f3 = Vec::new();
            let steps = 8 + rng.pick(6);
            for _ in 0..steps {
                match rng.pick(4) {
                    0 => {
                        // Call natively-admitted bulk_fast (f4)
                        f3.extend_from_slice(&[0x20, 0, 0x10, 4, 0x21, 0]);
                    }
                    1 => {
                        // table.grow t1 (ref.null func, 1) + table.fill t1 (dst=0, ref.func 0, len=2)
                        f3.extend_from_slice(&[
                            0xd0, 0x70, 0x41, 1, 0xfc, 0x0f, 1, 0x20, 0, 0x6a, 0x21, 0, 0x41, 0,
                            0xd2, 0, 0x41, 2, 0xfc, 0x11, 1,
                        ]);
                    }
                    2 => {
                        // try_table catching tag 0 from f0 or f1 (which tail-calls f2 -> call_indirect!)
                        let target_fn = rng.pick(2) as u8;
                        f3.extend_from_slice(&[
                            0x02, 0x7f, // block (result i32)
                            0x1f, 0x7f, 1, 0x00, 0,
                            0, // try_table (result i32) (catch tag=0 label=0)
                            0x20, 0, 0x10, target_fn, // call f0 or f1 (may throw!)
                            0x0b,      // end try_table
                            0x0b,      // end block
                            0x21, 0,
                        ]);
                    }
                    _ => {
                        // Direct call_indirect through t0 or t1 inside try_table catch_all
                        let ti = rng.pick(2) as u8;
                        let slot = rng.pick(2) as u8;
                        f3.extend_from_slice(&[
                            0x02, 0x40, // block ()
                            0x1f, 0x40, 1, 0x02, 0, // try_table () (catch_all label=0)
                            0x20, 0, 0x41, slot, 0x11, 0, ti, // call_indirect type 0 table ti
                            0x21, 0, 0x0b, // end try_table
                            0x0b, // end block
                        ]);
                    }
                }
            }
            f3.extend_from_slice(&[0x20, 0]);

            let mut code = vec![6];
            code.extend_from_slice(&body(&[], &f0));
            code.extend_from_slice(&body(&[], &f1));
            code.extend_from_slice(&body(&[], &f2));
            code.extend_from_slice(&body(&[], &f3));
            code.extend_from_slice(&body(&[], &f4));
            code.extend_from_slice(&body(&[], &f5));

            let wasm = wasm_mod(&[
                sec(1, &types),
                sec(3, &funcs),
                sec(4, &tbls),
                sec(5, &mems),
                sec(13, &tags),
                sec(7, &exps),
                sec(9, &elems),
                sec(12, &data_count),
                sec(10, &code),
                sec(11, &data_sec),
            ]);

            let x_in = (rng.next() as i32) & 0x7fff;
            for fuel in [None, Some(1), Some(9), Some(35), Some(90)] {
                let mut res = Vec::new();
                for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                    let mut cfg = Config {
                        features,
                        ..Config::default()
                    };
                    cfg.limits.fuel = fuel;
                    let engine = Engine::with_execution_policy(cfg, policy).unwrap();
                    let module = engine.compile(&wasm).unwrap();
                    if policy == ExecutionPolicy::Register {
                        // f1, f2 (tail-call chain) and f4 (all new bulk mem/table ops!) MUST be natively admitted!
                        for bi in [1usize, 2, 4] {
                            assert!(
                                is_admitted(&module, bi),
                                "hardened fuzz seed {seed_idx} body {bi} must be admitted"
                            );
                        }
                    }
                    let mut store = Store::new(&engine);
                    let inst = Linker::new(&engine)
                        .instantiate(&mut store, &module)
                        .unwrap();
                    let External::Func(fm) = inst.export(&store, "main").unwrap() else {
                        panic!()
                    };
                    let External::Memory(m0) = inst.export(&store, "m0").unwrap() else {
                        panic!()
                    };
                    let External::Memory(m1) = inst.export(&store, "m1").unwrap() else {
                        panic!()
                    };
                    let External::Memory(m2) = inst.export(&store, "m2").unwrap() else {
                        panic!()
                    };
                    let out = store.invoke(fm, &[Value::I32(x_in)]);
                    assert_eq!(store.active_local_slots, 0);
                    res.push((
                        out.map_err(|e| (e.kind, e.trap_code, e.message)),
                        store.current_fuel,
                        store.read_memory(m0, 0, 160).unwrap(),
                        store.read_memory(m1, 0, 160).unwrap(),
                        store.read_memory(m2, 0, 160).unwrap(),
                    ));
                }
                assert_eq!(
                    res[0], res[1],
                    "hardened differential fuzz mismatch on seed {seed_idx} (fuel={fuel:?})"
                );
            }
        }
    }
}
