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
const MAX_FAST_BLOCKS: usize = 65_536;
const MAX_FAST_STACK_TYPES: usize = 262_144;
const MAX_FAST_STATIC_LABELS: usize = 65_536;
const MAX_FAST_PREFIX_COPIES: usize = 262_144;
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
    Ref,
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
            ValType::Ref(_) => Self::Ref,
        })
    }

    #[inline(always)]
    fn slots(self) -> u16 {
        match self {
            Self::V128 => 2,
            Self::Ref => 0,
            _ => 1,
        }
    }

    #[inline(always)]
    fn ref_slots(self) -> u16 {
        u16::from(matches!(self, Self::Ref))
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
    arg_ref_base: u16,
    arg_ref_slots: u16,
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
    arg_ref_base: u16,
    arg_ref_slots: u16,
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
    /// Fused `dst = acc + (a * b)` with exact two-rounding IEEE-754 Wasm semantics (`f32`).
    F32MulAdd {
        dst: u16,
        acc: u16,
        a: u16,
        b: u16,
    },
    /// Fused `dst = acc + (a * b)` with exact two-rounding IEEE-754 Wasm semantics (`f64`).
    F64MulAdd {
        dst: u16,
        acc: u16,
        a: u16,
        b: u16,
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
    /// Fused `acc = acc.wrapping_add(u32(mem0[addr + offset]))`.
    LoadI32Add {
        acc: u16,
        addr: u16,
        offset: u32,
        src_off: u8,
    },
    /// Fused `acc = f32(acc) + f32(mem0[addr + offset])`.
    LoadF32Add {
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
    /// Fused `acc = acc.wrapping_add(u32(mem0[(base + (idx << 2)) + offset]))` (memory32).
    LoadI32Shl2AddAcc {
        acc: u16,
        base: u16,
        idx: u16,
        offset: u32,
        src_off: u8,
    },
    /// Fused `acc = f64(acc) + f64(mem0[(base + (idx << 3)) + offset])` (memory32).
    LoadF64Shl3AddAcc {
        acc: u16,
        base: u16,
        idx: u16,
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
    /// Fused `dst = mem0[(a + b) + offset]` (i64/f64, memory32).
    LoadI64AddAddr {
        dst: u16,
        a: u16,
        b: u16,
        offset: u32,
        src_off: u8,
    },
    /// Fused `dst = mem0[(idx << 2) + offset]` (i32/f32, memory32).
    LoadI32Shl2 {
        dst: u16,
        idx: u16,
        offset: u32,
        src_off: u8,
    },
    /// Fused `dst = mem0[(base + (idx << 2)) + offset]` (i32/f32, memory32).
    LoadI32Shl2Add {
        dst: u16,
        base: u16,
        idx: u16,
        offset: u32,
        src_off: u8,
    },
    /// Fused `dst = mem0[(base + (idx << 3)) + offset]` (i64/f64, memory32).
    LoadI64Shl3Add {
        dst: u16,
        base: u16,
        idx: u16,
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
    /// Fused 4-byte immediate store (`mem0[addr + offset] = val`, covers zero-stores & constants).
    StoreI32Imm {
        addr: u16,
        val: u32,
        offset: u16,
        src_off: u8,
    },
    /// Fused 1-byte immediate store (`mem0[addr + offset] = val`).
    StoreI32_8Imm {
        addr: u16,
        val: u8,
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
    /// Fused `mem0[(idx << 2) + offset] = val as u32` (memory32).
    StoreI32Shl2 {
        idx: u16,
        val: u16,
        offset: u32,
        src_off: u8,
    },
    /// Fused `mem0[(base + (idx << 2)) + offset] = val as u32` (memory32).
    StoreI32Shl2Add {
        base: u16,
        idx: u16,
        val: u16,
        offset: u32,
        src_off: u8,
    },
    /// Fused `mem0[(base + (idx << 3)) + offset] = val as u64` (memory32).
    StoreI64Shl3Add {
        base: u16,
        idx: u16,
        val: u16,
        offset: u32,
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
    /// `reg[r] += step; if (reg[r] as u32) != (reg[limit] as u32) { ip = target }`.
    I32IncNeBrIf {
        reg: u16,
        limit: u16,
        step: i16,
        target: u32,
    },
    /// `reg[r] += step; if (reg[r] as u32) != 0 { ip = target }`.
    I32IncBrIfNonZero {
        reg: u16,
        step: i32,
        target: u32,
    },
    /// `reg[r] += step; if cmp(reg[r], imm) { ip = target }` (signed `imm` for `lt_s/gt_s/le_s/ge_s`, unsigned otherwise).
    I32IncCmpImmBrIf {
        cmp: u8,
        step: i8,
        reg: u16,
        imm: u16,
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
    /// Unboxed `i8x16.splat` directly on `[u64; 2]` slots.
    V128Splat8 {
        dst: u16,
        src: u16,
    },
    /// Unboxed `i16x8.splat` directly on `[u64; 2]` slots.
    V128Splat16 {
        dst: u16,
        src: u16,
    },
    /// Unboxed `i32x4.splat` / `f32x4.splat` directly on `[u64; 2]` slots.
    V128Splat32 {
        dst: u16,
        src: u16,
    },
    /// Unboxed `i64x2.splat` / `f64x2.splat` directly on `[u64; 2]` slots.
    V128Splat64 {
        dst: u16,
        src: u16,
    },
    /// Unboxed `v128.not`.
    V128Not {
        dst: u16,
        src: u16,
    },
    /// Unboxed `v128.and`.
    V128And {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `v128.andnot`.
    V128AndNot {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `v128.or`.
    V128Or {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `v128.xor`.
    V128Xor {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `v128.bitselect` (and relaxed `laneselect`).
    V128Bitselect {
        dst: u16,
        a: u16,
        b: u16,
        c: u16,
    },
    /// Unboxed `v128.any_true`.
    V128AnyTrue {
        dst: u16,
        src: u16,
    },
    /// Unboxed `i8x16.bitmask`.
    I8x16Bitmask {
        dst: u16,
        src: u16,
    },
    /// Unboxed `i32x4.bitmask`.
    I32x4Bitmask {
        dst: u16,
        src: u16,
    },
    /// Unboxed `i32x4.extract_lane` / `f32x4.extract_lane` (zero-extended to `u64`).
    SimdExtract32 {
        dst: u16,
        src: u16,
        lane: u8,
    },
    /// Unboxed `i8x16.add`.
    I8x16Add {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `i8x16.sub`.
    I8x16Sub {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `i16x8.add`.
    I16x8Add {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `i16x8.sub`.
    I16x8Sub {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `i16x8.mul`.
    I16x8Mul {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `i32x4.add`.
    I32x4Add {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `i32x4.sub`.
    I32x4Sub {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `i32x4.mul`.
    I32x4Mul {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `i32x4.{eq,ne,lt_s,lt_u,gt_s,gt_u,le_s,le_u,ge_s,ge_u}` (`cmp` in `0..=9`).
    I32x4Cmp {
        cmp: u8,
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `i32x4.{shl,shr_s,shr_u}` (`kind` in `0..=2`, shift count masked `& 31`).
    I32x4Shift {
        kind: u8,
        dst: u16,
        a: u16,
        sh: u16,
    },
    /// Unboxed `i64x2.add`.
    I64x2Add {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `i64x2.sub`.
    I64x2Sub {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `i64x2.mul`.
    I64x2Mul {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `f32x4.abs`.
    F32x4Abs {
        dst: u16,
        src: u16,
    },
    /// Unboxed `f32x4.neg`.
    F32x4Neg {
        dst: u16,
        src: u16,
    },
    /// Unboxed `f32x4.sqrt`.
    F32x4Sqrt {
        dst: u16,
        src: u16,
    },
    /// Unboxed `f32x4.add`.
    F32x4Add {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `f32x4.sub`.
    F32x4Sub {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `f32x4.mul`.
    F32x4Mul {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `f32x4.div`.
    F32x4Div {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `f32x4.min` (and `f32x4.relaxed_min`) with canonical Wasm NaN & `-0.0` semantics.
    F32x4Min {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `f32x4.max` (and `f32x4.relaxed_max`) with canonical Wasm NaN & `+0.0` semantics.
    F32x4Max {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `f32x4.pmin`.
    F32x4PMin {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `f32x4.pmax`.
    F32x4PMax {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `f32x4.{eq,ne,lt,gt,le,ge}` (`cmp` in `0..=5`).
    F32x4Cmp {
        cmp: u8,
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Fused unboxed `dst = acc + (a * b)` on 4×`f32` with exact two-rounding IEEE-754 semantics.
    F32x4MulAdd {
        dst: u16,
        acc: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `f32x4.relaxed_madd`: `a * b + c` (fused single-rounding `mul_add`).
    F32x4RelaxedMadd {
        dst: u16,
        a: u16,
        b: u16,
        c: u16,
    },
    /// Unboxed `f32x4.relaxed_nmadd`: `(-a) * b + c` (fused single-rounding `mul_add`).
    F32x4RelaxedNmadd {
        dst: u16,
        a: u16,
        b: u16,
        c: u16,
    },
    /// Unboxed `f32x4.convert_i32x4_s`.
    F32x4ConvertI32x4S {
        dst: u16,
        src: u16,
    },
    /// Unboxed `f32x4.convert_i32x4_u`.
    F32x4ConvertI32x4U {
        dst: u16,
        src: u16,
    },
    /// Unboxed `f64x2.abs`.
    F64x2Abs {
        dst: u16,
        src: u16,
    },
    /// Unboxed `f64x2.neg`.
    F64x2Neg {
        dst: u16,
        src: u16,
    },
    /// Unboxed `f64x2.sqrt`.
    F64x2Sqrt {
        dst: u16,
        src: u16,
    },
    /// Unboxed `f64x2.add`.
    F64x2Add {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `f64x2.sub`.
    F64x2Sub {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `f64x2.mul`.
    F64x2Mul {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `f64x2.div`.
    F64x2Div {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `f64x2.min` (and `f64x2.relaxed_min`) with canonical Wasm NaN & `-0.0` semantics.
    F64x2Min {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `f64x2.max` (and `f64x2.relaxed_max`) with canonical Wasm NaN & `+0.0` semantics.
    F64x2Max {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `f64x2.pmin`.
    F64x2PMin {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `f64x2.pmax`.
    F64x2PMax {
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `f64x2.{eq,ne,lt,gt,le,ge}` (`cmp` in `0..=5`).
    F64x2Cmp {
        cmp: u8,
        dst: u16,
        a: u16,
        b: u16,
    },
    /// Fused unboxed `dst = acc + (a * b)` on 2×`f64` with exact two-rounding IEEE-754 semantics.
    F64x2MulAdd {
        dst: u16,
        acc: u16,
        a: u16,
        b: u16,
    },
    /// Unboxed `f64x2.relaxed_madd`: `a * b + c` (fused single-rounding `mul_add`).
    F64x2RelaxedMadd {
        dst: u16,
        a: u16,
        b: u16,
        c: u16,
    },
    /// Unboxed `f64x2.relaxed_nmadd`: `(-a) * b + c` (fused single-rounding `mul_add`).
    F64x2RelaxedNmadd {
        dst: u16,
        a: u16,
        b: u16,
        c: u16,
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
    // ---- Reference operations (parallel `ref_bank: Vec<RefValue>`) ----
    MovRef {
        dst: u16,
        src: u16,
    },
    MoveRefRange {
        dst_base: u16,
        src_base: u16,
        count: u16,
    },
    RefNull {
        dst: u16,
        heap_idx: u32,
    },
    RefIsNull {
        dst: u16,
        src: u16,
    },
    RefFunc {
        dst: u16,
        func_idx: u32,
    },
    RefEq {
        dst: u16,
        a: u16,
        b: u16,
    },
    RefAsNonNull {
        dst: u16,
        src: u16,
        src_off: u8,
    },
    SelectRef {
        dst: u16,
        a: u16,
        b: u16,
        cond: u16,
    },
    GlobalGetRef {
        dst: u16,
        global: u32,
    },
    GlobalSetRef {
        src: u16,
        global: u32,
    },
    TableGet {
        dst: u16,
        idx: u16,
        table: u32,
        src_off: u8,
    },
    TableSet {
        idx: u16,
        val: u16,
        table: u32,
        src_off: u8,
    },
    Return {
        base: u16,
        count: u16,
        ref_base: u16,
        ref_count: u16,
    },
    TrapUnreachable {
        src_off: u8,
    },
    /// Clean bail to the decoded interpreter at block boundary `meta`.
    Bail {
        meta: u32,
    },
}

const _: () = assert!(std::mem::size_of::<RegOp>() <= 12);

/// Lazily compiled untyped-`u64` + parallel `RefValue` register function owned by a `Module`.
#[derive(Debug)]
pub(crate) struct FastFunction {
    ops: Box<[RegOp]>,
    consts: Box<[u64]>,
    v128_consts: Box<[[u64; 2]]>,
    simd_masks: Box<[[u8; 16]]>,
    ref_heaps: Box<[crate::HeapType]>,
    local_ref_heaps: Box<[crate::HeapType]>,
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
    /// Number of Wasm function parameters (`<= num_locals`).
    num_params: u16,
    /// Physical `u64` register slots occupied by locals (`sum(ty.slots())`).
    local_reg_slots: u16,
    /// Physical `RefValue` slots occupied by reference locals (`sum(ty.ref_slots())`).
    local_ref_slots: u16,
    slots: u16,
    ref_slots: u16,
    stack_peak: usize,
    max_nesting: usize,
    mem0_is_64: bool,
    fuel_blocks: bool,
    /// True when the entire body is a straight-line, trap-free, scalar leaf (`slots <= 16`,
    /// `ref_slots == 0`, no calls/loops/memory/globals/bails) eligible for zero-frame entry.
    pure_leaf: bool,
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
    /// Lazy `(local[src] as u32) << shift` (`src < local_reg_slots`, `shift` in `1..=3`).
    ShlImm {
        src: u16,
        shift: u8,
    },
    /// Lazy `(local[base] as u32).wrapping_add((local[idx] as u32) << shift)` (both `< local_reg_slots`).
    AddShl {
        base: u16,
        idx: u16,
        shift: u8,
    },
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
    base_ref_slot: u16,
    prefix: Vec<FastType>,
    params: Vec<FastType>,
    results: Vec<FastType>,
    param_slots: u16,
    param_ref_slots: u16,
    result_slots: u16,
    result_ref_slots: u16,
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
        if let ImportKind::Func(ti) | ImportKind::ExactFunc(ti) = imp.kind {
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
                -24..=-11 => FastType::Ref,
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
        0x25 | 0x26 => {
            let Immediate::Index(ti) = insn.immediate else {
                return false;
            };
            declared_table(module, ti).is_some()
        }
        0xd0 | 0xd1 | 0xd3 | 0xd4 => true,
        0xd2 => {
            let Immediate::Index(fi) = insn.immediate else {
                return false;
            };
            declared_func_type(module, fi).is_some()
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

fn swap_i32_cmp(cmp: u8) -> u8 {
    match cmp {
        0 => 0, // eq
        1 => 1, // ne
        2 => 4, // lt_s <-> gt_s
        4 => 2,
        3 => 5, // lt_u <-> gt_u
        5 => 3,
        6 => 8, // le_s <-> ge_s
        8 => 6,
        7 => 9, // le_u <-> ge_u
        9 => 7,
        _ => unreachable!(),
    }
}

#[inline(always)]
fn eval_f32_cmp(cmp: u8, a: f32, b: f32) -> bool {
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
fn wasm_f32_min(fa: f32, fb: f32) -> u32 {
    let r = if fa.is_nan() || fb.is_nan() {
        f32::NAN
    } else if fa == fb {
        f32::from_bits(fa.to_bits() | fb.to_bits())
    } else {
        fa.min(fb)
    };
    r.to_bits()
}

#[inline(always)]
fn wasm_f32_max(fa: f32, fb: f32) -> u32 {
    let r = if fa.is_nan() || fb.is_nan() {
        f32::NAN
    } else if fa == fb {
        f32::from_bits(fa.to_bits() & fb.to_bits())
    } else {
        fa.max(fb)
    };
    r.to_bits()
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

#[inline(always)]
fn last_inc_step(ops: &[RegOp], reg: u16) -> Option<i32> {
    match ops.last().copied()? {
        RegOp::I32AddImm { dst, src, imm } if dst == reg && src == reg => Some(imm),
        RegOp::I32SubImm { dst, src, imm } if dst == reg && src == reg => imm.checked_neg(),
        _ => None,
    }
}

#[inline(always)]
fn encode_cmp_imm16(cmp: u8, imm: u32) -> Option<u16> {
    match cmp {
        2 | 4 | 6 | 8 => i16::try_from(imm as i32).ok().map(|v| v as u16),
        _ => u16::try_from(imm).ok(),
    }
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

/// Lazily compiles `body` of `module` to an untyped-`u64` + parallel `RefValue` 3-address `FastFunction`.
/// Supports scalar (`i32`, `i64`, `f32`, `f64`), native SIMD (`v128`, 2 `u64` slots per value),
/// and reference values (`funcref`/`externref`/`anyref`/GC refs in a parallel `RefValue` bank).
/// Returns `None` if the function exceeds per-body limits.
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
    let mut local_ref_heaps: Vec<crate::HeapType> = Vec::new();
    let mut local_reg_slots_usize = 0usize;
    let mut local_ref_slots_usize = 0usize;
    for (idx, &t) in ft.params.iter().chain(&code.locals).enumerate() {
        let fty = FastType::from_val(t)?;
        if fty == FastType::Ref {
            local_slots.push(u16::try_from(local_ref_slots_usize).ok()?);
            local_ref_slots_usize = local_ref_slots_usize.checked_add(1)?;
            if idx >= num_params
                && let ValType::Ref(rt) = t
            {
                local_ref_heaps.push(rt.heap);
            }
        } else {
            local_slots.push(u16::try_from(local_reg_slots_usize).ok()?);
            local_reg_slots_usize = local_reg_slots_usize.checked_add(fty.slots() as usize)?;
        }
        local_types.push(fty);
    }
    if local_reg_slots_usize.checked_add(bounds.stack_peak)? > MAX_FN_SLOTS
        || local_ref_slots_usize.checked_add(bounds.stack_peak)? > MAX_FN_SLOTS
    {
        return None;
    }
    let mut result_types = Vec::with_capacity(ft.results.len());
    let mut result_reg_slots_usize = 0usize;
    let mut result_ref_slots_usize = 0usize;
    for &t in &ft.results {
        let fty = FastType::from_val(t)?;
        result_reg_slots_usize = result_reg_slots_usize.checked_add(fty.slots() as usize)?;
        result_ref_slots_usize = result_ref_slots_usize.checked_add(fty.ref_slots() as usize)?;
        result_types.push(fty);
    }
    let result_reg_slots = u16::try_from(result_reg_slots_usize).ok()?;
    let result_ref_slots = u16::try_from(result_ref_slots_usize).ok()?;
    let local_reg_slots = u16::try_from(local_reg_slots_usize).ok()?;
    let local_ref_slots = u16::try_from(local_ref_slots_usize).ok()?;

    // Check block signatures and compute basic-block starts.
    let fuel_blocks = module.data.register_fast_fuel;
    let mut is_block_start = Vec::new();
    is_block_start.try_reserve_exact(n + 1).ok()?;
    is_block_start.resize(n + 1, false);
    let mut is_check_block = Vec::new();
    is_check_block.try_reserve_exact(n + 1).ok()?;
    is_check_block.resize(n + 1, false);
    let mut has_loops_or_calls = false;
    is_block_start[0] = true;
    is_check_block[0] = true;
    for (pc, insn) in code.instructions.iter().enumerate() {
        match insn.opcode {
            0x02..=0x04 => {
                // Unsupported block signatures decline the whole function cleanly.
                block_sig(module, &insn.immediate)?;
                let (end, other) = module.data.control_get(body, pc)?;
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

    let mut ops: Vec<RegOp> = Vec::new();
    ops.try_reserve_exact(n).ok()?;
    let mut consts: Vec<u64> = Vec::new();
    let mut v128_consts: Vec<[u64; 2]> = Vec::new();
    let mut simd_masks: Vec<[u8; 16]> = Vec::new();
    let mut ref_heaps: Vec<crate::HeapType> = Vec::new();
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
    let mut pc_to_ip: Vec<u32> = Vec::new();
    pc_to_ip.try_reserve_exact(n + 1).ok()?;
    pc_to_ip.resize(n + 1, u32::MAX);
    // (ip_of_branch_op, target_source_pc)
    let mut branch_patches: Vec<(usize, usize)> = Vec::new();
    // (table_targets_index, target_source_pc)
    let mut table_patches: Vec<(usize, usize)> = Vec::new();

    let mut max_slots: usize = local_reg_slots_usize + bounds.stack_peak;
    let mut max_ref_slots: usize = local_ref_slots_usize;
    let mut vstack: Vec<VSlot> = Vec::new();
    vstack.try_reserve_exact(bounds.stack_peak).ok()?;
    let mut labels: Vec<CompileLabel> = Vec::new();
    let mut prefix_copies: usize = 0;
    let mut dead = false;

    let mut in_block = false;
    let mut cur_block_ip: Option<usize> = None;
    let mut cur_block_start_ip: usize = 0;
    let mut cur_block_meta: u32 = BLOCK_META_NONE;
    let mut cur_block_ops: usize = 0;

    let slot_top = |vstack: &[VSlot]| -> u16 {
        vstack
            .iter()
            .rev()
            .find(|s| s.ty != FastType::Ref)
            .map_or(local_reg_slots, |s| s.slot_off + s.ty.slots())
    };

    let ref_top = |vstack: &[VSlot]| -> u16 {
        vstack
            .iter()
            .rev()
            .find(|s| s.ty == FastType::Ref)
            .map_or(local_ref_slots, |s| s.slot_off + 1)
    };

    let copy_prefix = |vstack: &[VSlot],
                       base_height: usize,
                       prefix_copies: &mut usize|
     -> Option<Vec<FastType>> {
        let next = prefix_copies.checked_add(base_height)?;
        if next > MAX_FAST_PREFIX_COPIES {
            return None;
        }
        let mut prefix = Vec::new();
        prefix.try_reserve_exact(base_height).ok()?;
        prefix.extend(vstack[..base_height].iter().map(|s| s.ty));
        *prefix_copies = next;
        Some(prefix)
    };

    let push_meta = |source_pc: usize,
                     vstack: &[VSlot],
                     labels: &[CompileLabel],
                     blocks: &mut Vec<BlockMeta>,
                     stack_types: &mut Vec<FastType>,
                     static_labels: &mut Vec<StaticLabel>|
     -> Option<u32> {
        if blocks.len() >= MAX_FAST_BLOCKS
            || stack_types
                .len()
                .checked_add(vstack.len())
                .is_none_or(|n| n > MAX_FAST_STACK_TYPES)
            || static_labels
                .len()
                .checked_add(labels.len())
                .is_none_or(|n| n > MAX_FAST_STATIC_LABELS)
        {
            return None;
        }
        blocks.try_reserve(1).ok()?;
        stack_types.try_reserve(vstack.len()).ok()?;
        static_labels.try_reserve(labels.len()).ok()?;
        let stack_start = u32::try_from(stack_types.len()).ok()?;
        for s in vstack {
            stack_types.push(s.ty);
        }
        let labels_start = u32::try_from(static_labels.len()).ok()?;
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
        let idx = u32::try_from(blocks.len()).ok()?;
        blocks.push(BlockMeta {
            source_pc: source_pc as u32,
            fuel: 0,
            stack_start,
            stack_len: u16::try_from(vstack.len()).ok()?,
            labels_start,
            labels_len: u16::try_from(labels.len()).ok()?,
        });
        Some(idx)
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

    let flush_local =
        |local_slot: u16, is_ref: bool, vstack: &mut [VSlot], ops: &mut Vec<RegOp>| {
            for slot in vstack.iter_mut() {
                if (slot.ty == FastType::Ref) != is_ref {
                    continue;
                }
                let dst = slot.slot_off;
                match slot.src {
                    VVal::Local(s) if s == local_slot => {
                        if is_ref {
                            ops.push(RegOp::MovRef {
                                dst,
                                src: local_slot,
                            });
                        } else if slot.ty == FastType::V128 {
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
                    VVal::ShlImm { src, shift } if !is_ref && src == local_slot => {
                        ops.push(RegOp::I32ShlImm {
                            dst,
                            src,
                            shift: u32::from(shift),
                        });
                        slot.src = VVal::Slot(dst);
                    }
                    VVal::AddShl { base, idx, shift }
                        if !is_ref && (base == local_slot || idx == local_slot) =>
                    {
                        ops.push(RegOp::I32ShlImm {
                            dst,
                            src: idx,
                            shift: u32::from(shift),
                        });
                        ops.push(RegOp::I32Add {
                            dst,
                            a: base,
                            b: dst,
                        });
                        slot.src = VVal::Slot(dst);
                    }
                    _ => {}
                }
            }
        };

    let flush_all = |vstack: &mut [VSlot], ops: &mut Vec<RegOp>, consts: &mut Vec<u64>| {
        for slot in vstack.iter_mut() {
            let dst = slot.slot_off;
            match slot.src {
                VVal::Slot(s) if s == dst => {}
                VVal::Slot(s) | VVal::Local(s) => {
                    if slot.ty == FastType::Ref {
                        ops.push(RegOp::MovRef { dst, src: s });
                    } else if slot.ty == FastType::V128 {
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
                VVal::ShlImm { src, shift } => {
                    ops.push(RegOp::I32ShlImm {
                        dst,
                        src,
                        shift: u32::from(shift),
                    });
                    slot.src = VVal::Slot(dst);
                }
                VVal::AddShl { base, idx, shift } => {
                    ops.push(RegOp::I32ShlImm {
                        dst,
                        src: idx,
                        shift: u32::from(shift),
                    });
                    ops.push(RegOp::I32Add {
                        dst,
                        a: base,
                        b: dst,
                    });
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
            VVal::ShlImm { src, shift } => {
                *max_slots = (*max_slots).max(tmp_slot as usize + 1);
                ops.push(RegOp::I32ShlImm {
                    dst: tmp_slot,
                    src,
                    shift: u32::from(shift),
                });
                tmp_slot
            }
            VVal::AddShl { base, idx, shift } => {
                *max_slots = (*max_slots).max(tmp_slot as usize + 1);
                ops.push(RegOp::I32ShlImm {
                    dst: tmp_slot,
                    src: idx,
                    shift: u32::from(shift),
                });
                ops.push(RegOp::I32Add {
                    dst: tmp_slot,
                    a: base,
                    b: tmp_slot,
                });
                tmp_slot
            }
        }
    };

    let mut pc = 0usize;
    while pc < n {
        if slot_top(&vstack) as usize + 8 > MAX_FN_SLOTS
            || ref_top(&vstack) as usize + 8 > MAX_FN_SLOTS
        {
            return None;
        }
        let insn = &code.instructions[pc];

        // Handle dead (unreachable) regions up to matching `else` or `end`.
        if dead {
            match insn.opcode {
                0x02..=0x04 => {
                    let (params, results) = block_sig(module, &insn.immediate)?;
                    let param_slots = params.iter().map(|t| t.slots()).sum();
                    let param_ref_slots = params.iter().map(|t| t.ref_slots()).sum();
                    let result_slots = results.iter().map(|t| t.slots()).sum();
                    let result_ref_slots = results.iter().map(|t| t.ref_slots()).sum();
                    let (end, other) = module.data.control_get(body, pc)?;
                    labels.try_reserve(1).ok()?;
                    labels.push(CompileLabel {
                        start: pc + 1,
                        end,
                        other,
                        loop_: insn.opcode == 0x03,
                        base_height: 0,
                        base_slot: local_reg_slots,
                        base_ref_slot: local_ref_slots,
                        prefix: Vec::new(),
                        params,
                        results,
                        param_slots,
                        param_ref_slots,
                        result_slots,
                        result_ref_slots,
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
                        let mut roff = local_ref_slots;
                        for &ty in top.prefix.iter().chain(&top.params) {
                            let s = if ty == FastType::Ref { roff } else { off };
                            vstack.push(VSlot {
                                src: VVal::Slot(s),
                                slot_off: s,
                                ty,
                            });
                            off += ty.slots();
                            roff += ty.ref_slots();
                        }
                        max_slots = max_slots.max(off as usize);
                        max_ref_slots = max_ref_slots.max(roff as usize);
                    }
                }
                0x0b => {
                    if let Some(label) = labels.pop() {
                        if !label.parent_dead {
                            dead = false;
                            vstack.clear();
                            let mut off = local_reg_slots;
                            let mut roff = local_ref_slots;
                            for &ty in label.prefix.iter().chain(&label.results) {
                                let s = if ty == FastType::Ref { roff } else { off };
                                vstack.push(VSlot {
                                    src: VVal::Slot(s),
                                    slot_off: s,
                                    ty,
                                });
                                off += ty.slots();
                                roff += ty.ref_slots();
                            }
                            max_slots = max_slots.max(off as usize);
                            max_ref_slots = max_ref_slots.max(roff as usize);
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
                                    )?;
                                    if static_labels.len() >= MAX_FAST_STATIC_LABELS {
                                        return None;
                                    }
                                    static_labels.try_reserve(1).ok()?;
                                    blocks[meta as usize].labels_len =
                                        blocks[meta as usize].labels_len.checked_add(1)?;
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
                )?;
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
                )?
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
            let is_ref = res_ty == FastType::Ref;
            if pc + 1 < n && !is_block_start[pc + 1] && cur_block_ops < MAX_BLOCK_OPS {
                let next = &code.instructions[pc + 1];
                if let Immediate::Index(l) = next.immediate
                    && (next.opcode == 0x21 || next.opcode == 0x22)
                {
                    let l_slot = local_slots[l as usize];
                    flush_local(l_slot, is_ref, vstack, ops);
                    return (l_slot, 1, next.opcode == 0x22);
                }
            }
            (
                if is_ref {
                    ref_top(vstack)
                } else {
                    slot_top(vstack)
                },
                0,
                false,
            )
        };

        let finish_dst = |dst: u16,
                          consumed_next: usize,
                          is_tee: bool,
                          res_ty: FastType,
                          vstack: &mut Vec<VSlot>,
                          max_slots: &mut usize,
                          max_ref_slots: &mut usize,
                          cur_block_ops: &mut usize,
                          pc: &mut usize| {
            let is_ref = res_ty == FastType::Ref;
            if consumed_next > 0 {
                *cur_block_ops += consumed_next;
                *pc += consumed_next;
                if is_tee {
                    let slot_off = if is_ref {
                        ref_top(vstack)
                    } else {
                        slot_top(vstack)
                    };
                    vstack.push(VSlot {
                        src: VVal::Local(dst),
                        slot_off,
                        ty: res_ty,
                    });
                    if is_ref {
                        *max_ref_slots = (*max_ref_slots).max(slot_off as usize + 1);
                    } else {
                        *max_slots = (*max_slots).max(slot_off as usize + res_ty.slots() as usize);
                    }
                }
            } else {
                vstack.push(VSlot {
                    src: VVal::Slot(dst),
                    slot_off: dst,
                    ty: res_ty,
                });
                if is_ref {
                    *max_ref_slots = (*max_ref_slots).max(dst as usize + 1);
                } else {
                    *max_slots = (*max_slots).max(dst as usize + res_ty.slots() as usize);
                }
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
                let param_ref_slots = params.iter().map(|t| t.ref_slots()).sum();
                let result_slots = results.iter().map(|t| t.slots()).sum();
                let result_ref_slots = results.iter().map(|t| t.ref_slots()).sum();
                let (end, other) = module.data.control_get(body, pc)?;
                if insn.opcode == 0x03 {
                    flush_all(&mut vstack, &mut ops, &mut consts);
                }
                let base_height = vstack.len().checked_sub(params.len())?;
                let base_slot = slot_top(&vstack[..base_height]);
                let base_ref_slot = ref_top(&vstack[..base_height]);
                let prefix = copy_prefix(&vstack, base_height, &mut prefix_copies)?;
                labels.try_reserve(1).ok()?;
                labels.push(CompileLabel {
                    start: pc + 1,
                    end,
                    other,
                    loop_: insn.opcode == 0x03,
                    base_height,
                    base_slot,
                    base_ref_slot,
                    prefix,
                    params,
                    results,
                    param_slots,
                    param_ref_slots,
                    result_slots,
                    result_ref_slots,
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
                let param_ref_slots = params.iter().map(|t| t.ref_slots()).sum();
                let result_slots = results.iter().map(|t| t.slots()).sum();
                let result_ref_slots = results.iter().map(|t| t.ref_slots()).sum();
                let (end, other) = module.data.control_get(body, pc)?;
                flush_all(&mut vstack, &mut ops, &mut consts);
                let cond_reg = mat(
                    cond_slot.src,
                    slot_top(&vstack),
                    &mut max_slots,
                    &mut ops,
                    &mut consts,
                );
                let base_height = vstack.len().checked_sub(params.len())?;
                let base_slot = slot_top(&vstack[..base_height]);
                let base_ref_slot = ref_top(&vstack[..base_height]);
                let prefix = copy_prefix(&vstack, base_height, &mut prefix_copies)?;
                labels.try_reserve(1).ok()?;
                labels.push(CompileLabel {
                    start: pc + 1,
                    end,
                    other,
                    loop_: false,
                    base_height,
                    base_slot,
                    base_ref_slot,
                    prefix,
                    params,
                    results,
                    param_slots,
                    param_ref_slots,
                    result_slots,
                    result_ref_slots,
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
                let mut roff = local_ref_slots;
                for &ty in top.prefix.iter().chain(&top.params) {
                    let s = if ty == FastType::Ref { roff } else { off };
                    vstack.push(VSlot {
                        src: VVal::Slot(s),
                        slot_off: s,
                        ty,
                    });
                    off += ty.slots();
                    roff += ty.ref_slots();
                }
                max_slots = max_slots.max(off as usize);
                max_ref_slots = max_ref_slots.max(roff as usize);
            }
            0x0b => {
                // end
                if let Some(_label) = labels.pop() {
                    // Block end: fallthrough.
                } else {
                    flush_all(&mut vstack, &mut ops, &mut consts);
                    let count = result_types.len();
                    let start = vstack.len().checked_sub(count)?;
                    let base = slot_top(&vstack[..start]);
                    let ref_base = ref_top(&vstack[..start]);
                    ops.push(RegOp::Return {
                        base,
                        count: result_reg_slots,
                        ref_base,
                        ref_count: result_ref_slots,
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
                    let base = slot_top(&vstack[..start]);
                    let ref_base = ref_top(&vstack[..start]);
                    ops.push(RegOp::Return {
                        base,
                        count: result_reg_slots,
                        ref_base,
                        ref_count: result_ref_slots,
                    });
                } else {
                    let idx = labels.len().checked_sub(depth + 1)?;
                    let label = &labels[idx];
                    let target_pc = if label.loop_ {
                        label.start
                    } else {
                        label.end + 1
                    };
                    let (arity, arity_slots, arity_ref_slots) = if label.loop_ {
                        (label.params.len(), label.param_slots, label.param_ref_slots)
                    } else {
                        (
                            label.results.len(),
                            label.result_slots,
                            label.result_ref_slots,
                        )
                    };
                    let src_start = vstack.len().checked_sub(arity)?;
                    let src_base = slot_top(&vstack[..src_start]);
                    let src_ref_base = ref_top(&vstack[..src_start]);
                    let dst_base = label.base_slot;
                    let dst_ref_base = label.base_ref_slot;
                    let move_reg = arity_slots > 0 && src_base != dst_base;
                    let move_ref = arity_ref_slots > 0 && src_ref_base != dst_ref_base;
                    if move_reg || move_ref {
                        if move_reg {
                            ops.push(RegOp::MoveRange {
                                dst_base,
                                src_base,
                                count: arity_slots,
                            });
                        }
                        if move_ref {
                            ops.push(RegOp::MoveRefRange {
                                dst_base: dst_ref_base,
                                src_base: src_ref_base,
                                count: arity_ref_slots,
                            });
                        }
                        let jmp_ip = ops.len();
                        ops.push(RegOp::Jump { target: 0 });
                        branch_patches.push((jmp_ip, target_pc));
                    } else if ops.len() > cur_block_start_ip
                        && let Some((reg, imm)) = match ops.last().copied() {
                            Some(RegOp::I32AddImm { dst, src, imm }) if dst == src => {
                                Some((dst, imm))
                            }
                            Some(RegOp::I32SubImm { dst, src, imm }) if dst == src => {
                                imm.checked_neg().map(|step| (dst, step))
                            }
                            _ => None,
                        }
                    {
                        // Fuse `r = r + imm; br target` (common top-tested loop backedge).
                        ops.pop();
                        let jmp_ip = ops.len();
                        ops.push(RegOp::I32AddImmJump {
                            reg,
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
                    let base = slot_top(&vstack[..start]);
                    let ref_base = ref_top(&vstack[..start]);
                    let skip_ip = ops.len();
                    ops.push(RegOp::BrIfZero {
                        cond: cond_reg,
                        target: (skip_ip + 2) as u32,
                    });
                    ops.push(RegOp::Return {
                        base,
                        count: result_reg_slots,
                        ref_base,
                        ref_count: result_ref_slots,
                    });
                } else {
                    let idx = labels.len().checked_sub(depth + 1)?;
                    let label = &labels[idx];
                    let target_pc = if label.loop_ {
                        label.start
                    } else {
                        label.end + 1
                    };
                    let (arity, arity_slots, arity_ref_slots) = if label.loop_ {
                        (label.params.len(), label.param_slots, label.param_ref_slots)
                    } else {
                        (
                            label.results.len(),
                            label.result_slots,
                            label.result_ref_slots,
                        )
                    };
                    let src_start = vstack.len().checked_sub(arity)?;
                    let src_base = slot_top(&vstack[..src_start]);
                    let src_ref_base = ref_top(&vstack[..src_start]);
                    let dst_base = label.base_slot;
                    let dst_ref_base = label.base_ref_slot;
                    let move_reg = arity_slots > 0 && src_base != dst_base;
                    let move_ref = arity_ref_slots > 0 && src_ref_base != dst_ref_base;
                    if move_reg || move_ref {
                        let skip_ip = ops.len();
                        let skip_count = 1 + usize::from(move_reg) + usize::from(move_ref) + 1;
                        ops.push(RegOp::BrIfZero {
                            cond: cond_reg,
                            target: (skip_ip + skip_count) as u32,
                        });
                        if move_reg {
                            ops.push(RegOp::MoveRange {
                                dst_base,
                                src_base,
                                count: arity_slots,
                            });
                        }
                        if move_ref {
                            ops.push(RegOp::MoveRefRange {
                                dst_base: dst_ref_base,
                                src_base: src_ref_base,
                                count: arity_ref_slots,
                            });
                        }
                        let jmp_ip = ops.len();
                        ops.push(RegOp::Jump { target: 0 });
                        branch_patches.push((jmp_ip, target_pc));
                    } else if ops.len() > cur_block_start_ip
                        && let Some(step) = last_inc_step(&ops, cond_reg)
                    {
                        // Fuse countdown / step loops: `r = r + step; br_if target`.
                        ops.pop();
                        let br_ip = ops.len();
                        ops.push(RegOp::I32IncBrIfNonZero {
                            reg: cond_reg,
                            step,
                            target: 0,
                        });
                        branch_patches.push((br_ip, target_pc));
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
                        let base = slot_top(&vstack[..start]);
                        let ref_base = ref_top(&vstack[..start]);
                        let tramp_ip = ops.len() as u32;
                        ops.push(RegOp::Return {
                            base,
                            count: result_reg_slots,
                            ref_base,
                            ref_count: result_ref_slots,
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
                        let (arity, arity_slots, arity_ref_slots) = if label.loop_ {
                            (label.params.len(), label.param_slots, label.param_ref_slots)
                        } else {
                            (
                                label.results.len(),
                                label.result_slots,
                                label.result_ref_slots,
                            )
                        };
                        let src_start = vstack.len().checked_sub(arity)?;
                        let src_base = slot_top(&vstack[..src_start]);
                        let src_ref_base = ref_top(&vstack[..src_start]);
                        let dst_base = label.base_slot;
                        let dst_ref_base = label.base_ref_slot;
                        let move_reg = arity_slots > 0 && src_base != dst_base;
                        let move_ref = arity_ref_slots > 0 && src_ref_base != dst_ref_base;
                        if move_reg || move_ref {
                            let tramp_ip = ops.len() as u32;
                            if move_reg {
                                ops.push(RegOp::MoveRange {
                                    dst_base,
                                    src_base,
                                    count: arity_slots,
                                });
                            }
                            if move_ref {
                                ops.push(RegOp::MoveRefRange {
                                    dst_base: dst_ref_base,
                                    src_base: src_ref_base,
                                    count: arity_ref_slots,
                                });
                            }
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
                let base = slot_top(&vstack[..start]);
                let ref_base = ref_top(&vstack[..start]);
                ops.push(RegOp::Return {
                    base,
                    count: result_reg_slots,
                    ref_base,
                    ref_count: result_ref_slots,
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
                let mut arg_slots = 0u16;
                let mut arg_ref_slots = 0u16;
                for &t in &callee_ft.params {
                    let fty = FastType::from_val(t)?;
                    arg_slots = arg_slots.checked_add(fty.slots())?;
                    arg_ref_slots = arg_ref_slots.checked_add(fty.ref_slots())?;
                }
                flush_all(&mut vstack, &mut ops, &mut consts);
                let meta_at_call = push_meta(
                    pc,
                    &vstack,
                    &labels,
                    &mut blocks,
                    &mut stack_types,
                    &mut static_labels,
                )?;
                let arg_start = vstack.len().checked_sub(arg_count)?;
                let arg_base = slot_top(&vstack[..arg_start]);
                let arg_ref_base = ref_top(&vstack[..arg_start]);
                vstack.truncate(arg_start);
                let meta_after_call = push_meta(
                    pc + 1,
                    &vstack,
                    &labels,
                    &mut blocks,
                    &mut stack_types,
                    &mut static_labels,
                )?;
                let call_idx = call_sites.len() as u32;
                call_sites.push(CallSite {
                    func_idx: fi,
                    arg_base,
                    arg_slots,
                    arg_ref_base,
                    arg_ref_slots,
                    logical_arg_start: arg_start as u16,
                    meta_at_call,
                    meta_after_call,
                });
                ops.push(RegOp::Call { call_idx, src_off });
                let mut off = arg_base;
                let mut roff = arg_ref_base;
                for &rt in &callee_ft.results {
                    let ty = FastType::from_val(rt)?;
                    let s = if ty == FastType::Ref { roff } else { off };
                    vstack.push(VSlot {
                        src: VVal::Slot(s),
                        slot_off: s,
                        ty,
                    });
                    off += ty.slots();
                    roff += ty.ref_slots();
                }
                max_slots = max_slots.max(off as usize);
                max_ref_slots = max_ref_slots.max(roff as usize);
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
                let mut arg_slots = 0u16;
                let mut arg_ref_slots = 0u16;
                for &t in &callee_ft.params {
                    let fty = FastType::from_val(t)?;
                    arg_slots = arg_slots.checked_add(fty.slots())?;
                    arg_ref_slots = arg_ref_slots.checked_add(fty.ref_slots())?;
                }
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
                )?;
                let idx_slot = vstack.pop()?;
                let index_reg = idx_slot.slot_off;
                let arg_start = vstack.len().checked_sub(arg_count)?;
                let arg_base = slot_top(&vstack[..arg_start]);
                let arg_ref_base = ref_top(&vstack[..arg_start]);
                vstack.truncate(arg_start);
                let meta_after_call = push_meta(
                    pc + 1,
                    &vstack,
                    &labels,
                    &mut blocks,
                    &mut stack_types,
                    &mut static_labels,
                )?;
                let call_idx = call_indirect_sites.len() as u32;
                call_indirect_sites.push(CallIndirectSite {
                    type_idx,
                    table_idx,
                    index_reg,
                    arg_base,
                    arg_slots,
                    arg_ref_base,
                    arg_ref_slots,
                    logical_arg_start: arg_start as u16,
                    table_is_64,
                    meta_at_call,
                    meta_after_call,
                });
                ops.push(RegOp::CallIndirect { call_idx, src_off });
                let mut off = arg_base;
                let mut roff = arg_ref_base;
                for &rt in &callee_ft.results {
                    let ty = FastType::from_val(rt)?;
                    let s = if ty == FastType::Ref { roff } else { off };
                    vstack.push(VSlot {
                        src: VVal::Slot(s),
                        slot_off: s,
                        ty,
                    });
                    off += ty.slots();
                    roff += ty.ref_slots();
                }
                max_slots = max_slots.max(off as usize);
                max_ref_slots = max_ref_slots.max(roff as usize);
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
                let mut arg_slots = 0u16;
                let mut arg_ref_slots = 0u16;
                for &t in &callee_ft.params {
                    let fty = FastType::from_val(t)?;
                    arg_slots = arg_slots.checked_add(fty.slots())?;
                    arg_ref_slots = arg_ref_slots.checked_add(fty.ref_slots())?;
                }
                flush_all(&mut vstack, &mut ops, &mut consts);
                let meta_at_call = push_meta(
                    pc,
                    &vstack,
                    &labels,
                    &mut blocks,
                    &mut stack_types,
                    &mut static_labels,
                )?;
                let arg_start = vstack.len().checked_sub(arg_count)?;
                let arg_base = slot_top(&vstack[..arg_start]);
                let arg_ref_base = ref_top(&vstack[..arg_start]);
                let call_idx = call_sites.len() as u32;
                call_sites.push(CallSite {
                    func_idx: fi,
                    arg_base,
                    arg_slots,
                    arg_ref_base,
                    arg_ref_slots,
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
                let mut arg_slots = 0u16;
                let mut arg_ref_slots = 0u16;
                for &t in &callee_ft.params {
                    let fty = FastType::from_val(t)?;
                    arg_slots = arg_slots.checked_add(fty.slots())?;
                    arg_ref_slots = arg_ref_slots.checked_add(fty.ref_slots())?;
                }
                flush_all(&mut vstack, &mut ops, &mut consts);
                let meta_at_call = push_meta(
                    pc,
                    &vstack,
                    &labels,
                    &mut blocks,
                    &mut stack_types,
                    &mut static_labels,
                )?;
                let idx_slot = vstack.pop()?;
                let index_reg = idx_slot.slot_off;
                let arg_start = vstack.len().checked_sub(arg_count)?;
                let arg_base = slot_top(&vstack[..arg_start]);
                let arg_ref_base = ref_top(&vstack[..arg_start]);
                let call_idx = call_indirect_sites.len() as u32;
                call_indirect_sites.push(CallIndirectSite {
                    type_idx,
                    table_idx,
                    index_reg,
                    arg_base,
                    arg_slots,
                    arg_ref_base,
                    arg_ref_slots,
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
                // select (scalar, v128, or ref)
                let cond_v = vstack.pop()?;
                let b_v = vstack.pop()?;
                let a_v = vstack.pop()?;
                let base_tmp = slot_top(&vstack);
                let (dst, consumed, is_tee) =
                    try_direct_local_dst(pc, cur_block_ops, &mut vstack, &mut ops, a_v.ty);
                if a_v.ty == FastType::Ref {
                    let (VVal::Slot(ra) | VVal::Local(ra)) = a_v.src else {
                        return None;
                    };
                    let (VVal::Slot(rb) | VVal::Local(rb)) = b_v.src else {
                        return None;
                    };
                    let rc = mat(cond_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                    ops.push(RegOp::SelectRef {
                        dst,
                        a: ra,
                        b: rb,
                        cond: rc,
                    });
                } else {
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
                }
                finish_dst(
                    dst,
                    consumed,
                    is_tee,
                    a_v.ty,
                    &mut vstack,
                    &mut max_slots,
                    &mut max_ref_slots,
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
                let slot_off = if ty == FastType::Ref {
                    ref_top(&vstack)
                } else {
                    slot_top(&vstack)
                };
                vstack.push(VSlot {
                    src: VVal::Local(l_slot),
                    slot_off,
                    ty,
                });
                if ty == FastType::Ref {
                    max_ref_slots = max_ref_slots.max(slot_off as usize + 1);
                } else {
                    max_slots = max_slots.max(slot_off as usize + ty.slots() as usize);
                }
            }
            0x21 | 0x22 => {
                // local.set / local.tee
                let Immediate::Index(l) = insn.immediate else {
                    return None;
                };
                let l_slot = *local_slots.get(l as usize)?;
                let val = vstack.pop()?;
                let is_ref = val.ty == FastType::Ref;
                flush_local(l_slot, is_ref, &mut vstack, &mut ops);
                match val.src {
                    VVal::Local(s) if s == l_slot => {}
                    VVal::Slot(s) | VVal::Local(s) => {
                        if is_ref {
                            ops.push(RegOp::MovRef {
                                dst: l_slot,
                                src: s,
                            });
                        } else if val.ty == FastType::V128 {
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
                    VVal::ShlImm { src, shift } => {
                        ops.push(RegOp::I32ShlImm {
                            dst: l_slot,
                            src,
                            shift: u32::from(shift),
                        });
                    }
                    VVal::AddShl { base, idx, shift } => {
                        let tmp = slot_top(&vstack);
                        max_slots = max_slots.max(tmp as usize + 1);
                        ops.push(RegOp::I32ShlImm {
                            dst: tmp,
                            src: idx,
                            shift: u32::from(shift),
                        });
                        ops.push(RegOp::I32Add {
                            dst: l_slot,
                            a: base,
                            b: tmp,
                        });
                    }
                }
                if insn.opcode == 0x22 {
                    let slot_off = if is_ref {
                        ref_top(&vstack)
                    } else {
                        slot_top(&vstack)
                    };
                    vstack.push(VSlot {
                        src: VVal::Local(l_slot),
                        slot_off,
                        ty: val.ty,
                    });
                    if is_ref {
                        max_ref_slots = max_ref_slots.max(slot_off as usize + 1);
                    } else {
                        max_slots = max_slots.max(slot_off as usize + val.ty.slots() as usize);
                    }
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
                if ty == FastType::Ref {
                    ops.push(RegOp::GlobalGetRef { dst, global: gi });
                } else if ty == FastType::V128 {
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
                    &mut max_ref_slots,
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
                if val.ty == FastType::Ref {
                    let (VVal::Slot(src) | VVal::Local(src)) = val.src else {
                        return None;
                    };
                    ops.push(RegOp::GlobalSetRef { src, global: gi });
                } else {
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
            }
            0x25 => {
                // table.get
                let Immediate::Index(table) = insn.immediate else {
                    return None;
                };
                let idx_v = vstack.pop()?;
                let r_idx = mat(
                    idx_v.src,
                    slot_top(&vstack),
                    &mut max_slots,
                    &mut ops,
                    &mut consts,
                );
                let (dst, consumed, is_tee) =
                    try_direct_local_dst(pc, cur_block_ops, &mut vstack, &mut ops, FastType::Ref);
                ops.push(RegOp::TableGet {
                    dst,
                    idx: r_idx,
                    table,
                    src_off,
                });
                finish_dst(
                    dst,
                    consumed,
                    is_tee,
                    FastType::Ref,
                    &mut vstack,
                    &mut max_slots,
                    &mut max_ref_slots,
                    &mut cur_block_ops,
                    &mut pc,
                );
            }
            0x26 => {
                // table.set
                let Immediate::Index(table) = insn.immediate else {
                    return None;
                };
                let val_v = vstack.pop()?;
                let idx_v = vstack.pop()?;
                let (VVal::Slot(r_val) | VVal::Local(r_val)) = val_v.src else {
                    return None;
                };
                let r_idx = mat(
                    idx_v.src,
                    slot_top(&vstack),
                    &mut max_slots,
                    &mut ops,
                    &mut consts,
                );
                ops.push(RegOp::TableSet {
                    idx: r_idx,
                    val: r_val,
                    table,
                    src_off,
                });
            }
            0xd0 => {
                // ref.null heap
                let Immediate::Heap(heap) = insn.immediate else {
                    return None;
                };
                let heap_idx = if let Some(pos) = ref_heaps.iter().position(|&h| h == heap) {
                    pos as u32
                } else {
                    let idx = u32::try_from(ref_heaps.len()).ok()?;
                    ref_heaps.push(heap);
                    idx
                };
                let (dst, consumed, is_tee) =
                    try_direct_local_dst(pc, cur_block_ops, &mut vstack, &mut ops, FastType::Ref);
                ops.push(RegOp::RefNull { dst, heap_idx });
                finish_dst(
                    dst,
                    consumed,
                    is_tee,
                    FastType::Ref,
                    &mut vstack,
                    &mut max_slots,
                    &mut max_ref_slots,
                    &mut cur_block_ops,
                    &mut pc,
                );
            }
            0xd1 => {
                // ref.is_null
                let val_v = vstack.pop()?;
                let (VVal::Slot(src) | VVal::Local(src)) = val_v.src else {
                    return None;
                };
                let (dst, consumed, is_tee) =
                    try_direct_local_dst(pc, cur_block_ops, &mut vstack, &mut ops, FastType::I32);
                ops.push(RegOp::RefIsNull { dst, src });
                finish_dst(
                    dst,
                    consumed,
                    is_tee,
                    FastType::I32,
                    &mut vstack,
                    &mut max_slots,
                    &mut max_ref_slots,
                    &mut cur_block_ops,
                    &mut pc,
                );
            }
            0xd2 => {
                // ref.func idx
                let Immediate::Index(func_idx) = insn.immediate else {
                    return None;
                };
                let (dst, consumed, is_tee) =
                    try_direct_local_dst(pc, cur_block_ops, &mut vstack, &mut ops, FastType::Ref);
                ops.push(RegOp::RefFunc { dst, func_idx });
                finish_dst(
                    dst,
                    consumed,
                    is_tee,
                    FastType::Ref,
                    &mut vstack,
                    &mut max_slots,
                    &mut max_ref_slots,
                    &mut cur_block_ops,
                    &mut pc,
                );
            }
            0xd3 => {
                // ref.eq
                let b_v = vstack.pop()?;
                let a_v = vstack.pop()?;
                let (VVal::Slot(ra) | VVal::Local(ra)) = a_v.src else {
                    return None;
                };
                let (VVal::Slot(rb) | VVal::Local(rb)) = b_v.src else {
                    return None;
                };
                let (dst, consumed, is_tee) =
                    try_direct_local_dst(pc, cur_block_ops, &mut vstack, &mut ops, FastType::I32);
                ops.push(RegOp::RefEq { dst, a: ra, b: rb });
                finish_dst(
                    dst,
                    consumed,
                    is_tee,
                    FastType::I32,
                    &mut vstack,
                    &mut max_slots,
                    &mut max_ref_slots,
                    &mut cur_block_ops,
                    &mut pc,
                );
            }
            0xd4 => {
                // ref.as_non_null
                let val_v = vstack.pop()?;
                let (VVal::Slot(src) | VVal::Local(src)) = val_v.src else {
                    return None;
                };
                let (dst, consumed, is_tee) =
                    try_direct_local_dst(pc, cur_block_ops, &mut vstack, &mut ops, FastType::Ref);
                ops.push(RegOp::RefAsNonNull { dst, src, src_off });
                finish_dst(
                    dst,
                    consumed,
                    is_tee,
                    FastType::Ref,
                    &mut vstack,
                    &mut max_slots,
                    &mut max_ref_slots,
                    &mut cur_block_ops,
                    &mut pc,
                );
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
                    let fast_mem32 = memory == 0 && mem0_unshared && !mem0_is_64;
                    let off32_opt = u32::try_from(offset).ok();
                    // Fast path for lazy `VVal::AddShl` / `VVal::ShlImm` scaled store addresses!
                    let fused_scaled_store = if fast_mem32 && let Some(off32) = off32_opt {
                        match (insn.opcode, addr_v.src) {
                            (0x36 | 0x38, VVal::ShlImm { src: idx, shift: 2 }) => {
                                let r_val =
                                    mat(val_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                                ops.push(RegOp::StoreI32Shl2 {
                                    idx,
                                    val: r_val,
                                    offset: off32,
                                    src_off,
                                });
                                true
                            }
                            (
                                0x36 | 0x38,
                                VVal::AddShl {
                                    base,
                                    idx,
                                    shift: 2,
                                },
                            ) => {
                                let r_val =
                                    mat(val_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                                ops.push(RegOp::StoreI32Shl2Add {
                                    base,
                                    idx,
                                    val: r_val,
                                    offset: off32,
                                    src_off,
                                });
                                true
                            }
                            (
                                0x37 | 0x39,
                                VVal::AddShl {
                                    base,
                                    idx,
                                    shift: 3,
                                },
                            ) => {
                                let r_val =
                                    mat(val_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                                ops.push(RegOp::StoreI64Shl3Add {
                                    base,
                                    idx,
                                    val: r_val,
                                    offset: off32,
                                    src_off,
                                });
                                true
                            }
                            _ => false,
                        }
                    } else {
                        false
                    };
                    if fused_scaled_store {
                        // already emitted
                    } else {
                        // Peephole: fuse preceding `I32AddImm` or `I32ShlImm(shift=2)` on addr slot.
                        let fused_shl2_op = if fast_mem32
                            && let Some(off32) = off32_opt
                            && matches!(insn.opcode, 0x36 | 0x38)
                            && let VVal::Slot(r_addr) = addr_v.src
                            && r_addr >= local_reg_slots
                            && val_v.src != VVal::Slot(r_addr)
                            && ops.len() > cur_block_start_ip
                            && let Some(RegOp::I32ShlImm { dst, src, shift: 2 }) =
                                ops.last().copied()
                            && dst == r_addr
                        {
                            ops.pop();
                            Some((src, off32))
                        } else {
                            None
                        };
                        let fused_add_imm = if fused_shl2_op.is_none()
                            && fast_mem32
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
                        if let Some((idx, off32)) = fused_shl2_op {
                            let val_tmp = base_tmp.max(idx) + 1;
                            let r_val =
                                mat(val_v.src, val_tmp, &mut max_slots, &mut ops, &mut consts);
                            ops.push(RegOp::StoreI32Shl2 {
                                idx,
                                val: r_val,
                                offset: off32,
                                src_off,
                            });
                        } else if let Some((base_reg, imm)) = fused_add_imm {
                            let val_tmp = base_tmp.max(base_reg) + 1;
                            let r_val =
                                mat(val_v.src, val_tmp, &mut max_slots, &mut ops, &mut consts);
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
                            let fused_store_imm = if memory == 0
                                && mem0_unshared
                                && let VVal::Imm32(imm_val) = val_v.src
                            {
                                if matches!(insn.opcode, 0x36 | 0x38)
                                    && let Ok(off16) = u16::try_from(offset)
                                {
                                    ops.push(RegOp::StoreI32Imm {
                                        addr: r_addr,
                                        val: imm_val,
                                        offset: off16,
                                        src_off,
                                    });
                                    true
                                } else if insn.opcode == 0x3a
                                    && let Some(off32) = off32_opt
                                {
                                    ops.push(RegOp::StoreI32_8Imm {
                                        addr: r_addr,
                                        val: imm_val as u8,
                                        offset: off32,
                                        src_off,
                                    });
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            };
                            if !fused_store_imm {
                                let r_val = mat(
                                    val_v.src,
                                    base_tmp + 1,
                                    &mut max_slots,
                                    &mut ops,
                                    &mut consts,
                                );
                                if memory == 0
                                    && mem0_unshared
                                    && let Some(off32) = off32_opt
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
                        }
                    }
                } else {
                    let addr_v = vstack.pop()?;
                    let base_tmp = slot_top(&vstack);
                    let fast_mem32 = memory == 0 && mem0_unshared && !mem0_is_64;
                    let off32_opt = u32::try_from(offset).ok();
                    let lazy_fused = if fast_mem32 && let Some(off32) = off32_opt {
                        match (insn.opcode, addr_v.src) {
                            (0x28 | 0x2a, VVal::ShlImm { src: idx, shift: 2 }) => {
                                let (dst, consumed, is_tee) = try_direct_local_dst(
                                    pc,
                                    cur_block_ops,
                                    &mut vstack,
                                    &mut ops,
                                    ty,
                                );
                                ops.push(RegOp::LoadI32Shl2 {
                                    dst,
                                    idx,
                                    offset: off32,
                                    src_off,
                                });
                                Some((dst, consumed, is_tee))
                            }
                            (
                                0x28 | 0x2a,
                                VVal::AddShl {
                                    base,
                                    idx,
                                    shift: 2,
                                },
                            ) => {
                                let (dst, consumed, is_tee) = try_direct_local_dst(
                                    pc,
                                    cur_block_ops,
                                    &mut vstack,
                                    &mut ops,
                                    ty,
                                );
                                ops.push(RegOp::LoadI32Shl2Add {
                                    dst,
                                    base,
                                    idx,
                                    offset: off32,
                                    src_off,
                                });
                                Some((dst, consumed, is_tee))
                            }
                            (
                                0x29 | 0x2b,
                                VVal::AddShl {
                                    base,
                                    idx,
                                    shift: 3,
                                },
                            ) => {
                                let (dst, consumed, is_tee) = try_direct_local_dst(
                                    pc,
                                    cur_block_ops,
                                    &mut vstack,
                                    &mut ops,
                                    ty,
                                );
                                ops.push(RegOp::LoadI64Shl3Add {
                                    dst,
                                    base,
                                    idx,
                                    offset: off32,
                                    src_off,
                                });
                                Some((dst, consumed, is_tee))
                            }
                            _ => None,
                        }
                    } else {
                        None
                    };
                    let (dst, consumed, is_tee) = if let Some(res) = lazy_fused {
                        res
                    } else {
                        let r_addr =
                            mat(addr_v.src, base_tmp, &mut max_slots, &mut ops, &mut consts);
                        let (dst, consumed, is_tee) =
                            try_direct_local_dst(pc, cur_block_ops, &mut vstack, &mut ops, ty);
                        if memory == 0
                            && mem0_unshared
                            && let Some(off32) = off32_opt
                        {
                            let can_fuse_addr = !mem0_is_64
                                && matches!(insn.opcode, 0x28 | 0x29 | 0x2a | 0x2b | 0x2d)
                                && r_addr >= local_reg_slots
                                && ops.len() > cur_block_start_ip;
                            let fused_shl2 = if can_fuse_addr
                                && matches!(insn.opcode, 0x28 | 0x2a)
                                && let Some(RegOp::I32ShlImm {
                                    dst: shl_dst,
                                    src: idx,
                                    shift: 2,
                                }) = ops.last().copied()
                                && shl_dst == r_addr
                            {
                                ops.pop();
                                Some(idx)
                            } else {
                                None
                            };
                            let fused_addr = if fused_shl2.is_none()
                                && can_fuse_addr
                                && let Some(RegOp::I32Add { dst: add_dst, a, b }) =
                                    ops.last().copied()
                                && add_dst == r_addr
                            {
                                ops.pop();
                                Some((a, b))
                            } else {
                                None
                            };
                            let fused_imm = if fused_shl2.is_none()
                                && fused_addr.is_none()
                                && can_fuse_addr
                                && matches!(insn.opcode, 0x28 | 0x2a | 0x2d)
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
                            match (insn.opcode, fused_shl2, fused_addr, fused_imm) {
                                (0x28 | 0x2a, Some(idx), _, _) => ops.push(RegOp::LoadI32Shl2 {
                                    dst,
                                    idx,
                                    offset: off32,
                                    src_off,
                                }),
                                (0x28 | 0x2a, None, Some((a, b)), _) => {
                                    ops.push(RegOp::LoadI32AddAddr {
                                        dst,
                                        a,
                                        b,
                                        offset: off32,
                                        src_off,
                                    })
                                }
                                (0x29 | 0x2b, None, Some((a, b)), _) => {
                                    ops.push(RegOp::LoadI64AddAddr {
                                        dst,
                                        a,
                                        b,
                                        offset: off32,
                                        src_off,
                                    })
                                }
                                (0x2d, None, Some((a, b)), _) => ops.push(RegOp::LoadU8AddAddr {
                                    dst,
                                    a,
                                    b,
                                    offset: off32,
                                    src_off,
                                }),
                                (0x28 | 0x2a, None, None, Some((base, imm))) => {
                                    ops.push(RegOp::LoadI32AddImm {
                                        dst,
                                        addr: base,
                                        imm,
                                        src_off,
                                    })
                                }
                                (0x2d, None, None, Some((base, imm))) => {
                                    ops.push(RegOp::LoadU8AddImm {
                                        dst,
                                        addr: base,
                                        imm,
                                        src_off,
                                    })
                                }
                                (0x28 | 0x2a, None, None, None) => ops.push(RegOp::LoadI32 {
                                    dst,
                                    addr: r_addr,
                                    offset: off32,
                                    src_off,
                                }),
                                (0x29 | 0x2b, None, None, None) => ops.push(RegOp::LoadI64 {
                                    dst,
                                    addr: r_addr,
                                    offset: off32,
                                    src_off,
                                }),
                                (0x2d, None, None, None) => ops.push(RegOp::LoadI32U8 {
                                    dst,
                                    addr: r_addr,
                                    offset: off32,
                                    src_off,
                                }),
                                (0x2f, None, None, None) => ops.push(RegOp::LoadI32U16 {
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
                        (dst, consumed, is_tee)
                    };
                    finish_dst(
                        dst,
                        consumed,
                        is_tee,
                        ty,
                        &mut vstack,
                        &mut max_slots,
                        &mut max_ref_slots,
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
                    &mut max_ref_slots,
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
                    &mut max_ref_slots,
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
                    &mut max_ref_slots,
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
                            let param_ref_slots = params.iter().map(|t| t.ref_slots()).sum();
                            let result_slots = results.iter().map(|t| t.slots()).sum();
                            let result_ref_slots = results.iter().map(|t| t.ref_slots()).sum();
                            let (end, other) = module.data.control_get(body, pc + 1)?;
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
                            let base_slot = slot_top(&vstack[..base_height]);
                            let base_ref_slot = ref_top(&vstack[..base_height]);
                            let prefix = copy_prefix(&vstack, base_height, &mut prefix_copies)?;
                            labels.try_reserve(1).ok()?;
                            labels.push(CompileLabel {
                                start: pc + 2,
                                end,
                                other,
                                loop_: false,
                                base_height,
                                base_slot,
                                base_ref_slot,
                                prefix,
                                params,
                                results,
                                param_slots,
                                param_ref_slots,
                                result_slots,
                                result_ref_slots,
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
                        &mut max_ref_slots,
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
                                } else if let Some((var_src, eff_cmp, imm)) =
                                    if let VVal::Imm32(imm) = b_v.src {
                                        Some((a_v.src, cmp, imm))
                                    } else if let VVal::Imm32(imm) = a_v.src {
                                        Some((b_v.src, swap_i32_cmp(cmp), imm))
                                    } else {
                                        None
                                    }
                                {
                                    let ra = mat(
                                        var_src,
                                        base_tmp,
                                        &mut max_slots,
                                        &mut ops,
                                        &mut consts,
                                    );
                                    if ops.len() > cur_block_start_ip
                                        && let Some(step) = last_inc_step(&ops, ra)
                                    {
                                        if eff_cmp == 1 && imm == 0 {
                                            ops.pop();
                                            let ip = ops.len();
                                            ops.push(RegOp::I32IncBrIfNonZero {
                                                reg: ra,
                                                step,
                                                target: 0,
                                            });
                                            ip
                                        } else if let Ok(step8) = i8::try_from(step)
                                            && let Some(imm16) = encode_cmp_imm16(eff_cmp, imm)
                                        {
                                            ops.pop();
                                            let ip = ops.len();
                                            ops.push(RegOp::I32IncCmpImmBrIf {
                                                cmp: eff_cmp,
                                                step: step8,
                                                reg: ra,
                                                imm: imm16,
                                                target: 0,
                                            });
                                            ip
                                        } else {
                                            let ip = ops.len();
                                            ops.push(RegOp::I32CmpImmBrIf {
                                                cmp: eff_cmp,
                                                a: ra,
                                                imm: imm as i32,
                                                target: 0,
                                            });
                                            ip
                                        }
                                    } else {
                                        let ip = ops.len();
                                        ops.push(RegOp::I32CmpImmBrIf {
                                            cmp: eff_cmp,
                                            a: ra,
                                            imm: imm as i32,
                                            target: 0,
                                        });
                                        ip
                                    }
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
                                    if matches!(cmp, 1..=3)
                                        && rb != ra
                                        && ops.len() > cur_block_start_ip
                                        && let Some(step_i32) = last_inc_step(&ops, ra)
                                        && let Ok(step) = i16::try_from(step_i32)
                                    {
                                        ops.pop();
                                        let ip = ops.len();
                                        match cmp {
                                            3 => ops.push(RegOp::I32IncLtUBrIf {
                                                reg: ra,
                                                limit: rb,
                                                step,
                                                target: 0,
                                            }),
                                            2 => ops.push(RegOp::I32IncLtSBrIf {
                                                reg: ra,
                                                limit: rb,
                                                step,
                                                target: 0,
                                            }),
                                            _ => ops.push(RegOp::I32IncNeBrIf {
                                                reg: ra,
                                                limit: rb,
                                                step,
                                                target: 0,
                                            }),
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
                            let param_ref_slots = params.iter().map(|t| t.ref_slots()).sum();
                            let result_slots = results.iter().map(|t| t.slots()).sum();
                            let result_ref_slots = results.iter().map(|t| t.ref_slots()).sum();
                            let (end, other) = module.data.control_get(body, pc + 1)?;
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
                            let base_slot = slot_top(&vstack[..base_height]);
                            let base_ref_slot = ref_top(&vstack[..base_height]);
                            let prefix = copy_prefix(&vstack, base_height, &mut prefix_copies)?;
                            labels.try_reserve(1).ok()?;
                            labels.push(CompileLabel {
                                start: pc + 2,
                                end,
                                other,
                                loop_: false,
                                base_height,
                                base_slot,
                                base_ref_slot,
                                prefix,
                                params,
                                results,
                                param_slots,
                                param_ref_slots,
                                result_slots,
                                result_ref_slots,
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

                    // Lazy scaled-address expressions over local registers (`local << 2|3` and
                    // `local_base + (local_idx << 2|3)`) for zero-overhead fused indexed loads/stores!
                    let next_is_local_set = pc + 1 < n
                        && !is_block_start[pc + 1]
                        && cur_block_ops < MAX_BLOCK_OPS
                        && matches!(code.instructions[pc + 1].opcode, 0x21 | 0x22);
                    if !next_is_local_set {
                        let lazy_expr = match (insn.opcode, a_v.src, b_v.src) {
                            (0x74, VVal::Local(src), VVal::Imm32(c)) if matches!(c & 31, 2 | 3) => {
                                Some(VVal::ShlImm {
                                    src,
                                    shift: (c & 31) as u8,
                                })
                            }
                            (0x6c, VVal::Local(src), VVal::Imm32(4 | 8))
                            | (0x6c, VVal::Imm32(4 | 8), VVal::Local(src)) => {
                                let shift = if matches!(b_v.src, VVal::Imm32(4))
                                    || matches!(a_v.src, VVal::Imm32(4))
                                {
                                    2u8
                                } else {
                                    3u8
                                };
                                Some(VVal::ShlImm { src, shift })
                            }
                            (
                                0x6a,
                                VVal::Local(base),
                                VVal::ShlImm {
                                    src: idx,
                                    shift: shift @ (2 | 3),
                                },
                            )
                            | (
                                0x6a,
                                VVal::ShlImm {
                                    src: idx,
                                    shift: shift @ (2 | 3),
                                },
                                VVal::Local(base),
                            ) => Some(VVal::AddShl { base, idx, shift }),
                            _ => None,
                        };
                        if let Some(expr) = lazy_expr {
                            let slot_off = slot_top(&vstack);
                            vstack.push(VSlot {
                                src: expr,
                                slot_off,
                                ty: FastType::I32,
                            });
                            max_slots = max_slots.max(slot_off as usize + 1);
                            pc += 1;
                            continue;
                        }
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
                            if c > 1 && c.is_power_of_two() {
                                ops.push(RegOp::I32ShlImm {
                                    dst,
                                    src: ra,
                                    shift: c.trailing_zeros(),
                                });
                            } else {
                                ops.push(RegOp::I32MulImm {
                                    dst,
                                    src: ra,
                                    imm: c as i32,
                                });
                            }
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
                                Some(RegOp::LoadI32 {
                                    dst: ldst,
                                    addr,
                                    offset,
                                    src_off: l_src_off,
                                }) if ldst >= local_reg_slots
                                    && ((rb == ldst && ra == dst) || (ra == ldst && rb == dst))
                                    && addr != dst =>
                                {
                                    ops.pop();
                                    ops.push(RegOp::LoadI32Add {
                                        acc: dst,
                                        addr,
                                        offset,
                                        src_off: l_src_off,
                                    });
                                    true
                                }
                                Some(RegOp::LoadI32Shl2Add {
                                    dst: ldst,
                                    base,
                                    idx,
                                    offset,
                                    src_off: l_src_off,
                                }) if ldst >= local_reg_slots
                                    && ((rb == ldst && ra == dst) || (ra == ldst && rb == dst))
                                    && base != dst
                                    && idx != dst =>
                                {
                                    ops.pop();
                                    ops.push(RegOp::LoadI32Shl2AddAcc {
                                        acc: dst,
                                        base,
                                        idx,
                                        offset,
                                        src_off: l_src_off,
                                    });
                                    true
                                }
                                _ => false,
                            }
                        } else if in_cur_block && insn.opcode == 0x92 {
                            match ops.last().copied() {
                                Some(RegOp::F32Mul {
                                    dst: mtmp,
                                    a: ma,
                                    b: mb,
                                }) if mtmp >= local_reg_slots && rb == mtmp && ra != mtmp => {
                                    ops.pop();
                                    ops.push(RegOp::F32MulAdd {
                                        dst,
                                        acc: ra,
                                        a: ma,
                                        b: mb,
                                    });
                                    true
                                }
                                Some(RegOp::LoadI32 {
                                    dst: ldst,
                                    addr,
                                    offset,
                                    src_off: l_src_off,
                                }) if ldst >= local_reg_slots
                                    && rb == ldst
                                    && ra == dst
                                    && addr != dst =>
                                {
                                    ops.pop();
                                    ops.push(RegOp::LoadF32Add {
                                        acc: dst,
                                        addr,
                                        offset,
                                        src_off: l_src_off,
                                    });
                                    true
                                }
                                _ => false,
                            }
                        } else if in_cur_block && insn.opcode == 0xa0 {
                            match ops.last().copied() {
                                Some(RegOp::F64Mul {
                                    dst: mtmp,
                                    a: ma,
                                    b: mb,
                                }) if mtmp >= local_reg_slots && rb == mtmp && ra != mtmp => {
                                    ops.pop();
                                    ops.push(RegOp::F64MulAdd {
                                        dst,
                                        acc: ra,
                                        a: ma,
                                        b: mb,
                                    });
                                    true
                                }
                                Some(RegOp::LoadI64 {
                                    dst: ldst,
                                    addr,
                                    offset,
                                    src_off: l_src_off,
                                }) if ldst >= local_reg_slots
                                    && rb == ldst
                                    && ra == dst
                                    && addr != dst =>
                                {
                                    ops.pop();
                                    ops.push(RegOp::LoadF64Add {
                                        acc: dst,
                                        addr,
                                        offset,
                                        src_off: l_src_off,
                                    });
                                    true
                                }
                                Some(RegOp::LoadI64Shl3Add {
                                    dst: ldst,
                                    base,
                                    idx,
                                    offset,
                                    src_off: l_src_off,
                                }) if ldst >= local_reg_slots
                                    && rb == ldst
                                    && ra == dst
                                    && base != dst
                                    && idx != dst =>
                                {
                                    ops.pop();
                                    ops.push(RegOp::LoadF64Shl3AddAcc {
                                        acc: dst,
                                        base,
                                        idx,
                                        offset,
                                        src_off: l_src_off,
                                    });
                                    true
                                }
                                _ => false,
                            }
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
                        &mut max_ref_slots,
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
                            &mut max_ref_slots,
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
                            &mut max_ref_slots,
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
                    if mask.iter().any(|&m| m >= 32) {
                        return None;
                    }
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
                        &mut max_ref_slots,
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
                    let rop = match insn.opcode {
                        0xfd0f => RegOp::V128Splat8 { dst, src: ra },
                        0xfd10 => RegOp::V128Splat16 { dst, src: ra },
                        0xfd11 | 0xfd13 => RegOp::V128Splat32 { dst, src: ra },
                        0xfd12 | 0xfd14 => RegOp::V128Splat64 { dst, src: ra },
                        0xfd1b | 0xfd1f if lane < 4 => RegOp::SimdExtract32 { dst, src: ra, lane },
                        0xfd4d => RegOp::V128Not { dst, src: ra },
                        0xfd53 => RegOp::V128AnyTrue { dst, src: ra },
                        0xfd64 => RegOp::I8x16Bitmask { dst, src: ra },
                        0xfda4 => RegOp::I32x4Bitmask { dst, src: ra },
                        0xfde0 => RegOp::F32x4Abs { dst, src: ra },
                        0xfde1 => RegOp::F32x4Neg { dst, src: ra },
                        0xfde3 => RegOp::F32x4Sqrt { dst, src: ra },
                        0xfdec => RegOp::F64x2Abs { dst, src: ra },
                        0xfded => RegOp::F64x2Neg { dst, src: ra },
                        0xfdef => RegOp::F64x2Sqrt { dst, src: ra },
                        0xfdfa => RegOp::F32x4ConvertI32x4S { dst, src: ra },
                        0xfdfb => RegOp::F32x4ConvertI32x4U { dst, src: ra },
                        _ => RegOp::SimdUnary {
                            op: insn.opcode,
                            dst,
                            src: ra,
                            lane,
                        },
                    };
                    ops.push(rop);
                    finish_dst(
                        dst,
                        consumed,
                        is_tee,
                        out_ty,
                        &mut vstack,
                        &mut max_slots,
                        &mut max_ref_slots,
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
                    if insn.opcode == 0xfde4
                        && ops.len() > cur_block_start_ip
                        && let Some(RegOp::F32x4Mul {
                            dst: mtmp,
                            a: ma,
                            b: mb,
                        }) = ops.last().copied()
                        && mtmp >= local_reg_slots
                        && rb == mtmp
                        && ra != mtmp
                    {
                        ops.pop();
                        ops.push(RegOp::F32x4MulAdd {
                            dst,
                            acc: ra,
                            a: ma,
                            b: mb,
                        });
                    } else if insn.opcode == 0xfdf0
                        && ops.len() > cur_block_start_ip
                        && let Some(RegOp::F64x2Mul {
                            dst: mtmp,
                            a: ma,
                            b: mb,
                        }) = ops.last().copied()
                        && mtmp >= local_reg_slots
                        && rb == mtmp
                        && ra != mtmp
                    {
                        ops.pop();
                        ops.push(RegOp::F64x2MulAdd {
                            dst,
                            acc: ra,
                            a: ma,
                            b: mb,
                        });
                    } else {
                        let rop = match insn.opcode {
                            0xfd37..=0xfd40 => RegOp::I32x4Cmp {
                                cmp: (insn.opcode - 0xfd37) as u8,
                                dst,
                                a: ra,
                                b: rb,
                            },
                            0xfd41..=0xfd46 => RegOp::F32x4Cmp {
                                cmp: (insn.opcode - 0xfd41) as u8,
                                dst,
                                a: ra,
                                b: rb,
                            },
                            0xfd47..=0xfd4c => RegOp::F64x2Cmp {
                                cmp: (insn.opcode - 0xfd47) as u8,
                                dst,
                                a: ra,
                                b: rb,
                            },
                            0xfd4e => RegOp::V128And { dst, a: ra, b: rb },
                            0xfd4f => RegOp::V128AndNot { dst, a: ra, b: rb },
                            0xfd50 => RegOp::V128Or { dst, a: ra, b: rb },
                            0xfd51 => RegOp::V128Xor { dst, a: ra, b: rb },
                            0xfd6e => RegOp::I8x16Add { dst, a: ra, b: rb },
                            0xfd71 => RegOp::I8x16Sub { dst, a: ra, b: rb },
                            0xfd8e => RegOp::I16x8Add { dst, a: ra, b: rb },
                            0xfd91 => RegOp::I16x8Sub { dst, a: ra, b: rb },
                            0xfd95 => RegOp::I16x8Mul { dst, a: ra, b: rb },
                            0xfdab..=0xfdad => RegOp::I32x4Shift {
                                kind: (insn.opcode - 0xfdab) as u8,
                                dst,
                                a: ra,
                                sh: rb,
                            },
                            0xfdae => RegOp::I32x4Add { dst, a: ra, b: rb },
                            0xfdb1 => RegOp::I32x4Sub { dst, a: ra, b: rb },
                            0xfdb5 => RegOp::I32x4Mul { dst, a: ra, b: rb },
                            0xfdce => RegOp::I64x2Add { dst, a: ra, b: rb },
                            0xfdd1 => RegOp::I64x2Sub { dst, a: ra, b: rb },
                            0xfdd5 => RegOp::I64x2Mul { dst, a: ra, b: rb },
                            0xfde4 => RegOp::F32x4Add { dst, a: ra, b: rb },
                            0xfde5 => RegOp::F32x4Sub { dst, a: ra, b: rb },
                            0xfde6 => RegOp::F32x4Mul { dst, a: ra, b: rb },
                            0xfde7 => RegOp::F32x4Div { dst, a: ra, b: rb },
                            0xfde8 | 0xfa0d => RegOp::F32x4Min { dst, a: ra, b: rb },
                            0xfde9 | 0xfa0e => RegOp::F32x4Max { dst, a: ra, b: rb },
                            0xfdea => RegOp::F32x4PMin { dst, a: ra, b: rb },
                            0xfdeb => RegOp::F32x4PMax { dst, a: ra, b: rb },
                            0xfdf0 => RegOp::F64x2Add { dst, a: ra, b: rb },
                            0xfdf1 => RegOp::F64x2Sub { dst, a: ra, b: rb },
                            0xfdf2 => RegOp::F64x2Mul { dst, a: ra, b: rb },
                            0xfdf3 => RegOp::F64x2Div { dst, a: ra, b: rb },
                            0xfdf4 | 0xfa0f => RegOp::F64x2Min { dst, a: ra, b: rb },
                            0xfdf5 | 0xfa10 => RegOp::F64x2Max { dst, a: ra, b: rb },
                            0xfdf6 => RegOp::F64x2PMin { dst, a: ra, b: rb },
                            0xfdf7 => RegOp::F64x2PMax { dst, a: ra, b: rb },
                            _ => RegOp::SimdBinary {
                                op: insn.opcode,
                                dst,
                                a: ra,
                                b: rb,
                                lane,
                            },
                        };
                        ops.push(rop);
                    }
                    finish_dst(
                        dst,
                        consumed,
                        is_tee,
                        FastType::V128,
                        &mut vstack,
                        &mut max_slots,
                        &mut max_ref_slots,
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
                    let rop = match insn.opcode {
                        0xfd52 | 0xfa09..=0xfa0c => RegOp::V128Bitselect {
                            dst,
                            a: ra,
                            b: rb,
                            c: rc,
                        },
                        0xfa05 => RegOp::F32x4RelaxedMadd {
                            dst,
                            a: ra,
                            b: rb,
                            c: rc,
                        },
                        0xfa06 => RegOp::F32x4RelaxedNmadd {
                            dst,
                            a: ra,
                            b: rb,
                            c: rc,
                        },
                        0xfa07 => RegOp::F64x2RelaxedMadd {
                            dst,
                            a: ra,
                            b: rb,
                            c: rc,
                        },
                        0xfa08 => RegOp::F64x2RelaxedNmadd {
                            dst,
                            a: ra,
                            b: rb,
                            c: rc,
                        },
                        _ => RegOp::SimdTernary {
                            op: insn.opcode,
                            dst,
                            a: ra,
                            b: rb,
                            c: rc,
                        },
                    };
                    ops.push(rop);
                    finish_dst(
                        dst,
                        consumed,
                        is_tee,
                        FastType::V128,
                        &mut vstack,
                        &mut max_slots,
                        &mut max_ref_slots,
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

    if max_slots > MAX_FN_SLOTS || max_ref_slots > MAX_FN_SLOTS {
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
            | RegOp::I32IncNeBrIf { target, .. }
            | RegOp::I32IncBrIfNonZero { target, .. }
            | RegOp::I32IncCmpImmBrIf { target, .. }
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

    let pure_leaf = !fuel_blocks
        && !has_loops_or_calls
        && max_slots <= 16
        && max_ref_slots == 0
        && !ops.is_empty()
        && local_types.iter().all(|t| {
            matches!(
                t,
                FastType::I32 | FastType::I64 | FastType::F32 | FastType::F64
            )
        })
        && result_types.iter().all(|t| {
            matches!(
                t,
                FastType::I32 | FastType::I64 | FastType::F32 | FastType::F64
            )
        })
        && ops.iter().all(|op| {
            matches!(
                op,
                RegOp::Const32 { .. }
                    | RegOp::Const64 { .. }
                    | RegOp::Mov { .. }
                    | RegOp::I32Add { .. }
                    | RegOp::I32Sub { .. }
                    | RegOp::I32Mul { .. }
                    | RegOp::I32And { .. }
                    | RegOp::I32Or { .. }
                    | RegOp::I32Xor { .. }
                    | RegOp::I32Shl { .. }
                    | RegOp::I32ShrS { .. }
                    | RegOp::I32ShrU { .. }
                    | RegOp::I32Rotl { .. }
                    | RegOp::I32Rotr { .. }
                    | RegOp::I32AddImm { .. }
                    | RegOp::I32SubImm { .. }
                    | RegOp::I32MulImm { .. }
                    | RegOp::I32AndImm { .. }
                    | RegOp::I32OrImm { .. }
                    | RegOp::I32XorImm { .. }
                    | RegOp::I32ShlImm { .. }
                    | RegOp::I32ShrSImm { .. }
                    | RegOp::I32ShrUImm { .. }
                    | RegOp::I32RotlImm { .. }
                    | RegOp::I32RotrImm { .. }
                    | RegOp::I32Eqz { .. }
                    | RegOp::I32Cmp { .. }
                    | RegOp::I32CmpImm { .. }
                    | RegOp::I64Add { .. }
                    | RegOp::I64Sub { .. }
                    | RegOp::I64Mul { .. }
                    | RegOp::I64And { .. }
                    | RegOp::I64Or { .. }
                    | RegOp::I64Xor { .. }
                    | RegOp::I64Shl { .. }
                    | RegOp::I64ShrS { .. }
                    | RegOp::I64ShrU { .. }
                    | RegOp::I64AddImm { .. }
                    | RegOp::I64Cmp { .. }
                    | RegOp::I32WrapI64 { .. }
                    | RegOp::I64ExtendI32S { .. }
                    | RegOp::F32Add { .. }
                    | RegOp::F32Sub { .. }
                    | RegOp::F32Mul { .. }
                    | RegOp::F32Div { .. }
                    | RegOp::F64Add { .. }
                    | RegOp::F64Sub { .. }
                    | RegOp::F64Mul { .. }
                    | RegOp::F64Div { .. }
                    | RegOp::F64Min { .. }
                    | RegOp::F64Max { .. }
                    | RegOp::F64Cmp { .. }
                    | RegOp::F64AddConst { .. }
                    | RegOp::F64SubConst { .. }
                    | RegOp::F64MulConst { .. }
                    | RegOp::F64MinConst { .. }
                    | RegOp::F64MaxConst { .. }
                    | RegOp::F32MulAdd { .. }
                    | RegOp::F64MulAdd { .. }
                    | RegOp::Select { .. }
                    | RegOp::Return { .. }
            )
        });

    Some(Arc::new(FastFunction {
        ops: ops.into_boxed_slice(),
        consts: consts.into_boxed_slice(),
        v128_consts: v128_consts.into_boxed_slice(),
        simd_masks: simd_masks.into_boxed_slice(),
        ref_heaps: ref_heaps.into_boxed_slice(),
        local_ref_heaps: local_ref_heaps.into_boxed_slice(),
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
        num_params: num_params as u16,
        local_reg_slots,
        local_ref_slots,
        slots: max_slots as u16,
        ref_slots: max_ref_slots as u16,
        stack_peak: bounds.stack_peak,
        max_nesting: bounds.maximum_nesting,
        mem0_is_64,
        fuel_blocks,
        pure_leaf,
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
fn read_v128_words(regs: &[u64], slot: u16) -> [u64; 2] {
    let s = slot as usize;
    let pair = &regs[s..s + 2];
    [pair[0], pair[1]]
}

#[inline(always)]
fn write_v128_words(regs: &mut [u64], slot: u16, words: [u64; 2]) {
    let s = slot as usize;
    let pair = &mut regs[s..s + 2];
    pair[0] = words[0];
    pair[1] = words[1];
}

#[inline(always)]
fn read_u16x8(regs: &[u64], slot: u16) -> [u16; 8] {
    let [lo, hi] = read_v128_words(regs, slot);
    [
        lo as u16,
        (lo >> 16) as u16,
        (lo >> 32) as u16,
        (lo >> 48) as u16,
        hi as u16,
        (hi >> 16) as u16,
        (hi >> 32) as u16,
        (hi >> 48) as u16,
    ]
}

#[inline(always)]
fn write_u16x8(regs: &mut [u64], slot: u16, v: [u16; 8]) {
    write_v128_words(
        regs,
        slot,
        [
            u64::from(v[0])
                | (u64::from(v[1]) << 16)
                | (u64::from(v[2]) << 32)
                | (u64::from(v[3]) << 48),
            u64::from(v[4])
                | (u64::from(v[5]) << 16)
                | (u64::from(v[6]) << 32)
                | (u64::from(v[7]) << 48),
        ],
    );
}

#[inline(always)]
fn swar_add_u8x8(x: u64, y: u64) -> u64 {
    const LO7: u64 = 0x7f7f_7f7f_7f7f_7f7f;
    const HI1: u64 = 0x8080_8080_8080_8080;
    (x & LO7).wrapping_add(y & LO7) ^ ((x ^ y) & HI1)
}

#[inline(always)]
fn swar_sub_u8x8(x: u64, y: u64) -> u64 {
    const LO7: u64 = 0x7f7f_7f7f_7f7f_7f7f;
    const HI1: u64 = 0x8080_8080_8080_8080;
    (x | HI1).wrapping_sub(y & LO7) ^ (!(x ^ y) & HI1)
}

#[inline(always)]
fn swar_add_u16x4(x: u64, y: u64) -> u64 {
    const LO15: u64 = 0x7fff_7fff_7fff_7fff;
    const HI1: u64 = 0x8000_8000_8000_8000;
    (x & LO15).wrapping_add(y & LO15) ^ ((x ^ y) & HI1)
}

#[inline(always)]
fn swar_sub_u16x4(x: u64, y: u64) -> u64 {
    const LO15: u64 = 0x7fff_7fff_7fff_7fff;
    const HI1: u64 = 0x8000_8000_8000_8000;
    (x | HI1).wrapping_sub(y & LO15) ^ (!(x ^ y) & HI1)
}

#[inline(always)]
fn swar_bitmask_u8x8(w: u64) -> u64 {
    (((w & 0x8080_8080_8080_8080) >> 7).wrapping_mul(0x0102_0408_1020_4080)) >> 56
}

#[inline(always)]
fn read_u32x4(regs: &[u64], slot: u16) -> [u32; 4] {
    let [lo, hi] = read_v128_words(regs, slot);
    [lo as u32, (lo >> 32) as u32, hi as u32, (hi >> 32) as u32]
}

#[inline(always)]
fn write_u32x4(regs: &mut [u64], slot: u16, v: [u32; 4]) {
    write_v128_words(
        regs,
        slot,
        [
            u64::from(v[0]) | (u64::from(v[1]) << 32),
            u64::from(v[2]) | (u64::from(v[3]) << 32),
        ],
    );
}

#[inline(always)]
fn read_f32x4(regs: &[u64], slot: u16) -> [f32; 4] {
    let u = read_u32x4(regs, slot);
    [
        f32::from_bits(u[0]),
        f32::from_bits(u[1]),
        f32::from_bits(u[2]),
        f32::from_bits(u[3]),
    ]
}

#[inline(always)]
fn write_f32x4(regs: &mut [u64], slot: u16, v: [f32; 4]) {
    write_u32x4(
        regs,
        slot,
        [
            v[0].to_bits(),
            v[1].to_bits(),
            v[2].to_bits(),
            v[3].to_bits(),
        ],
    );
}

#[inline(always)]
fn read_v128(regs: &[u64], slot: u16) -> [u8; 16] {
    let [lo, hi] = read_v128_words(regs, slot);
    let mut out = [0u8; 16];
    out[0..8].copy_from_slice(&lo.to_le_bytes());
    out[8..16].copy_from_slice(&hi.to_le_bytes());
    out
}

#[inline(always)]
fn write_v128(regs: &mut [u64], slot: u16, bytes: [u8; 16]) {
    write_v128_words(
        regs,
        slot,
        [
            u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        ],
    );
}

#[inline(always)]
fn slots_to_value(
    ty: FastType,
    slots: &[u64],
    off: usize,
    refs: &[RefValue],
    ref_off: usize,
) -> Value {
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
        FastType::Ref => Value::Ref(refs[ref_off]),
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
pub(crate) struct ActiveFrame {
    inst_id: usize,
    body: usize,
    func_handle: crate::Func,
    /// Next instruction pointer to resume when callee returns.
    ip: usize,
    /// Offset in `reg_bank` where this frame's scalar slots begin.
    reg_base: usize,
    /// Offset in `ref_bank` where this frame's reference slots begin.
    ref_base: usize,
    /// Relative scalar slot in caller where callee results should be written.
    ret_dst_base: u16,
    /// Relative ref slot in caller where callee reference results should be written.
    ret_ref_dst_base: u16,
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
fn try_promote_mem0_dense(store: &mut Store, mem0: Option<usize>) {
    if let Some(i) = mem0
        && store.mems[i].ty.index_type == IndexType::I32
    {
        store.mems[i].bytes.try_promote_dense();
    }
}

#[inline(always)]
fn try_promote_mem0_dense_if_written(store: &mut Store, mem0: Option<usize>) {
    if let Some(i) = mem0
        && store.mems[i].ty.index_type == IndexType::I32
        && store.mems[i].bytes.has_written_storage()
    {
        store.mems[i].bytes.try_promote_dense();
    }
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
fn fast_read_m32<const N: usize, const DENSE: bool>(
    addr: u32,
    offset: u32,
    p0_base: usize,
    p0: &[u8],
    p1_base: usize,
    p1: &[u8],
) -> Option<[u8; N]> {
    #[cfg(target_pointer_width = "64")]
    if DENSE {
        let at = (addr as usize) + (offset as usize);
        return p0.get(at..at + N).map(|s| s.try_into().unwrap());
    }
    let at = (addr as usize).checked_add(offset as usize)?;
    fast_read_bytes::<N, DENSE>(at, p0_base, p0, p1_base, p1)
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn fast_read_mem0<const N: usize, const DENSE: bool>(
    raw: u64,
    offset: u32,
    mem0_64: bool,
    p0_base: usize,
    p0: &[u8],
    p1_base: usize,
    p1: &[u8],
) -> Option<[u8; N]> {
    if DENSE {
        // Invariant: `dispatch_inner::<FUEL, true>` is only entered for unshared memory32 (`!mem0_64`).
        return fast_read_m32::<N, true>(raw as u32, offset, p0_base, p0, p1_base, p1);
    }
    let at = if !mem0_64 {
        (raw as u32 as usize).checked_add(offset as usize)?
    } else {
        raw.checked_add(offset as u64)
            .and_then(|e| usize::try_from(e).ok())?
    };
    fast_read_bytes::<N, false>(at, p0_base, p0, p1_base, p1)
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
fn fast_read_u8_m32<const DENSE: bool>(
    addr: u32,
    offset: u32,
    p0_base: usize,
    p0: &[u8],
    p1_base: usize,
    p1: &[u8],
) -> Option<u8> {
    #[cfg(target_pointer_width = "64")]
    if DENSE {
        let at = (addr as usize) + (offset as usize);
        return p0.get(at).copied();
    }
    let at = (addr as usize).checked_add(offset as usize)?;
    fast_read_u8::<DENSE>(at, p0_base, p0, p1_base, p1)
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn fast_read_u8_mem0<const DENSE: bool>(
    raw: u64,
    offset: u32,
    mem0_64: bool,
    p0_base: usize,
    p0: &[u8],
    p1_base: usize,
    p1: &[u8],
) -> Option<u8> {
    if DENSE {
        return fast_read_u8_m32::<true>(raw as u32, offset, p0_base, p0, p1_base, p1);
    }
    let at = if !mem0_64 {
        (raw as u32 as usize).checked_add(offset as usize)?
    } else {
        raw.checked_add(offset as u64)
            .and_then(|e| usize::try_from(e).ok())?
    };
    fast_read_u8::<false>(at, p0_base, p0, p1_base, p1)
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
#[allow(clippy::too_many_arguments)]
fn fast_write_m32<const N: usize, const DENSE: bool>(
    addr: u32,
    offset: u32,
    data: [u8; N],
    p0_base: usize,
    p0: &mut [u8],
    p1_base: usize,
    p1: &mut [u8],
) -> bool {
    #[cfg(target_pointer_width = "64")]
    if DENSE {
        let at = (addr as usize) + (offset as usize);
        if let Some(sub) = p0.get_mut(at..at + N) {
            sub.copy_from_slice(&data);
            return true;
        }
        return false;
    }
    let Some(at) = (addr as usize).checked_add(offset as usize) else {
        return false;
    };
    fast_write_bytes::<N, DENSE>(at, data, p0_base, p0, p1_base, p1)
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn fast_write_mem0<const N: usize, const DENSE: bool>(
    raw: u64,
    offset: u32,
    data: [u8; N],
    mem0_64: bool,
    p0_base: usize,
    p0: &mut [u8],
    p1_base: usize,
    p1: &mut [u8],
) -> bool {
    if DENSE {
        return fast_write_m32::<N, true>(raw as u32, offset, data, p0_base, p0, p1_base, p1);
    }
    let Some(at) = (if !mem0_64 {
        (raw as u32 as usize).checked_add(offset as usize)
    } else {
        raw.checked_add(offset as u64)
            .and_then(|e| usize::try_from(e).ok())
    }) else {
        return false;
    };
    fast_write_bytes::<N, false>(at, data, p0_base, p0, p1_base, p1)
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

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn fast_write_u8_m32<const DENSE: bool>(
    addr: u32,
    offset: u32,
    val: u8,
    p0_base: usize,
    p0: &mut [u8],
    p1_base: usize,
    p1: &mut [u8],
) -> bool {
    #[cfg(target_pointer_width = "64")]
    if DENSE {
        let at = (addr as usize) + (offset as usize);
        if let Some(slot) = p0.get_mut(at) {
            *slot = val;
            return true;
        }
        return false;
    }
    let Some(at) = (addr as usize).checked_add(offset as usize) else {
        return false;
    };
    fast_write_u8::<DENSE>(at, val, p0_base, p0, p1_base, p1)
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn fast_write_u8_mem0<const DENSE: bool>(
    raw: u64,
    offset: u32,
    val: u8,
    mem0_64: bool,
    p0_base: usize,
    p0: &mut [u8],
    p1_base: usize,
    p1: &mut [u8],
) -> bool {
    if DENSE {
        return fast_write_u8_m32::<true>(raw as u32, offset, val, p0_base, p0, p1_base, p1);
    }
    let Some(at) = (if !mem0_64 {
        (raw as u32 as usize).checked_add(offset as usize)
    } else {
        raw.checked_add(offset as u64)
            .and_then(|e| usize::try_from(e).ok())
    }) else {
        return false;
    };
    fast_write_u8::<false>(at, val, p0_base, p0, p1_base, p1)
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
    cur_ref_base: usize,
    cur_meta_idx: u32,
    reg_bank: &[u64],
    ref_bank: &[RefValue],
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

    let push_operands = |stack: &mut Vec<Value>,
                         func: &FastFunction,
                         meta: &BlockMeta,
                         slots: &[u64],
                         refs: &[RefValue]| {
        let s_start = meta.stack_start as usize;
        let s_end = s_start + meta.stack_len as usize;
        let mut off = func.local_reg_slots as usize;
        let mut roff = func.local_ref_slots as usize;
        for &ty in &func.stack_types[s_start..s_end] {
            stack.push(slots_to_value(ty, slots, off, refs, roff));
            off += ty.slots() as usize;
            roff += ty.ref_slots() as usize;
        }
    };

    let push_locals =
        |dst: &mut Vec<Value>, func: &FastFunction, slots: &[u64], refs: &[RefValue]| {
            let mut off = 0usize;
            let mut roff = 0usize;
            for &ty in &func.local_types {
                dst.push(slots_to_value(ty, slots, off, refs, roff));
                off += ty.slots() as usize;
                roff += ty.ref_slots() as usize;
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
        let refs = &ref_bank[cur_ref_base..cur_ref_base + cur_func.ref_slots as usize];
        entry_frame.function = store.funcs[cur_handle.0.index].clone();
        entry_frame.locals.clear();
        if entry_frame
            .locals
            .try_reserve(cur_func.local_types.len())
            .is_err()
        {
            return Err(err(ErrorKind::ResourceLimit, "frame allocation failed"));
        }
        push_locals(&mut entry_frame.locals, &cur_func, slots, refs);
        entry_frame.pc = meta.source_pc as usize;
        entry_frame.register = None;
        fill_labels(store, &mut entry_frame.labels, &cur_func, meta, entry_base)?;
        push_operands(stack, &cur_func, meta, slots, refs);
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
        let refs = &ref_bank[f0.ref_base..f0.ref_base + f0_func.ref_slots as usize];
        entry_frame.function = store.funcs[f0.func_handle.0.index].clone();
        entry_frame.locals.clear();
        if entry_frame
            .locals
            .try_reserve(f0_func.local_types.len())
            .is_err()
        {
            return Err(err(ErrorKind::ResourceLimit, "frame allocation failed"));
        }
        push_locals(&mut entry_frame.locals, &f0_func, slots, refs);
        entry_frame.pc = meta.source_pc as usize;
        entry_frame.register = None;
        fill_labels(store, &mut entry_frame.labels, &f0_func, meta, entry_base)?;
        push_operands(stack, &f0_func, meta, slots, refs);
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
        let refs = &ref_bank[af.ref_base..af.ref_base + af_func.ref_slots as usize];
        let base = stack.len();
        let mut locals = Vec::with_capacity(af_func.num_locals as usize);
        push_locals(&mut locals, &af_func, slots, refs);
        let mut labels = Vec::new();
        fill_labels(store, &mut labels, &af_func, meta, base)?;
        push_operands(stack, &af_func, meta, slots, refs);
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
        let refs = &ref_bank[cur_ref_base..cur_ref_base + cur_func.ref_slots as usize];
        let base = stack.len();
        let mut locals = Vec::with_capacity(cur_func.num_locals as usize);
        push_locals(&mut locals, &cur_func, slots, refs);
        let mut labels = Vec::new();
        fill_labels(store, &mut labels, &cur_func, meta, base)?;
        push_operands(stack, &cur_func, meta, slots, refs);
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

/// Zero-frame fast path for invoking straight-line, trap-free scalar leaf functions
/// directly from `Store::invoke` when fuel is disabled and limits permit execution.
/// Returns `None` on any argument mismatch, tight resource limit, or non-leaf body so
/// the canonical path handles errors and general execution identically.
#[inline]
pub(crate) fn try_invoke_leaf(
    store: &Store,
    func_idx: usize,
    args: &[Value],
) -> Option<crate::RunOutcome> {
    let fdata = &store.funcs[func_idx];
    let func = get_or_compile_ref(&store.instances[fdata.instance].module, fdata.body)?;
    if !func.pure_leaf || args.len() != func.num_params as usize {
        return None;
    }
    let lim = &store.engine.config.limits;
    if lim.max_call_depth == 0
        || func.stack_peak > lim.max_stack_height
        || store
            .active_local_slots
            .checked_add(func.num_locals as usize)
            .is_none_or(|n| n > store.engine.max_local_slots_limit)
        || store
            .active_labels
            .checked_add(func.max_nesting)
            .is_none_or(|n| n > store.engine.max_labels_limit)
    {
        return None;
    }
    let mut regs = [0u64; 16];
    for (i, (&arg, &ty)) in args.iter().zip(func.local_types.iter()).enumerate() {
        regs[i] = match (ty, arg) {
            (FastType::I32, Value::I32(n)) => n as u32 as u64,
            (FastType::I64, Value::I64(n)) => n as u64,
            (FastType::F32, Value::F32(n)) => n as u64,
            (FastType::F64, Value::F64(n)) => n,
            _ => return None,
        };
    }
    let consts = &func.consts;
    for &op in &func.ops {
        match op {
            RegOp::Const32 { dst, val } => regs[dst as usize] = val as u64,
            RegOp::Const64 { dst, idx } => regs[dst as usize] = consts[idx as usize],
            RegOp::Mov { dst, src } => regs[dst as usize] = regs[src as usize],
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
            RegOp::I32And { dst, a, b } => {
                regs[dst as usize] = ((regs[a as usize] as u32) & (regs[b as usize] as u32)) as u64;
            }
            RegOp::I32Or { dst, a, b } => {
                regs[dst as usize] = ((regs[a as usize] as u32) | (regs[b as usize] as u32)) as u64;
            }
            RegOp::I32Xor { dst, a, b } => {
                regs[dst as usize] = ((regs[a as usize] as u32) ^ (regs[b as usize] as u32)) as u64;
            }
            RegOp::I32Shl { dst, a, b } => {
                regs[dst as usize] =
                    (regs[a as usize] as u32).wrapping_shl(regs[b as usize] as u32) as u64;
            }
            RegOp::I32ShrS { dst, a, b } => {
                regs[dst as usize] =
                    ((regs[a as usize] as i32).wrapping_shr(regs[b as usize] as u32)) as u32 as u64;
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
            RegOp::I32AddImm { dst, src, imm } => {
                regs[dst as usize] = (regs[src as usize] as u32).wrapping_add(imm as u32) as u64;
            }
            RegOp::I32SubImm { dst, src, imm } => {
                regs[dst as usize] = (regs[src as usize] as u32).wrapping_sub(imm as u32) as u64;
            }
            RegOp::I32MulImm { dst, src, imm } => {
                regs[dst as usize] = (regs[src as usize] as u32).wrapping_mul(imm as u32) as u64;
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
            RegOp::I32Eqz { dst, src } => {
                regs[dst as usize] = u64::from((regs[src as usize] as u32) == 0);
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
            RegOp::I64Add { dst, a, b } => {
                regs[dst as usize] = regs[a as usize].wrapping_add(regs[b as usize]);
            }
            RegOp::I64Sub { dst, a, b } => {
                regs[dst as usize] = regs[a as usize].wrapping_sub(regs[b as usize]);
            }
            RegOp::I64Mul { dst, a, b } => {
                regs[dst as usize] = regs[a as usize].wrapping_mul(regs[b as usize]);
            }
            RegOp::I64And { dst, a, b } => regs[dst as usize] = regs[a as usize] & regs[b as usize],
            RegOp::I64Or { dst, a, b } => regs[dst as usize] = regs[a as usize] | regs[b as usize],
            RegOp::I64Xor { dst, a, b } => regs[dst as usize] = regs[a as usize] ^ regs[b as usize],
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
            RegOp::I32WrapI64 { dst, src } => regs[dst as usize] = regs[src as usize] as u32 as u64,
            RegOp::I64ExtendI32S { dst, src } => {
                regs[dst as usize] = regs[src as usize] as u32 as i32 as i64 as u64;
            }
            RegOp::F32Add { dst, a, b } => {
                regs[dst as usize] = (f32::from_bits(regs[a as usize] as u32)
                    + f32::from_bits(regs[b as usize] as u32))
                .to_bits() as u64;
            }
            RegOp::F32Sub { dst, a, b } => {
                regs[dst as usize] = (f32::from_bits(regs[a as usize] as u32)
                    - f32::from_bits(regs[b as usize] as u32))
                .to_bits() as u64;
            }
            RegOp::F32Mul { dst, a, b } => {
                regs[dst as usize] = (f32::from_bits(regs[a as usize] as u32)
                    * f32::from_bits(regs[b as usize] as u32))
                .to_bits() as u64;
            }
            RegOp::F32Div { dst, a, b } => {
                regs[dst as usize] = (f32::from_bits(regs[a as usize] as u32)
                    / f32::from_bits(regs[b as usize] as u32))
                .to_bits() as u64;
            }
            RegOp::F64Add { dst, a, b } => {
                regs[dst as usize] =
                    (f64::from_bits(regs[a as usize]) + f64::from_bits(regs[b as usize])).to_bits();
            }
            RegOp::F64Sub { dst, a, b } => {
                regs[dst as usize] =
                    (f64::from_bits(regs[a as usize]) - f64::from_bits(regs[b as usize])).to_bits();
            }
            RegOp::F64Mul { dst, a, b } => {
                regs[dst as usize] =
                    (f64::from_bits(regs[a as usize]) * f64::from_bits(regs[b as usize])).to_bits();
            }
            RegOp::F64Div { dst, a, b } => {
                regs[dst as usize] =
                    (f64::from_bits(regs[a as usize]) / f64::from_bits(regs[b as usize])).to_bits();
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
                regs[dst as usize] = (f64::from_bits(regs[src as usize])
                    + f64::from_bits(consts[cidx as usize]))
                .to_bits();
            }
            RegOp::F64SubConst { dst, src, cidx } => {
                regs[dst as usize] = (f64::from_bits(regs[src as usize])
                    - f64::from_bits(consts[cidx as usize]))
                .to_bits();
            }
            RegOp::F64MulConst { dst, src, cidx } => {
                regs[dst as usize] = (f64::from_bits(regs[src as usize])
                    * f64::from_bits(consts[cidx as usize]))
                .to_bits();
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
            RegOp::F32MulAdd { dst, acc, a, b } => {
                let prod = f32::from_bits(regs[a as usize] as u32)
                    * f32::from_bits(regs[b as usize] as u32);
                regs[dst as usize] =
                    (f32::from_bits(regs[acc as usize] as u32) + prod).to_bits() as u64;
            }
            RegOp::F64MulAdd { dst, acc, a, b } => {
                let prod = f64::from_bits(regs[a as usize]) * f64::from_bits(regs[b as usize]);
                regs[dst as usize] = (f64::from_bits(regs[acc as usize]) + prod).to_bits();
            }
            RegOp::Select { dst, a, b, cond } => {
                regs[dst as usize] = if regs[cond as usize] as u32 != 0 {
                    regs[a as usize]
                } else {
                    regs[b as usize]
                };
            }
            RegOp::Return { base, .. } => {
                let mut out = Vec::with_capacity(func.result_types.len());
                for (i, &ty) in func.result_types.iter().enumerate() {
                    out.push(slots_to_value(ty, &regs, base as usize + i, &[], 0));
                }
                return Some(crate::RunOutcome::Returned(out));
            }
            _ => return None,
        }
    }
    None
}

/// Executes the frame at the top of `frames` on the lazy untyped-`u64` + parallel `RefValue` register tier.
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
        inst.funcs[inst.module.data.imported_funcs + entry_body]
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

    // Avoid compiling an uncached multi-instruction body when zero fuel remains
    // (op 0 was already charged by `execution.rs`, so a multi-op body will bail
    // immediately to `Decoded`).
    if *fuel == Some(0) {
        let entry_mod = &store.instances[entry_inst].module;
        let already_compiled = entry_mod
            .data
            .register_fast
            .get(entry_body)
            .and_then(|s| s.get())
            .is_some();
        if !already_compiled
            && entry_mod
                .data
                .inner
                .decoded()
                .code
                .get(entry_body)
                .is_none_or(|c| c.instructions.len() > 1)
        {
            return Ok(FastOutcome::Declined);
        }
    }

    let (
        entry_slots,
        entry_ref_slots,
        entry_stack_peak,
        entry_max_nesting,
        entry_fuel_blocks,
        first_op,
    ) = {
        let Some(entry_func) = get_or_compile_ref(&store.instances[entry_inst].module, entry_body)
        else {
            return Ok(FastOutcome::Declined);
        };
        (
            entry_func.slots as usize,
            entry_func.ref_slots as usize,
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

    // Take pooled register banks and call stack from `Store` (safe under re-entry:
    // nested invocations receive empty Vecs and restore their own buffers on return).
    let mut reg_bank = std::mem::take(&mut store.fast_reg_bank);
    let mut ref_bank = std::mem::take(&mut store.fast_ref_bank);
    let mut call_stack = std::mem::take(&mut store.fast_call_stack);
    reg_bank.clear();
    ref_bank.clear();
    call_stack.clear();

    let restore_pools = |store: &mut Store,
                         mut reg_bank: Vec<u64>,
                         mut ref_bank: Vec<RefValue>,
                         mut call_stack: Vec<ActiveFrame>| {
        reg_bank.clear();
        if reg_bank.capacity() > 8192 {
            reg_bank.shrink_to(1024);
        }
        // Always clear `ref_bank` so no stale GC/extern handles linger in the pool!
        ref_bank.clear();
        if ref_bank.capacity() > 2048 {
            ref_bank.shrink_to(256);
        }
        call_stack.clear();
        if call_stack.capacity() > 256 {
            call_stack.shrink_to(64);
        }
        if store.fast_reg_bank.capacity() < reg_bank.capacity() {
            store.fast_reg_bank = reg_bank;
        }
        if store.fast_ref_bank.capacity() < ref_bank.capacity() {
            store.fast_ref_bank = ref_bank;
        }
        if store.fast_call_stack.capacity() < call_stack.capacity() {
            store.fast_call_stack = call_stack;
        }
    };

    if entry_slots > reg_bank.capacity() && reg_bank.try_reserve(entry_slots).is_err() {
        restore_pools(store, reg_bank, ref_bank, call_stack);
        return Err(err(ErrorKind::ResourceLimit, "frame allocation failed"));
    }
    if entry_ref_slots > ref_bank.capacity() && ref_bank.try_reserve(entry_ref_slots).is_err() {
        restore_pools(store, reg_bank, ref_bank, call_stack);
        return Err(err(ErrorKind::ResourceLimit, "frame allocation failed"));
    }
    for &v in &entry_frame.locals {
        match v {
            Value::Ref(r) => ref_bank.push(r),
            Value::V128(vec) => {
                reg_bank.push(u64::from_le_bytes(vec.0[0..8].try_into().unwrap()));
                reg_bank.push(u64::from_le_bytes(vec.0[8..16].try_into().unwrap()));
            }
            other => {
                if let Some(bits) = value_to_bits(other) {
                    reg_bank.push(bits);
                } else {
                    restore_pools(store, reg_bank, ref_bank, call_stack);
                    return Ok(FastOutcome::Declined);
                }
            }
        }
    }
    reg_bank.resize(entry_slots, 0);
    if entry_ref_slots > ref_bank.len() {
        ref_bank.resize(entry_ref_slots, RefValue::Null(crate::HeapType::None));
    }

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
            entry_inst,
            entry_body,
            entry_handle,
            entry_base,
            start_ip,
            first_block_fuel,
            &mut reg_bank,
            &mut ref_bank,
            &mut call_stack,
            max_stack_height,
            max_call_depth,
        )
    } else {
        run_loop::<false>(
            store,
            frames,
            stack,
            &mut local_fuel,
            entry_inst,
            entry_body,
            entry_handle,
            entry_base,
            start_ip,
            first_block_fuel,
            &mut reg_bank,
            &mut ref_bank,
            &mut call_stack,
            max_stack_height,
            max_call_depth,
        )
    };
    restore_pools(store, reg_bank, ref_bank, call_stack);
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
    AddF32,
    AddF64,
}

#[derive(Clone, Copy)]
struct ExecCursors {
    inst: usize,
    body: usize,
    handle: crate::Func,
    reg_base: usize,
    ref_base: usize,
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
        arg_ref_base: u16,
        arg_ref_slots: u16,
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
        arg_ref_base: u16,
        arg_ref_slots: u16,
        src_off: u8,
    },
    SwitchInst,
    ReturnTop {
        base: u16,
        count: u16,
        ref_base: u16,
        ref_count: u16,
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

#[cold]
#[inline(never)]
fn exec_cold_scalar_alu(op: RegOp, regs: &mut [u64]) -> Result<()> {
    match op {
        RegOp::I32DivS { dst, a, b, .. } => {
            let va = regs[a as usize] as i32;
            let vb = regs[b as usize] as i32;
            if vb == 0 {
                return Err(trap("integer divide by zero"));
            }
            let Some(q) = va.checked_div(vb) else {
                return Err(trap("integer overflow"));
            };
            regs[dst as usize] = q as u32 as u64;
        }
        RegOp::I32DivU { dst, a, b, .. } => {
            let vb = regs[b as usize] as u32;
            if vb == 0 {
                return Err(trap("integer divide by zero"));
            }
            regs[dst as usize] = ((regs[a as usize] as u32) / vb) as u64;
        }
        RegOp::I32RemS { dst, a, b, .. } => {
            let vb = regs[b as usize] as i32;
            if vb == 0 {
                return Err(trap("integer divide by zero"));
            }
            regs[dst as usize] = (regs[a as usize] as i32).wrapping_rem(vb) as u32 as u64;
        }
        RegOp::I32RemU { dst, a, b, .. } => {
            let vb = regs[b as usize] as u32;
            if vb == 0 {
                return Err(trap("integer divide by zero"));
            }
            regs[dst as usize] = ((regs[a as usize] as u32) % vb) as u64;
        }
        RegOp::Unary {
            op: uop, dst, src, ..
        } => {
            let a = regs[src as usize];
            let v = if integer::supported(uop) {
                integer::unary(uop, a)?
            } else {
                float::unary(uop, a)?
            };
            regs[dst as usize] = v;
        }
        RegOp::Binary {
            op: bop, dst, a, b, ..
        } => {
            let va = regs[a as usize];
            let vb = regs[b as usize];
            let v = if integer::supported(bop) {
                integer::binary(bop, va, vb)?
            } else {
                float::binary(bop, va, vb)?
            };
            regs[dst as usize] = v;
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
        _ => unreachable!(),
    }
    Ok(())
}

#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn resolve_indirect_callee(
    site: CallIndirectSite,
    regs: &[u64],
    inst_tabs: &[crate::Table],
    tabs: &crate::arena::Arena<crate::Tab>,
    funcs: &crate::arena::Arena<crate::Function>,
    inst_canonical: &[crate::CanonicalTypeId],
    canonical_lock: &std::sync::Mutex<crate::canonical::CanonicalTypes>,
    max_type_work: usize,
    store_id: u64,
) -> Result<crate::Func> {
    let raw_ix = regs[site.index_reg as usize];
    let ix = if site.table_is_64 {
        raw_ix
    } else {
        raw_ix as u32 as u64
    };
    let tab_h = inst_tabs[site.table_idx as usize];
    let tab = &tabs[tab_h.0.index];
    let Some(&val) = usize::try_from(ix).ok().and_then(|i| tab.values.get(i)) else {
        return Err(trap("table out of bounds"));
    };
    let Value::Ref(RefValue::Func(callee_handle)) = val else {
        return Err(trap("null indirect call"));
    };
    let Some(callee_fn) = funcs
        .get_live(callee_handle.0.index, callee_handle.0.generation)
        .filter(|_| callee_handle.0.store == store_id)
    else {
        return Err(err(ErrorKind::Host, "foreign or stale handle"));
    };
    let expected_canonical = inst_canonical[site.type_idx as usize];
    let compatible = if callee_fn.canonical == expected_canonical {
        true
    } else {
        let guard = canonical_lock
            .lock()
            .map_err(|_| err(ErrorKind::Host, "canonical registry poisoned"))?;
        guard.is_subtype(
            callee_fn.canonical,
            expected_canonical,
            false,
            max_type_work,
        )?
    };
    if !compatible {
        return Err(trap("indirect call type mismatch"));
    }
    Ok(callee_handle)
}

#[cold]
#[inline(never)]
fn exec_cold_simd(
    op: RegOp,
    cur_func: &FastFunction,
    regs: &mut [u64],
    inst_globals: &[crate::Global],
    globals: &mut crate::arena::Arena<crate::Glob>,
    simd_scratch: &mut Vec<Value>,
) -> Result<Option<InnerExit>> {
    match op {
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
            return Ok(Some(InnerExit::SlowSimdMem {
                op: mop,
                mem: arg.memory,
                offset: arg.offset,
                raw,
                dst: val,
                in_vec,
                lane: arg.lane,
                src_off,
            }));
        }
        RegOp::SimdUnary {
            op: sop,
            dst,
            src,
            lane,
        } => match sop {
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
        } => simd_binary_slow(sop, lane, regs, dst, a, b, simd_scratch)?,
        RegOp::SimdTernary {
            op: sop,
            dst,
            a,
            b,
            c,
        } => simd_ternary_slow(sop, regs, dst, a, b, c, simd_scratch)?,
        _ => unreachable!(),
    }
    Ok(None)
}

#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn exec_cold_mem_bulk<const DENSE: bool>(
    op: RegOp,
    cur_func: &FastFunction,
    regs: &[u64],
    p0_base: usize,
    p0_slice: &mut [u8],
    p1_base: usize,
    p1_slice: &mut [u8],
    #[cfg(test)] has_bulk_cancel_hook: bool,
) -> Option<InnerExit> {
    match op {
        RegOp::MemGen {
            op: mop,
            addr,
            val,
            arg_idx,
            src_off,
        } => {
            let arg = cur_func.mem_args[arg_idx as usize];
            let raw = regs[addr as usize];
            if mop >= 0x36 {
                Some(InnerExit::SlowStore {
                    op: mop,
                    mem: arg.memory,
                    offset: arg.offset,
                    raw,
                    val: regs[val as usize],
                    src_off,
                })
            } else {
                Some(InnerExit::SlowLoad {
                    op: mop,
                    mem: arg.memory,
                    offset: arg.offset,
                    raw,
                    dst: val,
                    kind: SlowLoadKind::Assign,
                    src_off,
                })
            }
        }
        RegOp::MemSize { dst, mem } => Some(InnerExit::MemSize { dst, mem }),
        RegOp::MemGrow {
            dst,
            delta,
            mem,
            src_off: _,
        } => Some(InnerExit::MemGrow {
            dst,
            delta: regs[delta as usize],
            mem,
        }),
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
                    if !p0_slice.is_empty()
                        && d0.checked_add(n).is_some_and(|e| e <= p0_slice.len())
                        && s0.checked_add(n).is_some_and(|e| e <= p0_slice.len())
                    {
                        p0_slice.copy_within(s0..s0 + n, d0);
                        true
                    } else {
                        let d1 = d.wrapping_sub(p1_base);
                        let s1 = s.wrapping_sub(p1_base);
                        if !p1_slice.is_empty()
                            && d1.checked_add(n).is_some_and(|e| e <= p1_slice.len())
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
            if fast_done {
                None
            } else {
                Some(InnerExit::MemCopy {
                    dst_mem: 0,
                    src_mem: 0,
                    dst: d_raw,
                    src: s_raw,
                    len: n_raw,
                    src_off,
                })
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
                    if let Some(sub) = d.checked_add(n).and_then(|e| p0_slice.get_mut(d..e)) {
                        sub.fill(v_raw as u8);
                        true
                    } else {
                        false
                    }
                } else {
                    let d0 = d.wrapping_sub(p0_base);
                    if !p0_slice.is_empty()
                        && let Some(sub) = d0.checked_add(n).and_then(|e| p0_slice.get_mut(d0..e))
                    {
                        sub.fill(v_raw as u8);
                        true
                    } else {
                        let d1 = d.wrapping_sub(p1_base);
                        if !p1_slice.is_empty()
                            && let Some(sub) =
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
            if fast_done {
                None
            } else {
                Some(InnerExit::MemFill {
                    mem: 0,
                    dst: d_raw,
                    val: v_raw,
                    len: n_raw,
                    src_off,
                })
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
            Some(match kind {
                0 => InnerExit::MemCopy {
                    dst_mem: arg.dst_mem,
                    src_mem: arg.src_mem,
                    dst: regs[dst as usize],
                    src: regs[src_or_val as usize],
                    len: regs[len as usize],
                    src_off,
                },
                1 => InnerExit::MemFill {
                    mem: arg.dst_mem,
                    dst: regs[dst as usize],
                    val: regs[src_or_val as usize],
                    len: regs[len as usize],
                    src_off,
                },
                2 => InnerExit::MemInit {
                    mem: arg.dst_mem,
                    seg: arg.src_mem,
                    dst: regs[dst as usize],
                    src: regs[src_or_val as usize],
                    len: regs[len as usize],
                    src_off,
                },
                3 => InnerExit::DataDrop { seg: arg.dst_mem },
                4 => InnerExit::TableInit {
                    table: arg.dst_mem,
                    seg: arg.src_mem,
                    dst: regs[dst as usize],
                    src: regs[src_or_val as usize],
                    len: regs[len as usize],
                    src_off,
                },
                5 => InnerExit::ElemDrop { seg: arg.dst_mem },
                _ => InnerExit::TableCopy {
                    dst_table: arg.dst_mem,
                    src_table: arg.src_mem,
                    dst: regs[dst as usize],
                    src: regs[src_or_val as usize],
                    len: regs[len as usize],
                    src_off,
                },
            })
        }
        RegOp::TableSize { dst, table } => Some(InnerExit::TableSize { dst, table }),
        _ => unreachable!(),
    }
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
    ref_bank: &mut Vec<RefValue>,
    simd_scratch: &mut Vec<Value>,
    fuel: &mut Option<u64>,
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
    let mut cur_ref_base = cursors.ref_base;
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
        let tabs = &mut store.tabs;
        let globals = &mut store.globals;
        let cancel = &store.cancellation.0;
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
        let mem0_64 = cur_func.mem0_is_64;
        let mut regs: &mut [u64] = &mut reg_bank[cur_reg_base..];
        let mut refs: &mut [RefValue] = &mut ref_bank[cur_ref_base..];
        let mut remaining_fuel = if FUEL { fuel.unwrap_or(0) } else { 0 };

        macro_rules! inline_call_target {
            ($callee_handle:expr, $arg_base:expr, $arg_slots:expr, $arg_ref_base:expr, $arg_ref_slots:expr, $logical_arg_start:expr, $meta_at_call:expr, $meta_after_call:expr, $src_off:expr) => {{
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
                            let new_ref_base = cur_ref_base + cur_func.ref_slots as usize;
                            let needed_ref = new_ref_base + callee_fast.ref_slots as usize;
                            if needed_ref > ref_bank.len() {
                                if ref_bank.try_reserve(needed_ref - ref_bank.len()).is_err() {
                                    if FUEL {
                                        *fuel = Some(remaining_fuel);
                                    }
                                    refund_on_trap!($src_off);
                                    return Err(err(
                                        ErrorKind::ResourceLimit,
                                        "frame allocation failed",
                                    ));
                                }
                                ref_bank.resize(needed_ref, RefValue::Null(crate::HeapType::None));
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
                            let arg_ref_src = cur_ref_base + $arg_ref_base as usize;
                            let arg_ref_slots = $arg_ref_slots as usize;
                            if arg_ref_slots > 0 {
                                ref_bank.copy_within(
                                    arg_ref_src..arg_ref_src + arg_ref_slots,
                                    new_ref_base,
                                );
                            }
                            for (i, &heap) in callee_fast.local_ref_heaps.iter().enumerate() {
                                ref_bank[new_ref_base + arg_ref_slots + i] = RefValue::Null(heap);
                            }
                            active_local_slots += nloc;
                            call_stack.push(ActiveFrame {
                                inst_id: cur_inst,
                                body: cur_body,
                                func_handle: cur_handle,
                                ip,
                                reg_base: cur_reg_base,
                                ref_base: cur_ref_base,
                                ret_dst_base: $arg_base,
                                ret_ref_dst_base: $arg_ref_base,
                                logical_base: cur_logical_base,
                                meta_after_call: $meta_after_call,
                                mem0: cur_mem0,
                            });
                            cur_body = callee_body;
                            cur_handle = callee_handle;
                            cur_func = callee_fast;
                            cur_reg_base = new_reg_base;
                            cur_ref_base = new_ref_base;
                            cur_logical_base = next_logical_base;
                            ops = &cur_func.ops;
                            consts = &cur_func.consts;
                            regs = &mut reg_bank[cur_reg_base..];
                            refs = &mut ref_bank[cur_ref_base..];
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
                                arg_ref_base: $arg_ref_base,
                                arg_ref_slots: $arg_ref_slots,
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
            ($callee_handle:expr, $arg_base:expr, $arg_slots:expr, $arg_ref_base:expr, $arg_ref_slots:expr, $meta_at_call:expr, $src_off:expr) => {{
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
                            let needed_ref = cur_ref_base + callee_fast.ref_slots as usize;
                            if needed_ref > ref_bank.len() {
                                if ref_bank.try_reserve(needed_ref - ref_bank.len()).is_err() {
                                    if FUEL {
                                        *fuel = Some(remaining_fuel);
                                    }
                                    refund_on_trap!($src_off);
                                    return Err(err(
                                        ErrorKind::ResourceLimit,
                                        "frame allocation failed",
                                    ));
                                }
                                ref_bank.resize(needed_ref, RefValue::Null(crate::HeapType::None));
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
                            let arg_ref_src = cur_ref_base + $arg_ref_base as usize;
                            let arg_ref_slots = $arg_ref_slots as usize;
                            if arg_ref_slots > 0 && arg_ref_src != cur_ref_base {
                                ref_bank.copy_within(
                                    arg_ref_src..arg_ref_src + arg_ref_slots,
                                    cur_ref_base,
                                );
                            }
                            for (i, &heap) in callee_fast.local_ref_heaps.iter().enumerate() {
                                ref_bank[cur_ref_base + arg_ref_slots + i] = RefValue::Null(heap);
                            }
                            active_local_slots = next_active_locals;
                            cur_body = callee_body;
                            cur_handle = callee_handle;
                            cur_func = callee_fast;
                            ops = &cur_func.ops;
                            consts = &cur_func.consts;
                            regs = &mut reg_bank[cur_reg_base..];
                            refs = &mut ref_bank[cur_ref_base..];
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
                                arg_ref_base: $arg_ref_base,
                                arg_ref_slots: $arg_ref_slots,
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
                RegOp::I32DivS { src_off, .. }
                | RegOp::I32DivU { src_off, .. }
                | RegOp::I32RemS { src_off, .. }
                | RegOp::I32RemU { src_off, .. }
                | RegOp::Unary { src_off, .. }
                | RegOp::Binary { src_off, .. } => {
                    if let Err(e) = exec_cold_scalar_alu(op, regs) {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!(src_off);
                        return Err(e);
                    }
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
                RegOp::F32MulAdd { dst, acc, a, b } => {
                    let prod = f32::from_bits(regs[a as usize] as u32)
                        * f32::from_bits(regs[b as usize] as u32);
                    regs[dst as usize] =
                        (f32::from_bits(regs[acc as usize] as u32) + prod).to_bits() as u64;
                }
                RegOp::F64MulAdd { dst, acc, a, b } => {
                    let prod = f64::from_bits(regs[a as usize]) * f64::from_bits(regs[b as usize]);
                    regs[dst as usize] = (f64::from_bits(regs[acc as usize]) + prod).to_bits();
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

                // ---- Fast memory-0 loads & stores (hoisted dense slice / cached 2-page slices!) ----
                RegOp::LoadI32 {
                    dst,
                    addr,
                    offset,
                    src_off,
                } => {
                    let raw = regs[addr as usize];
                    if let Some(arr) = fast_read_mem0::<4, DENSE>(
                        raw, offset, mem0_64, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
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
                    if let Some(arr) = fast_read_mem0::<8, DENSE>(
                        raw, offset, mem0_64, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
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
                    if let Some(b) = fast_read_u8_mem0::<DENSE>(
                        raw, offset, mem0_64, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
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
                    if let Some(arr) = fast_read_mem0::<2, DENSE>(
                        raw, offset, mem0_64, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
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
                    if let Some(b) = fast_read_u8_mem0::<DENSE>(
                        raw, offset, mem0_64, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
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
                RegOp::LoadI32Add {
                    acc,
                    addr,
                    offset,
                    src_off,
                } => {
                    let raw = regs[addr as usize];
                    if let Some(arr) = fast_read_mem0::<4, DENSE>(
                        raw, offset, mem0_64, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
                        let v = u32::from_le_bytes(arr);
                        regs[acc as usize] = (regs[acc as usize] as u32).wrapping_add(v) as u64;
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x28,
                            mem: 0,
                            offset: offset as u64,
                            raw,
                            dst: acc,
                            kind: SlowLoadKind::AddU32,
                            src_off,
                        };
                    }
                }
                RegOp::LoadF32Add {
                    acc,
                    addr,
                    offset,
                    src_off,
                } => {
                    let raw = regs[addr as usize];
                    if let Some(arr) = fast_read_mem0::<4, DENSE>(
                        raw, offset, mem0_64, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
                        let v = f32::from_bits(u32::from_le_bytes(arr));
                        let cur = f32::from_bits(regs[acc as usize] as u32);
                        regs[acc as usize] = (cur + v).to_bits() as u64;
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x2a,
                            mem: 0,
                            offset: offset as u64,
                            raw,
                            dst: acc,
                            kind: SlowLoadKind::AddF32,
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
                    if let Some(arr) = fast_read_mem0::<8, DENSE>(
                        raw, offset, mem0_64, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
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
                RegOp::LoadI32Shl2AddAcc {
                    acc,
                    base,
                    idx,
                    offset,
                    src_off,
                } => {
                    let raw32 = (regs[base as usize] as u32)
                        .wrapping_add((regs[idx as usize] as u32).wrapping_shl(2));
                    if let Some(arr) = fast_read_m32::<4, DENSE>(
                        raw32, offset, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
                        let v = u32::from_le_bytes(arr);
                        regs[acc as usize] = (regs[acc as usize] as u32).wrapping_add(v) as u64;
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x28,
                            mem: 0,
                            offset: offset as u64,
                            raw: raw32 as u64,
                            dst: acc,
                            kind: SlowLoadKind::AddU32,
                            src_off,
                        };
                    }
                }
                RegOp::LoadF64Shl3AddAcc {
                    acc,
                    base,
                    idx,
                    offset,
                    src_off,
                } => {
                    let raw32 = (regs[base as usize] as u32)
                        .wrapping_add((regs[idx as usize] as u32).wrapping_shl(3));
                    if let Some(arr) = fast_read_m32::<8, DENSE>(
                        raw32, offset, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
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
                            raw: raw32 as u64,
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
                    let raw32 = (regs[a as usize] as u32).wrapping_add(regs[b as usize] as u32);
                    if let Some(arr) = fast_read_m32::<4, DENSE>(
                        raw32, offset, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
                        regs[dst as usize] = u32::from_le_bytes(arr) as u64;
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x28,
                            mem: 0,
                            offset: offset as u64,
                            raw: raw32 as u64,
                            dst,
                            kind: SlowLoadKind::Assign,
                            src_off,
                        };
                    }
                }
                RegOp::LoadI64AddAddr {
                    dst,
                    a,
                    b,
                    offset,
                    src_off,
                } => {
                    let raw32 = (regs[a as usize] as u32).wrapping_add(regs[b as usize] as u32);
                    if let Some(arr) = fast_read_m32::<8, DENSE>(
                        raw32, offset, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
                        regs[dst as usize] = u64::from_le_bytes(arr);
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x29,
                            mem: 0,
                            offset: offset as u64,
                            raw: raw32 as u64,
                            dst,
                            kind: SlowLoadKind::Assign,
                            src_off,
                        };
                    }
                }
                RegOp::LoadI32Shl2 {
                    dst,
                    idx,
                    offset,
                    src_off,
                } => {
                    let raw32 = (regs[idx as usize] as u32).wrapping_shl(2);
                    if let Some(arr) = fast_read_m32::<4, DENSE>(
                        raw32, offset, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
                        regs[dst as usize] = u32::from_le_bytes(arr) as u64;
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x28,
                            mem: 0,
                            offset: offset as u64,
                            raw: raw32 as u64,
                            dst,
                            kind: SlowLoadKind::Assign,
                            src_off,
                        };
                    }
                }
                RegOp::LoadI32Shl2Add {
                    dst,
                    base,
                    idx,
                    offset,
                    src_off,
                } => {
                    let raw32 = (regs[base as usize] as u32)
                        .wrapping_add((regs[idx as usize] as u32).wrapping_shl(2));
                    if let Some(arr) = fast_read_m32::<4, DENSE>(
                        raw32, offset, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
                        regs[dst as usize] = u32::from_le_bytes(arr) as u64;
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x28,
                            mem: 0,
                            offset: offset as u64,
                            raw: raw32 as u64,
                            dst,
                            kind: SlowLoadKind::Assign,
                            src_off,
                        };
                    }
                }
                RegOp::LoadI64Shl3Add {
                    dst,
                    base,
                    idx,
                    offset,
                    src_off,
                } => {
                    let raw32 = (regs[base as usize] as u32)
                        .wrapping_add((regs[idx as usize] as u32).wrapping_shl(3));
                    if let Some(arr) = fast_read_m32::<8, DENSE>(
                        raw32, offset, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
                        regs[dst as usize] = u64::from_le_bytes(arr);
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x29,
                            mem: 0,
                            offset: offset as u64,
                            raw: raw32 as u64,
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
                    let raw32 = (regs[a as usize] as u32).wrapping_add(regs[b as usize] as u32);
                    if let Some(byte) = fast_read_u8_m32::<DENSE>(
                        raw32, offset, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
                        regs[dst as usize] = byte as u64;
                    } else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break InnerExit::SlowLoad {
                            op: 0x2d,
                            mem: 0,
                            offset: offset as u64,
                            raw: raw32 as u64,
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
                    let raw32 = (regs[addr as usize] as u32).wrapping_add(imm);
                    if let Some(arr) =
                        fast_read_m32::<4, DENSE>(raw32, 0, p0_base, p0_slice, p1_base, p1_slice)
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
                            raw: raw32 as u64,
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
                    let raw32 = (regs[addr as usize] as u32).wrapping_add(imm);
                    if let Some(byte) =
                        fast_read_u8_m32::<DENSE>(raw32, 0, p0_base, p0_slice, p1_base, p1_slice)
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
                            raw: raw32 as u64,
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
                    let raw32 = (regs[addr as usize] as u32).wrapping_add(imm);
                    if let Some(byte) =
                        fast_read_u8_m32::<DENSE>(raw32, 0, p0_base, p0_slice, p1_base, p1_slice)
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
                            raw: raw32 as u64,
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
                    if fast_write_mem0::<4, DENSE>(
                        raw,
                        offset,
                        (v as u32).to_le_bytes(),
                        mem0_64,
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
                    if fast_write_mem0::<8, DENSE>(
                        raw,
                        offset,
                        v.to_le_bytes(),
                        mem0_64,
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
                    if fast_write_u8_mem0::<DENSE>(
                        raw, offset, v as u8, mem0_64, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
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
                    if fast_write_mem0::<2, DENSE>(
                        raw,
                        offset,
                        (v as u16).to_le_bytes(),
                        mem0_64,
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
                            op: 0x3b,
                            mem: 0,
                            offset: offset as u64,
                            raw,
                            val: v,
                            src_off,
                        };
                    }
                }
                RegOp::StoreI32Imm {
                    addr,
                    val,
                    offset,
                    src_off,
                } => {
                    let raw = regs[addr as usize];
                    if fast_write_mem0::<4, DENSE>(
                        raw,
                        u32::from(offset),
                        val.to_le_bytes(),
                        mem0_64,
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
                            offset: u64::from(offset),
                            raw,
                            val: u64::from(val),
                            src_off,
                        };
                    }
                }
                RegOp::StoreI32_8Imm {
                    addr,
                    val,
                    offset,
                    src_off,
                } => {
                    let raw = regs[addr as usize];
                    if fast_write_u8_mem0::<DENSE>(
                        raw, offset, val, mem0_64, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
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
                            val: u64::from(val),
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
                    let raw32 = (regs[addr as usize] as u32).wrapping_add(imm);
                    let v = regs[val as usize];
                    if fast_write_m32::<4, DENSE>(
                        raw32,
                        0,
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
                            raw: raw32 as u64,
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
                    let raw32 = (regs[addr as usize] as u32).wrapping_add(imm);
                    let v = regs[val as usize];
                    if fast_write_u8_m32::<DENSE>(
                        raw32, 0, v as u8, p0_base, p0_slice, p1_base, p1_slice,
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
                            raw: raw32 as u64,
                            val: v,
                            src_off,
                        };
                    }
                }
                RegOp::StoreI32Shl2 {
                    idx,
                    val,
                    offset,
                    src_off,
                } => {
                    let raw32 = (regs[idx as usize] as u32).wrapping_shl(2);
                    let v = regs[val as usize];
                    if fast_write_m32::<4, DENSE>(
                        raw32,
                        offset,
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
                            offset: offset as u64,
                            raw: raw32 as u64,
                            val: v,
                            src_off,
                        };
                    }
                }
                RegOp::StoreI32Shl2Add {
                    base,
                    idx,
                    val,
                    offset,
                    src_off,
                } => {
                    let raw32 = (regs[base as usize] as u32)
                        .wrapping_add((regs[idx as usize] as u32).wrapping_shl(2));
                    let v = regs[val as usize];
                    if fast_write_m32::<4, DENSE>(
                        raw32,
                        offset,
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
                            offset: offset as u64,
                            raw: raw32 as u64,
                            val: v,
                            src_off,
                        };
                    }
                }
                RegOp::StoreI64Shl3Add {
                    base,
                    idx,
                    val,
                    offset,
                    src_off,
                } => {
                    let raw32 = (regs[base as usize] as u32)
                        .wrapping_add((regs[idx as usize] as u32).wrapping_shl(3));
                    let v = regs[val as usize];
                    if fast_write_m32::<8, DENSE>(
                        raw32,
                        offset,
                        v.to_le_bytes(),
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
                            op: 0x37,
                            mem: 0,
                            offset: offset as u64,
                            raw: raw32 as u64,
                            val: v,
                            src_off,
                        };
                    }
                }
                RegOp::MemGen { .. }
                | RegOp::MemSize { .. }
                | RegOp::MemGrow { .. }
                | RegOp::MemCopy0 { .. }
                | RegOp::MemFill0 { .. }
                | RegOp::MemBulkGen { .. }
                | RegOp::TableSize { .. } => {
                    if let Some(cold_exit) = exec_cold_mem_bulk::<DENSE>(
                        op,
                        cur_func,
                        regs,
                        p0_base,
                        p0_slice,
                        p1_base,
                        p1_slice,
                        #[cfg(test)]
                        has_bulk_cancel_hook,
                    ) {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break cold_exit;
                    }
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
                RegOp::I32IncNeBrIf {
                    reg,
                    limit,
                    step,
                    target,
                } => {
                    let next = (regs[reg as usize] as u32).wrapping_add(step as i32 as u32);
                    regs[reg as usize] = next as u64;
                    if next != regs[limit as usize] as u32 {
                        ip = target as usize;
                    }
                }
                RegOp::I32IncBrIfNonZero { reg, step, target } => {
                    let next = (regs[reg as usize] as u32).wrapping_add(step as u32);
                    regs[reg as usize] = next as u64;
                    if next != 0 {
                        ip = target as usize;
                    }
                }
                RegOp::I32IncCmpImmBrIf {
                    cmp,
                    step,
                    reg,
                    imm,
                    target,
                } => {
                    let next = (regs[reg as usize] as u32).wrapping_add(i32::from(step) as u32);
                    regs[reg as usize] = next as u64;
                    let rhs = match cmp {
                        2 | 4 | 6 | 8 => i32::from(imm as i16) as u32,
                        _ => u32::from(imm),
                    };
                    if eval_i32_cmp(cmp, next, rhs) {
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
                        site.arg_ref_base,
                        site.arg_ref_slots,
                        site.logical_arg_start,
                        site.meta_at_call,
                        site.meta_after_call,
                        src_off
                    );
                }
                RegOp::CallIndirect { call_idx, src_off } => {
                    let site = cur_func.call_indirect_sites[call_idx as usize];
                    let callee_handle = match resolve_indirect_callee(
                        site,
                        regs,
                        inst_tabs,
                        tabs,
                        funcs,
                        inst_canonical,
                        canonical_lock,
                        max_type_work,
                        store_id,
                    ) {
                        Ok(h) => h,
                        Err(e) => {
                            if FUEL {
                                *fuel = Some(remaining_fuel);
                            }
                            refund_on_trap!(src_off);
                            return Err(e);
                        }
                    };
                    inline_call_target!(
                        callee_handle,
                        site.arg_base,
                        site.arg_slots,
                        site.arg_ref_base,
                        site.arg_ref_slots,
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
                        site.arg_ref_base,
                        site.arg_ref_slots,
                        site.meta_at_call,
                        src_off
                    );
                }
                RegOp::ReturnCallIndirect { call_idx, src_off } => {
                    let site = cur_func.call_indirect_sites[call_idx as usize];
                    let callee_handle = match resolve_indirect_callee(
                        site,
                        regs,
                        inst_tabs,
                        tabs,
                        funcs,
                        inst_canonical,
                        canonical_lock,
                        max_type_work,
                        store_id,
                    ) {
                        Ok(h) => h,
                        Err(e) => {
                            if FUEL {
                                *fuel = Some(remaining_fuel);
                            }
                            refund_on_trap!(src_off);
                            return Err(e);
                        }
                    };
                    inline_tail_call_target!(
                        callee_handle,
                        site.arg_base,
                        site.arg_slots,
                        site.arg_ref_base,
                        site.arg_ref_slots,
                        site.meta_at_call,
                        src_off
                    );
                }
                RegOp::WideArith { .. } => {
                    exec_cold_scalar_alu(op, regs)?;
                }
                // ---- Hot unboxed v128 SIMD directly on `[u64; 2]` pairs in `regs` ----
                RegOp::MovV128 { dst, src } => {
                    let words = read_v128_words(regs, src);
                    write_v128_words(regs, dst, words);
                }
                RegOp::ConstV128 { dst, idx } => {
                    write_v128_words(regs, dst, cur_func.v128_consts[idx as usize]);
                }
                RegOp::SelectV128 { dst, a, b, cond } => {
                    let s = if regs[cond as usize] as u32 != 0 {
                        a
                    } else {
                        b
                    };
                    let words = read_v128_words(regs, s);
                    write_v128_words(regs, dst, words);
                }
                RegOp::LoadV128 {
                    dst,
                    addr,
                    offset,
                    src_off,
                } => {
                    let raw = regs[addr as usize];
                    if let Some(arr) = fast_read_mem0::<16, DENSE>(
                        raw, offset, mem0_64, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
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
                    if fast_write_mem0::<16, DENSE>(
                        raw, offset, data, mem0_64, p0_base, p0_slice, p1_base, p1_slice,
                    ) {
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
                RegOp::SimdShuffle {
                    dst,
                    a,
                    b,
                    mask_idx,
                } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    let mut src = [0u8; 32];
                    src[0..8].copy_from_slice(&alo.to_le_bytes());
                    src[8..16].copy_from_slice(&ahi.to_le_bytes());
                    src[16..24].copy_from_slice(&blo.to_le_bytes());
                    src[24..32].copy_from_slice(&bhi.to_le_bytes());
                    let ix = &cur_func.simd_masks[mask_idx as usize];
                    let mut out = [0u8; 16];
                    for i in 0..16 {
                        out[i] = src[(ix[i] & 31) as usize];
                    }
                    write_v128(regs, dst, out);
                }
                RegOp::V128Splat8 { dst, src } => {
                    let w = u64::from_le_bytes([regs[src as usize] as u8; 8]);
                    write_v128_words(regs, dst, [w, w]);
                }
                RegOp::V128Splat16 { dst, src } => {
                    let v = u64::from(regs[src as usize] as u16);
                    let w = v | (v << 16) | (v << 32) | (v << 48);
                    write_v128_words(regs, dst, [w, w]);
                }
                RegOp::V128Splat32 { dst, src } => {
                    let x = regs[src as usize] as u32 as u64;
                    let w = x | (x << 32);
                    write_v128_words(regs, dst, [w, w]);
                }
                RegOp::V128Splat64 { dst, src } => {
                    let w = regs[src as usize];
                    write_v128_words(regs, dst, [w, w]);
                }
                RegOp::V128Not { dst, src } => {
                    let [lo, hi] = read_v128_words(regs, src);
                    write_v128_words(regs, dst, [!lo, !hi]);
                }
                RegOp::V128And { dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(regs, dst, [alo & blo, ahi & bhi]);
                }
                RegOp::V128AndNot { dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(regs, dst, [alo & !blo, ahi & !bhi]);
                }
                RegOp::V128Or { dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(regs, dst, [alo | blo, ahi | bhi]);
                }
                RegOp::V128Xor { dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(regs, dst, [alo ^ blo, ahi ^ bhi]);
                }
                RegOp::V128Bitselect { dst, a, b, c } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    let [clo, chi] = read_v128_words(regs, c);
                    write_v128_words(
                        regs,
                        dst,
                        [(alo & clo) | (blo & !clo), (ahi & chi) | (bhi & !chi)],
                    );
                }
                RegOp::V128AnyTrue { dst, src } => {
                    let [lo, hi] = read_v128_words(regs, src);
                    regs[dst as usize] = u64::from((lo | hi) != 0);
                }
                RegOp::I8x16Bitmask { dst, src } => {
                    let [lo, hi] = read_v128_words(regs, src);
                    regs[dst as usize] = swar_bitmask_u8x8(lo) | (swar_bitmask_u8x8(hi) << 8);
                }
                RegOp::I32x4Bitmask { dst, src } => {
                    let [lo, hi] = read_v128_words(regs, src);
                    regs[dst as usize] =
                        ((lo >> 31) & 1) | ((lo >> 62) & 2) | ((hi >> 29) & 4) | ((hi >> 60) & 8);
                }
                RegOp::SimdExtract32 { dst, src, lane } => {
                    let word = regs[src as usize + (lane as usize >> 1)];
                    regs[dst as usize] = u64::from((word >> ((lane & 1) * 32)) as u32);
                }
                RegOp::I8x16Add { dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(
                        regs,
                        dst,
                        [swar_add_u8x8(alo, blo), swar_add_u8x8(ahi, bhi)],
                    );
                }
                RegOp::I8x16Sub { dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(
                        regs,
                        dst,
                        [swar_sub_u8x8(alo, blo), swar_sub_u8x8(ahi, bhi)],
                    );
                }
                RegOp::I16x8Add { dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(
                        regs,
                        dst,
                        [swar_add_u16x4(alo, blo), swar_add_u16x4(ahi, bhi)],
                    );
                }
                RegOp::I16x8Sub { dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(
                        regs,
                        dst,
                        [swar_sub_u16x4(alo, blo), swar_sub_u16x4(ahi, bhi)],
                    );
                }
                RegOp::I16x8Mul { dst, a, b } => {
                    let va = read_u16x8(regs, a);
                    let vb = read_u16x8(regs, b);
                    write_u16x8(
                        regs,
                        dst,
                        [
                            va[0].wrapping_mul(vb[0]),
                            va[1].wrapping_mul(vb[1]),
                            va[2].wrapping_mul(vb[2]),
                            va[3].wrapping_mul(vb[3]),
                            va[4].wrapping_mul(vb[4]),
                            va[5].wrapping_mul(vb[5]),
                            va[6].wrapping_mul(vb[6]),
                            va[7].wrapping_mul(vb[7]),
                        ],
                    );
                }
                RegOp::I32x4Add { dst, a, b } => {
                    let va = read_u32x4(regs, a);
                    let vb = read_u32x4(regs, b);
                    write_u32x4(
                        regs,
                        dst,
                        [
                            va[0].wrapping_add(vb[0]),
                            va[1].wrapping_add(vb[1]),
                            va[2].wrapping_add(vb[2]),
                            va[3].wrapping_add(vb[3]),
                        ],
                    );
                }
                RegOp::I32x4Sub { dst, a, b } => {
                    let va = read_u32x4(regs, a);
                    let vb = read_u32x4(regs, b);
                    write_u32x4(
                        regs,
                        dst,
                        [
                            va[0].wrapping_sub(vb[0]),
                            va[1].wrapping_sub(vb[1]),
                            va[2].wrapping_sub(vb[2]),
                            va[3].wrapping_sub(vb[3]),
                        ],
                    );
                }
                RegOp::I32x4Mul { dst, a, b } => {
                    let va = read_u32x4(regs, a);
                    let vb = read_u32x4(regs, b);
                    write_u32x4(
                        regs,
                        dst,
                        [
                            va[0].wrapping_mul(vb[0]),
                            va[1].wrapping_mul(vb[1]),
                            va[2].wrapping_mul(vb[2]),
                            va[3].wrapping_mul(vb[3]),
                        ],
                    );
                }
                RegOp::I32x4Cmp { cmp, dst, a, b } => {
                    let va = read_u32x4(regs, a);
                    let vb = read_u32x4(regs, b);
                    let lane = |x: u32, y: u32| -> u32 {
                        if eval_i32_cmp(cmp, x, y) { u32::MAX } else { 0 }
                    };
                    write_u32x4(
                        regs,
                        dst,
                        [
                            lane(va[0], vb[0]),
                            lane(va[1], vb[1]),
                            lane(va[2], vb[2]),
                            lane(va[3], vb[3]),
                        ],
                    );
                }
                RegOp::I32x4Shift { kind, dst, a, sh } => {
                    let va = read_u32x4(regs, a);
                    let s = (regs[sh as usize] as u32) & 31;
                    let out = match kind {
                        0 => [
                            va[0].wrapping_shl(s),
                            va[1].wrapping_shl(s),
                            va[2].wrapping_shl(s),
                            va[3].wrapping_shl(s),
                        ],
                        1 => [
                            (va[0] as i32).wrapping_shr(s) as u32,
                            (va[1] as i32).wrapping_shr(s) as u32,
                            (va[2] as i32).wrapping_shr(s) as u32,
                            (va[3] as i32).wrapping_shr(s) as u32,
                        ],
                        _ => [
                            va[0].wrapping_shr(s),
                            va[1].wrapping_shr(s),
                            va[2].wrapping_shr(s),
                            va[3].wrapping_shr(s),
                        ],
                    };
                    write_u32x4(regs, dst, out);
                }
                RegOp::I64x2Add { dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(regs, dst, [alo.wrapping_add(blo), ahi.wrapping_add(bhi)]);
                }
                RegOp::I64x2Sub { dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(regs, dst, [alo.wrapping_sub(blo), ahi.wrapping_sub(bhi)]);
                }
                RegOp::I64x2Mul { dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(regs, dst, [alo.wrapping_mul(blo), ahi.wrapping_mul(bhi)]);
                }
                RegOp::F32x4Abs { dst, src } => {
                    let [lo, hi] = read_v128_words(regs, src);
                    write_v128_words(
                        regs,
                        dst,
                        [lo & 0x7fff_ffff_7fff_ffff, hi & 0x7fff_ffff_7fff_ffff],
                    );
                }
                RegOp::F32x4Neg { dst, src } => {
                    let [lo, hi] = read_v128_words(regs, src);
                    write_v128_words(
                        regs,
                        dst,
                        [lo ^ 0x8000_0000_8000_0000, hi ^ 0x8000_0000_8000_0000],
                    );
                }
                RegOp::F32x4Sqrt { dst, src } => {
                    let va = read_f32x4(regs, src);
                    write_f32x4(
                        regs,
                        dst,
                        [va[0].sqrt(), va[1].sqrt(), va[2].sqrt(), va[3].sqrt()],
                    );
                }
                RegOp::F32x4Add { dst, a, b } => {
                    let va = read_f32x4(regs, a);
                    let vb = read_f32x4(regs, b);
                    write_f32x4(
                        regs,
                        dst,
                        [va[0] + vb[0], va[1] + vb[1], va[2] + vb[2], va[3] + vb[3]],
                    );
                }
                RegOp::F32x4Sub { dst, a, b } => {
                    let va = read_f32x4(regs, a);
                    let vb = read_f32x4(regs, b);
                    write_f32x4(
                        regs,
                        dst,
                        [va[0] - vb[0], va[1] - vb[1], va[2] - vb[2], va[3] - vb[3]],
                    );
                }
                RegOp::F32x4Mul { dst, a, b } => {
                    let va = read_f32x4(regs, a);
                    let vb = read_f32x4(regs, b);
                    write_f32x4(
                        regs,
                        dst,
                        [va[0] * vb[0], va[1] * vb[1], va[2] * vb[2], va[3] * vb[3]],
                    );
                }
                RegOp::F32x4Div { dst, a, b } => {
                    let va = read_f32x4(regs, a);
                    let vb = read_f32x4(regs, b);
                    write_f32x4(
                        regs,
                        dst,
                        [va[0] / vb[0], va[1] / vb[1], va[2] / vb[2], va[3] / vb[3]],
                    );
                }
                RegOp::F32x4Min { dst, a, b } => {
                    let va = read_f32x4(regs, a);
                    let vb = read_f32x4(regs, b);
                    write_u32x4(
                        regs,
                        dst,
                        [
                            wasm_f32_min(va[0], vb[0]),
                            wasm_f32_min(va[1], vb[1]),
                            wasm_f32_min(va[2], vb[2]),
                            wasm_f32_min(va[3], vb[3]),
                        ],
                    );
                }
                RegOp::F32x4Max { dst, a, b } => {
                    let va = read_f32x4(regs, a);
                    let vb = read_f32x4(regs, b);
                    write_u32x4(
                        regs,
                        dst,
                        [
                            wasm_f32_max(va[0], vb[0]),
                            wasm_f32_max(va[1], vb[1]),
                            wasm_f32_max(va[2], vb[2]),
                            wasm_f32_max(va[3], vb[3]),
                        ],
                    );
                }
                RegOp::F32x4PMin { dst, a, b } => {
                    let ua = read_u32x4(regs, a);
                    let ub = read_u32x4(regs, b);
                    write_u32x4(
                        regs,
                        dst,
                        [
                            if f32::from_bits(ub[0]) < f32::from_bits(ua[0]) {
                                ub[0]
                            } else {
                                ua[0]
                            },
                            if f32::from_bits(ub[1]) < f32::from_bits(ua[1]) {
                                ub[1]
                            } else {
                                ua[1]
                            },
                            if f32::from_bits(ub[2]) < f32::from_bits(ua[2]) {
                                ub[2]
                            } else {
                                ua[2]
                            },
                            if f32::from_bits(ub[3]) < f32::from_bits(ua[3]) {
                                ub[3]
                            } else {
                                ua[3]
                            },
                        ],
                    );
                }
                RegOp::F32x4PMax { dst, a, b } => {
                    let ua = read_u32x4(regs, a);
                    let ub = read_u32x4(regs, b);
                    write_u32x4(
                        regs,
                        dst,
                        [
                            if f32::from_bits(ua[0]) < f32::from_bits(ub[0]) {
                                ub[0]
                            } else {
                                ua[0]
                            },
                            if f32::from_bits(ua[1]) < f32::from_bits(ub[1]) {
                                ub[1]
                            } else {
                                ua[1]
                            },
                            if f32::from_bits(ua[2]) < f32::from_bits(ub[2]) {
                                ub[2]
                            } else {
                                ua[2]
                            },
                            if f32::from_bits(ua[3]) < f32::from_bits(ub[3]) {
                                ub[3]
                            } else {
                                ua[3]
                            },
                        ],
                    );
                }
                RegOp::F32x4Cmp { cmp, dst, a, b } => {
                    let va = read_f32x4(regs, a);
                    let vb = read_f32x4(regs, b);
                    let lane = |x: f32, y: f32| -> u32 {
                        if eval_f32_cmp(cmp, x, y) { u32::MAX } else { 0 }
                    };
                    write_u32x4(
                        regs,
                        dst,
                        [
                            lane(va[0], vb[0]),
                            lane(va[1], vb[1]),
                            lane(va[2], vb[2]),
                            lane(va[3], vb[3]),
                        ],
                    );
                }
                RegOp::F32x4MulAdd { dst, acc, a, b } => {
                    let va = read_f32x4(regs, a);
                    let vb = read_f32x4(regs, b);
                    let vc = read_f32x4(regs, acc);
                    write_f32x4(
                        regs,
                        dst,
                        [
                            vc[0] + (va[0] * vb[0]),
                            vc[1] + (va[1] * vb[1]),
                            vc[2] + (va[2] * vb[2]),
                            vc[3] + (va[3] * vb[3]),
                        ],
                    );
                }
                RegOp::F32x4RelaxedMadd { dst, a, b, c } => {
                    let va = read_f32x4(regs, a);
                    let vb = read_f32x4(regs, b);
                    let vc = read_f32x4(regs, c);
                    write_f32x4(
                        regs,
                        dst,
                        [
                            va[0].mul_add(vb[0], vc[0]),
                            va[1].mul_add(vb[1], vc[1]),
                            va[2].mul_add(vb[2], vc[2]),
                            va[3].mul_add(vb[3], vc[3]),
                        ],
                    );
                }
                RegOp::F32x4RelaxedNmadd { dst, a, b, c } => {
                    let va = read_f32x4(regs, a);
                    let vb = read_f32x4(regs, b);
                    let vc = read_f32x4(regs, c);
                    write_f32x4(
                        regs,
                        dst,
                        [
                            (-va[0]).mul_add(vb[0], vc[0]),
                            (-va[1]).mul_add(vb[1], vc[1]),
                            (-va[2]).mul_add(vb[2], vc[2]),
                            (-va[3]).mul_add(vb[3], vc[3]),
                        ],
                    );
                }
                RegOp::F32x4ConvertI32x4S { dst, src } => {
                    let va = read_u32x4(regs, src);
                    write_f32x4(
                        regs,
                        dst,
                        [
                            (va[0] as i32) as f32,
                            (va[1] as i32) as f32,
                            (va[2] as i32) as f32,
                            (va[3] as i32) as f32,
                        ],
                    );
                }
                RegOp::F32x4ConvertI32x4U { dst, src } => {
                    let va = read_u32x4(regs, src);
                    write_f32x4(
                        regs,
                        dst,
                        [va[0] as f32, va[1] as f32, va[2] as f32, va[3] as f32],
                    );
                }
                RegOp::F64x2Abs { dst, src } => {
                    let [lo, hi] = read_v128_words(regs, src);
                    write_v128_words(
                        regs,
                        dst,
                        [lo & 0x7fff_ffff_ffff_ffff, hi & 0x7fff_ffff_ffff_ffff],
                    );
                }
                RegOp::F64x2Neg { dst, src } => {
                    let [lo, hi] = read_v128_words(regs, src);
                    write_v128_words(
                        regs,
                        dst,
                        [lo ^ 0x8000_0000_0000_0000, hi ^ 0x8000_0000_0000_0000],
                    );
                }
                RegOp::F64x2Sqrt { dst, src } => {
                    let [lo, hi] = read_v128_words(regs, src);
                    write_v128_words(
                        regs,
                        dst,
                        [
                            f64::from_bits(lo).sqrt().to_bits(),
                            f64::from_bits(hi).sqrt().to_bits(),
                        ],
                    );
                }
                RegOp::F64x2Add { dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(
                        regs,
                        dst,
                        [
                            (f64::from_bits(alo) + f64::from_bits(blo)).to_bits(),
                            (f64::from_bits(ahi) + f64::from_bits(bhi)).to_bits(),
                        ],
                    );
                }
                RegOp::F64x2Sub { dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(
                        regs,
                        dst,
                        [
                            (f64::from_bits(alo) - f64::from_bits(blo)).to_bits(),
                            (f64::from_bits(ahi) - f64::from_bits(bhi)).to_bits(),
                        ],
                    );
                }
                RegOp::F64x2Mul { dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(
                        regs,
                        dst,
                        [
                            (f64::from_bits(alo) * f64::from_bits(blo)).to_bits(),
                            (f64::from_bits(ahi) * f64::from_bits(bhi)).to_bits(),
                        ],
                    );
                }
                RegOp::F64x2Div { dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(
                        regs,
                        dst,
                        [
                            (f64::from_bits(alo) / f64::from_bits(blo)).to_bits(),
                            (f64::from_bits(ahi) / f64::from_bits(bhi)).to_bits(),
                        ],
                    );
                }
                RegOp::F64x2Min { dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(
                        regs,
                        dst,
                        [
                            wasm_f64_min(f64::from_bits(alo), f64::from_bits(blo)),
                            wasm_f64_min(f64::from_bits(ahi), f64::from_bits(bhi)),
                        ],
                    );
                }
                RegOp::F64x2Max { dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(
                        regs,
                        dst,
                        [
                            wasm_f64_max(f64::from_bits(alo), f64::from_bits(blo)),
                            wasm_f64_max(f64::from_bits(ahi), f64::from_bits(bhi)),
                        ],
                    );
                }
                RegOp::F64x2PMin { dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(
                        regs,
                        dst,
                        [
                            if f64::from_bits(blo) < f64::from_bits(alo) {
                                blo
                            } else {
                                alo
                            },
                            if f64::from_bits(bhi) < f64::from_bits(ahi) {
                                bhi
                            } else {
                                ahi
                            },
                        ],
                    );
                }
                RegOp::F64x2PMax { dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(
                        regs,
                        dst,
                        [
                            if f64::from_bits(alo) < f64::from_bits(blo) {
                                blo
                            } else {
                                alo
                            },
                            if f64::from_bits(ahi) < f64::from_bits(bhi) {
                                bhi
                            } else {
                                ahi
                            },
                        ],
                    );
                }
                RegOp::F64x2Cmp { cmp, dst, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    write_v128_words(
                        regs,
                        dst,
                        [
                            if eval_f64_cmp(cmp, f64::from_bits(alo), f64::from_bits(blo)) {
                                u64::MAX
                            } else {
                                0
                            },
                            if eval_f64_cmp(cmp, f64::from_bits(ahi), f64::from_bits(bhi)) {
                                u64::MAX
                            } else {
                                0
                            },
                        ],
                    );
                }
                RegOp::F64x2MulAdd { dst, acc, a, b } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    let [clo, chi] = read_v128_words(regs, acc);
                    write_v128_words(
                        regs,
                        dst,
                        [
                            (f64::from_bits(clo) + (f64::from_bits(alo) * f64::from_bits(blo)))
                                .to_bits(),
                            (f64::from_bits(chi) + (f64::from_bits(ahi) * f64::from_bits(bhi)))
                                .to_bits(),
                        ],
                    );
                }
                RegOp::F64x2RelaxedMadd { dst, a, b, c } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    let [clo, chi] = read_v128_words(regs, c);
                    write_v128_words(
                        regs,
                        dst,
                        [
                            f64::from_bits(alo)
                                .mul_add(f64::from_bits(blo), f64::from_bits(clo))
                                .to_bits(),
                            f64::from_bits(ahi)
                                .mul_add(f64::from_bits(bhi), f64::from_bits(chi))
                                .to_bits(),
                        ],
                    );
                }
                RegOp::F64x2RelaxedNmadd { dst, a, b, c } => {
                    let [alo, ahi] = read_v128_words(regs, a);
                    let [blo, bhi] = read_v128_words(regs, b);
                    let [clo, chi] = read_v128_words(regs, c);
                    write_v128_words(
                        regs,
                        dst,
                        [
                            (-f64::from_bits(alo))
                                .mul_add(f64::from_bits(blo), f64::from_bits(clo))
                                .to_bits(),
                            (-f64::from_bits(ahi))
                                .mul_add(f64::from_bits(bhi), f64::from_bits(chi))
                                .to_bits(),
                        ],
                    );
                }
                RegOp::GlobalGetV128 { .. }
                | RegOp::GlobalSetV128 { .. }
                | RegOp::SimdMemGen { .. }
                | RegOp::SimdUnary { .. }
                | RegOp::SimdBinary { .. }
                | RegOp::SimdTernary { .. } => {
                    if let Some(simd_exit) =
                        exec_cold_simd(op, cur_func, regs, inst_globals, globals, simd_scratch)?
                    {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        break simd_exit;
                    }
                }
                // ---- Reference operations (parallel `ref_bank`) ----
                RegOp::MovRef { dst, src } => {
                    refs[dst as usize] = refs[src as usize];
                }
                RegOp::MoveRefRange {
                    dst_base,
                    src_base,
                    count,
                } => {
                    let s = src_base as usize;
                    let d = dst_base as usize;
                    let c = count as usize;
                    refs.copy_within(s..s + c, d);
                }
                RegOp::RefNull { dst, heap_idx } => {
                    refs[dst as usize] = RefValue::Null(cur_func.ref_heaps[heap_idx as usize]);
                }
                RegOp::RefIsNull { dst, src } => {
                    regs[dst as usize] = u64::from(matches!(refs[src as usize], RefValue::Null(_)));
                }
                RegOp::RefFunc { dst, func_idx } => {
                    refs[dst as usize] = RefValue::Func(inst_funcs[func_idx as usize]);
                }
                RegOp::RefEq { dst, a, b } => {
                    let va = refs[a as usize];
                    let vb = refs[b as usize];
                    regs[dst as usize] = u64::from(
                        va == vb || matches!((va, vb), (RefValue::Null(_), RefValue::Null(_))),
                    );
                }
                RegOp::RefAsNonNull { dst, src, src_off } => {
                    let v = refs[src as usize];
                    if matches!(v, RefValue::Null(_)) {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!(src_off);
                        return Err(trap("null reference"));
                    }
                    refs[dst as usize] = v;
                }
                RegOp::SelectRef { dst, a, b, cond } => {
                    refs[dst as usize] = if regs[cond as usize] as u32 != 0 {
                        refs[a as usize]
                    } else {
                        refs[b as usize]
                    };
                }
                RegOp::GlobalGetRef { dst, global } => {
                    let gh = inst_globals[global as usize];
                    let Value::Ref(r) = globals[gh.0.index].value else {
                        return Err(trap("global type"));
                    };
                    refs[dst as usize] = r;
                }
                RegOp::GlobalSetRef { src, global } => {
                    let gh = inst_globals[global as usize];
                    let slot = &mut globals[gh.0.index];
                    slot.value = Value::Ref(refs[src as usize]);
                }
                RegOp::TableGet {
                    dst,
                    idx,
                    table,
                    src_off,
                } => {
                    let tab_h = inst_tabs[table as usize];
                    let tab = &tabs[tab_h.0.index];
                    let raw = regs[idx as usize];
                    let ix = if tab.ty.index_type == IndexType::I64 {
                        raw
                    } else {
                        raw as u32 as u64
                    };
                    let Some(&Value::Ref(r)) =
                        usize::try_from(ix).ok().and_then(|i| tab.values.get(i))
                    else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!(src_off);
                        return Err(trap("table out of bounds"));
                    };
                    refs[dst as usize] = r;
                }
                RegOp::TableSet {
                    idx,
                    val,
                    table,
                    src_off,
                } => {
                    let tab_h = inst_tabs[table as usize];
                    let tab = &mut tabs[tab_h.0.index];
                    let raw = regs[idx as usize];
                    let ix = if tab.ty.index_type == IndexType::I64 {
                        raw
                    } else {
                        raw as u32 as u64
                    };
                    let Some(slot) = usize::try_from(ix).ok().and_then(|i| tab.values.get_mut(i))
                    else {
                        if FUEL {
                            *fuel = Some(remaining_fuel);
                        }
                        refund_on_trap!(src_off);
                        return Err(trap("table out of bounds"));
                    };
                    *slot = Value::Ref(refs[val as usize]);
                }
                RegOp::Return {
                    base,
                    count,
                    ref_base,
                    ref_count,
                } => {
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
                        let rc = ref_count as usize;
                        if rc > 0 {
                            let rsrc = cur_ref_base + ref_base as usize;
                            let rdst = caller.ref_base + caller.ret_ref_dst_base as usize;
                            if rc == 1 {
                                ref_bank[rdst] = ref_bank[rsrc];
                            } else {
                                ref_bank.copy_within(rsrc..rsrc + rc, rdst);
                            }
                        }
                        active_local_slots = active_local_slots.saturating_sub(released);
                        cur_body = caller.body;
                        cur_handle = caller.func_handle;
                        cur_reg_base = caller.reg_base;
                        cur_ref_base = caller.ref_base;
                        cur_logical_base = caller.logical_base;
                        ip = caller.ip;
                        if caller.inst_id == cur_inst {
                            cur_func = inst_fast[cur_body]
                                .get()
                                .and_then(|o| o.as_deref())
                                .unwrap();
                            ops = &cur_func.ops;
                            consts = &cur_func.consts;
                            regs = &mut reg_bank[cur_reg_base..];
                            refs = &mut ref_bank[cur_ref_base..];
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
                    break InnerExit::ReturnTop {
                        base,
                        count,
                        ref_base,
                        ref_count,
                    };
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
    cursors.ref_base = cur_ref_base;
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
    mut cur_inst: usize,
    mut cur_body: usize,
    mut cur_handle: crate::Func,
    mut cur_logical_base: usize,
    mut ip: usize,
    mut cur_block_fuel: u16,
    reg_bank: &mut Vec<u64>,
    ref_bank: &mut Vec<RefValue>,
    call_stack: &mut Vec<ActiveFrame>,
    max_stack_height: usize,
    max_call_depth: usize,
) -> Result<FastOutcome> {
    let mut simd_scratch: Vec<Value> = Vec::new();
    let mut cur_reg_base: usize = 0;
    let mut cur_ref_base: usize = 0;
    let mut cur_mem0: Option<usize> = resolve_mem0(store, cur_inst);
    try_promote_mem0_dense_if_written(store, cur_mem0);
    let mut cache_p0_idx: usize = 0;
    let mut cache_p1_idx: usize = 1;
    let frames_len = frames.len();
    let store_id = store.id;
    let max_local_slots_limit = store.engine.max_local_slots_limit;
    let max_labels_limit = store.engine.max_labels_limit;

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
                call_stack,
                cur_inst,
                cur_body,
                cur_handle,
                cur_reg_base,
                cur_ref_base,
                $meta,
                reg_bank,
                ref_bank,
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
            ref_base: cur_ref_base,
            logical_base: cur_logical_base,
            mem0: cur_mem0,
            ip,
            block_fuel: cur_block_fuel,
            active_local_slots: store.active_local_slots,
        };
        let is_dense = cur_mem0.is_some_and(|i| {
            store.mems[i].ty.index_type == IndexType::I32 && store.mems[i].bytes.is_dense()
        });
        let exit = if is_dense {
            dispatch_inner::<FUEL, true>(
                store,
                &mut cursors,
                call_stack,
                reg_bank,
                ref_bank,
                &mut simd_scratch,
                fuel,
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
                call_stack,
                reg_bank,
                ref_bank,
                &mut simd_scratch,
                fuel,
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
        cur_ref_base = cursors.ref_base;
        cur_logical_base = cursors.logical_base;
        cur_mem0 = cursors.mem0;
        ip = cursors.ip;
        cur_block_fuel = cursors.block_fuel;
        store.active_local_slots = cursors.active_local_slots;

        match exit {
            InnerExit::SwitchInst => {
                cache_p0_idx = 0;
                cache_p1_idx = 1;
                try_promote_mem0_dense_if_written(store, cur_mem0);
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
                        SlowLoadKind::AddF32 => {
                            *slot = (f32::from_bits(*slot as u32) + f32::from_bits(v as u32))
                                .to_bits() as u64;
                        }
                        SlowLoadKind::AddF64 => {
                            *slot = (f64::from_bits(*slot) + f64::from_bits(v)).to_bits();
                        }
                    }
                    if mem == 0 {
                        try_promote_mem0_dense(store, cur_mem0);
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
                        try_promote_mem0_dense(store, cur_mem0);
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
                        try_promote_mem0_dense(store, cur_mem0);
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
                reg_bank[cur_reg_base + dst as usize] = if m.ty.index_type == IndexType::I64 {
                    pages
                } else {
                    pages as u32 as u64
                };
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
                if dst_mem == 0 || src_mem == 0 {
                    try_promote_mem0_dense(store, cur_mem0);
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
                if mem == 0 {
                    try_promote_mem0_dense(store, cur_mem0);
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
                if mem == 0 {
                    try_promote_mem0_dense(store, cur_mem0);
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
                let len = t.values.len() as u64;
                reg_bank[cur_reg_base + dst as usize] = if t.ty.index_type == IndexType::I64 {
                    len
                } else {
                    len as u32 as u64
                };
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
                arg_ref_base,
                arg_ref_slots,
                logical_arg_start,
                meta_at_call: _,
                meta_after_call,
                src_off,
            } => {
                let (cur_slots, cur_ref_slots) = {
                    let f = store.instances[cur_inst].module.data.register_fast[cur_body]
                        .get()
                        .and_then(|o| o.as_deref())
                        .unwrap();
                    (f.slots as usize, f.ref_slots as usize)
                };
                let (
                    callee_slots,
                    callee_ref_slots,
                    callee_num_locals,
                    callee_local_reg_slots,
                    callee_ref_heaps,
                ) = {
                    let f = store.instances[callee_inst].module.data.register_fast[callee_body]
                        .get()
                        .and_then(|o| o.as_deref())
                        .unwrap();
                    (
                        f.slots as usize,
                        f.ref_slots as usize,
                        f.num_locals as usize,
                        f.local_reg_slots as usize,
                        f.local_ref_heaps.clone(),
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
                let new_ref_base = cur_ref_base + cur_ref_slots;
                let needed_ref = new_ref_base + callee_ref_slots;
                if needed_ref > ref_bank.len() {
                    if ref_bank.try_reserve(needed_ref - ref_bank.len()).is_err() {
                        refund_on_trap!(src_off);
                        return Err(err(ErrorKind::ResourceLimit, "frame allocation failed"));
                    }
                    ref_bank.resize(needed_ref, RefValue::Null(crate::HeapType::None));
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
                let arg_ref_src = cur_ref_base + arg_ref_base as usize;
                let arg_ref_slots_usize = arg_ref_slots as usize;
                if arg_ref_slots_usize > 0 {
                    ref_bank
                        .copy_within(arg_ref_src..arg_ref_src + arg_ref_slots_usize, new_ref_base);
                }
                for (i, &heap) in callee_ref_heaps.iter().enumerate() {
                    ref_bank[new_ref_base + arg_ref_slots_usize + i] = RefValue::Null(heap);
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
                    ref_base: cur_ref_base,
                    ret_dst_base: arg_base,
                    ret_ref_dst_base: arg_ref_base,
                    logical_base: cur_logical_base,
                    meta_after_call,
                    mem0: cur_mem0,
                });
                cur_inst = callee_inst;
                cur_body = callee_body;
                cur_handle = callee_handle;
                cur_reg_base = new_reg_base;
                cur_ref_base = new_ref_base;
                cur_logical_base = next_logical_base;
                cur_mem0 = callee_mem0;
                try_promote_mem0_dense_if_written(store, cur_mem0);
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
                arg_ref_base,
                arg_ref_slots,
                src_off,
            } => {
                let cur_num_locals = store.instances[cur_inst].module.data.register_fast[cur_body]
                    .get()
                    .and_then(|o| o.as_deref())
                    .unwrap()
                    .num_locals as usize;
                let (
                    callee_slots,
                    callee_ref_slots,
                    callee_num_locals,
                    callee_local_reg_slots,
                    callee_ref_heaps,
                ) = {
                    let f = store.instances[callee_inst].module.data.register_fast[callee_body]
                        .get()
                        .and_then(|o| o.as_deref())
                        .unwrap();
                    (
                        f.slots as usize,
                        f.ref_slots as usize,
                        f.num_locals as usize,
                        f.local_reg_slots as usize,
                        f.local_ref_heaps.clone(),
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
                let needed_ref = cur_ref_base + callee_ref_slots;
                if needed_ref > ref_bank.len() {
                    if ref_bank.try_reserve(needed_ref - ref_bank.len()).is_err() {
                        refund_on_trap!(src_off);
                        return Err(err(ErrorKind::ResourceLimit, "frame allocation failed"));
                    }
                    ref_bank.resize(needed_ref, RefValue::Null(crate::HeapType::None));
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
                let arg_ref_src = cur_ref_base + arg_ref_base as usize;
                let arg_ref_slots_usize = arg_ref_slots as usize;
                if arg_ref_slots_usize > 0 && arg_ref_src != cur_ref_base {
                    ref_bank
                        .copy_within(arg_ref_src..arg_ref_src + arg_ref_slots_usize, cur_ref_base);
                }
                for (i, &heap) in callee_ref_heaps.iter().enumerate() {
                    ref_bank[cur_ref_base + arg_ref_slots_usize + i] = RefValue::Null(heap);
                }
                store.release_local_slots(cur_num_locals);
                store.acquire_local_slots(callee_num_locals);
                cur_inst = callee_inst;
                cur_body = callee_body;
                cur_handle = callee_handle;
                cur_mem0 = resolve_mem0(store, callee_inst);
                try_promote_mem0_dense_if_written(store, cur_mem0);
                cache_p0_idx = 0;
                cache_p1_idx = 1;
                ip = 0;
            }
            InnerExit::ReturnTop {
                base,
                count: _,
                ref_base,
                ref_count: _,
            } => {
                let entry_frame = frames.last().unwrap();
                let entry_base = entry_frame.base;
                let entry_locals = entry_frame.locals.len();
                stack.truncate(entry_base);
                let regs = &reg_bank[cur_reg_base..];
                let refs = &ref_bank[cur_ref_base..];
                let cur_func = store.instances[cur_inst].module.data.register_fast[cur_body]
                    .get()
                    .and_then(|o| o.as_deref())
                    .unwrap();
                let cur_num_locals = cur_func.num_locals as usize;
                let mut off = base as usize;
                let mut roff = ref_base as usize;
                for &ty in &cur_func.result_types {
                    stack.push(slots_to_value(ty, regs, off, refs, roff));
                    off += ty.slots() as usize;
                    roff += ty.ref_slots() as usize;
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
    use crate::{Config, Engine, ExecutionPolicy, External, FeatureSet, Linker, RunOutcome};

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
        // Initial 8 pages (512 KiB), max past DENSE_MAX_BYTES so grow-and-demote works.
        let mut mem = vec![1, 0x01];
        leb_u32(&mut mem, 8);
        leb_u32(
            &mut mem,
            (crate::linear::DENSE_MAX_BYTES / 65536 + 64) as u32,
        );
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
            let dense_max_pages = crate::linear::DENSE_MAX_BYTES / 65536;
            cfg.limits.max_memory_pages = (dense_max_pages + 64) as u64;
            cfg.limits.max_memory_bytes = (crate::linear::DENSE_MAX_BYTES + (4 << 20)) as u64;
            cfg.limits.max_store_memory_bytes = (crate::linear::DENSE_MAX_BYTES + (4 << 20)) as u64;
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

            // Grow past DENSE_MAX_BYTES (8 + dense_max_pages pages) -> demotes cleanly to sparse!
            let grow_pages = (crate::linear::DENSE_MAX_BYTES / 65536) as i32;
            let probe_addr = ((grow_pages + 4) * 65536) + 12;
            let probe = store
                .invoke(f_grow, &[Value::I32(grow_pages), Value::I32(probe_addr)])
                .unwrap();
            if policy == ExecutionPolicy::Register {
                let mem_h = store.instances[0].mems[0];
                assert!(
                    !store.mems[mem_h.0.index].bytes.is_dense(),
                    "memory grown past DENSE_MAX_BYTES must demote back to sparse"
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

    #[test]
    fn fast_compile_declines_quadratic_metadata_and_zero_remaining_fuel() {
        // Push 512 i32s onto the operand stack, then run 600 empty block/end pairs
        // (512 * 600 = 307,200 > MAX_FAST_PREFIX_COPIES and MAX_FAST_STACK_TYPES),
        // drop the 512 values, and return 42.
        const DEPTH: usize = 512;
        const BLOCKS: usize = 600;
        let mut insns = Vec::with_capacity(DEPTH * 2 + BLOCKS * 3 + 4);
        for _ in 0..DEPTH {
            insns.extend_from_slice(&[0x41, 0x01]); // i32.const 1
        }
        for _ in 0..BLOCKS {
            insns.extend_from_slice(&[0x02, 0x40, 0x0b]); // block; end
        }
        insns.extend(std::iter::repeat_n(0x1a, DEPTH)); // drop
        insns.extend_from_slice(&[0x41, 0x2a]); // i32.const 42 (`body()` appends final `end`)

        let wasm = wasm_mod(&[
            sec(1, &[1, 0x60, 0, 1, 0x7f]), // type 0: () -> i32
            sec(3, &[1, 0]),                // func 0 -> type 0
            sec(7, &[1, 1, b'f', 0x00, 0]), // export "f" -> func 0
            sec(10, &{
                let mut c = vec![1];
                c.extend_from_slice(&body(&[], &insns));
                c
            }),
        ]);

        for fuel in [None, Some(50_000)] {
            let mut cfg = Config::default();
            cfg.limits.fuel = fuel;
            let engine = Engine::with_execution_policy(cfg, ExecutionPolicy::Register).unwrap();
            let module = engine.compile(&wasm).unwrap();
            assert!(
                !is_admitted(&module, 0),
                "quadratic metadata amplification must cleanly decline register fast tier (fuel={fuel:?})"
            );
            let mut store = Store::new(&engine);
            let inst = Linker::new(&engine)
                .instantiate(&mut store, &module)
                .unwrap();
            let External::Func(f) = inst.export(&store, "f").unwrap() else {
                panic!()
            };
            assert_eq!(
                store.invoke(f, &[]).unwrap(),
                RunOutcome::Returned(vec![Value::I32(42)])
            );
        }

        // With `fuel = Some(1)`, op 0 is charged by `execution.rs` leaving 0 remaining fuel;
        // an uncached multi-instruction function must decline before populating `register_fast`.
        let small_wasm = wasm_mod(&[
            sec(1, &[1, 0x60, 0, 1, 0x7f]),
            sec(3, &[1, 0]),
            sec(7, &[1, 1, b'f', 0x00, 0]),
            sec(10, &{
                let mut c = vec![1];
                c.extend_from_slice(&body(&[], &[0x41, 0x01, 0x41, 0x02, 0x6a]));
                c
            }),
        ]);
        let mut cfg = Config::default();
        cfg.limits.fuel = Some(1);
        let engine = Engine::with_execution_policy(cfg, ExecutionPolicy::Register).unwrap();
        let module = engine.compile(&small_wasm).unwrap();
        let mut store = Store::new(&engine);
        let inst = Linker::new(&engine)
            .instantiate(&mut store, &module)
            .unwrap();
        let External::Func(f) = inst.export(&store, "f").unwrap() else {
            panic!()
        };
        assert_eq!(
            store.invoke(f, &[]).unwrap_err().kind,
            ErrorKind::ResourceLimit
        );
        assert!(
            module.data.register_fast[0].get().is_none(),
            "zero-remaining-fuel call must decline without compiling uncached body"
        );
    }

    #[test]
    fn fast_register_ref_ops_gc_root_survival_and_reentry_parity() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        struct DropFlag(Arc<AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        // Types:
        //  0: (externref, i32) -> (i32, externref)   [entry & tail callee]
        //  1: (i32) -> i32                           [host gc_and_reenter]
        //  2: (externref) -> externref               [nested re-entry & indirect callee]
        //  3: (funcref, i32) -> i32                  [ref.as_non_null / table / eq / traps]
        //  4: (i32) -> (i32, externref)              [block signature for multi-value ref br]
        let mut type_sec = vec![5];
        type_sec.extend_from_slice(&[0x60, 2, 0x6f, 0x7f, 2, 0x7f, 0x6f]); // type 0
        type_sec.extend_from_slice(&[0x60, 1, 0x7f, 1, 0x7f]); // type 1
        type_sec.extend_from_slice(&[0x60, 1, 0x6f, 1, 0x6f]); // type 2
        type_sec.extend_from_slice(&[0x60, 2, 0x70, 0x7f, 1, 0x7f]); // type 3
        type_sec.extend_from_slice(&[0x60, 1, 0x7f, 2, 0x7f, 0x6f]); // type 4

        // Import 0: env.gc_and_reenter : type 1
        let mut imp_sec = vec![1];
        imp_sec.extend_from_slice(&[3, b'e', b'n', b'v', 2, b'g', b'c', 0x00, 1]);

        // Defined functions (indices 1..=4):
        //  func 1 (body 0): type 0  ("entry")
        //  func 2 (body 1): type 2  ("nested")
        //  func 3 (body 2): type 0  ("tail_target")
        //  func 4 (body 3): type 3  ("ref_edge")
        let func_sec = vec![4, 0, 2, 0, 3];

        // Tables:
        //  table 0: externref, min 4
        //  table 1: funcref, min 4
        let tab_sec = vec![2, 0x6f, 0x00, 4, 0x70, 0x00, 4];

        // Globals:
        //  global 0: mut externref = ref.null extern
        let glob_sec = vec![1, 0x6f, 0x01, 0xd0, 0x6f, 0x0b];

        // Exports: "entry" -> 1, "nested" -> 2, "ref_edge" -> 4
        let mut exp_sec = vec![3];
        exp_sec.extend_from_slice(&[5, b'e', b'n', b't', b'r', b'y', 0x00, 1]);
        exp_sec.extend_from_slice(&[6, b'n', b'e', b's', b't', b'e', b'd', 0x00, 2]);
        exp_sec.extend_from_slice(&[8, b'r', b'e', b'f', b'_', b'e', b'd', b'g', b'e', 0x00, 4]);

        // Element section: active table 1 offset 0 -> [func 2] (also declares func 2 & 3 for ref.func!)
        let mut elem_sec = vec![2];
        // Active elem for table 1 at offset 0: [func 2]
        elem_sec.extend_from_slice(&[0x02, 1, 0x41, 0x00, 0x0b, 0x00, 1, 2]);
        // Declarative elem for func 3 (so `ref.func 3` is valid)
        elem_sec.extend_from_slice(&[0x03, 0x00, 1, 3]);

        // Body 0 (func 1, "entry"): (local 0: externref, local 1: i32) -> (i32, externref)
        // Locals: 1 externref (local 2)
        // 1) local.set 2 = local.get 0 (keep a local ref copy)
        // 2) table.set 0[1] = local.get 0; global.set 0 = table.get 0[1]
        // 3) push local.get 0 onto operand stack, THEN call host func 0 (gc_and_reenter)
        //    -> forces clean bailout + materialize_all with ref LIVE in BOTH local 2 and operand stack!
        // 4) after host returns i32, use typed select (0x1c, 1, externref) between stack ref and global.get 0
        // 5) call_indirect table 1 type 2 (func 2 "nested") passing selected externref
        // 6) multi-value block returning (i32, externref) via br_if / br_table
        // 7) return_call func 3 ("tail_target")
        let body0 = body(
            &[(1, 0x6f)],
            &[
                // local.set 2 = local.get 0
                0x20, 0x00, 0x21, 0x02, // table.set 0[1] = local.get 2
                0x41, 0x01, 0x20, 0x02, 0x26, 0x00, // global.set 0 = table.get 0[1]
                0x41, 0x01, 0x25, 0x00, 0x24, 0x00,
                // clear table 0[1] and global 0 BEFORE host collect so ONLY stack + local 2 root the externref!
                0x41, 0x01, 0xd0, 0x6f, 0x26, 0x00, 0xd0, 0x6f, 0x24, 0x00,
                // push local.get 0 onto operand stack (stays live across host call!)
                0x20, 0x00, // call host env.gc(local.get 1)
                0x20, 0x01, 0x10, 0x00,
                // stack: [externref_on_stack, host_i32]
                // local.set 1 = host_i32
                0x21, 0x01, // push local.get 2, push local.get 1, typed select externref
                0x20, 0x02, 0x20, 0x01, 0x1c, 0x01, 0x6f,
                // call_indirect table 1 type 2 (idx 0 -> func 2 "nested")
                0x41, 0x00, 0x11, 0x02, 0x01, // local.tee 2
                0x22, 0x02,
                // multi-value block (type 4: i32 -> (i32, externref))
                0x1a, // drop tee copy
                0x20, 0x01, // push i32 param to block
                0x02, 0x04, // block (type 4)
                // stack inside block: [i32]
                0x20, 0x02, // push externref -> stack: [i32, externref]
                // br_if 0 if local.get 1 != 0
                0x20, 0x01, 0x0d, 0x00,
                // fallthrough if local.get 1 == 0: drop ref, push ref.null extern
                0x1a, 0xd0, 0x6f, 0x0b, // end block -> stack: [i32, externref]
                // swap order to (externref, i32) for tail call to func 3:
                0x21, 0x02, // save externref in local 2
                0x21, 0x01, // save i32 in local 1
                0x20, 0x02, // arg 0: externref
                0x20, 0x01, // arg 1: i32
                0x12, 0x03, // return_call func 3
            ],
        );

        // Body 1 (func 2, "nested"): (externref) -> externref
        // Exercises ref.is_null, select, table.set/get, global.set/get without host bailout!
        let body1 = body(
            &[],
            &[
                // table.set 0[2] = local.get 0
                0x41, 0x02, 0x20, 0x00, 0x26, 0x00, // global.set 0 = table.get 0[2]
                0x41, 0x02, 0x25, 0x00, 0x24, 0x00,
                // return select(global.get 0, ref.null extern, i32.eqz(ref.is_null(local.get 0)))
                0x23, 0x00, 0xd0, 0x6f, 0x20, 0x00, 0xd1, 0x45, 0x1c, 0x01, 0x6f,
                // clear table 0[2] and global 0 so no extra root remains
                0x21, 0x00, 0x41, 0x02, 0xd0, 0x6f, 0x26, 0x00, 0xd0, 0x6f, 0x24, 0x00, 0x20, 0x00,
            ],
        );

        // Body 2 (func 3, "tail_target"): (externref, i32) -> (i32, externref)
        let body2 = body(
            &[],
            &[
                // push (i32 + (ref.is_null(arg0) ? 10 : 1), arg0)
                0x20, 0x01, 0x41, 0x0a, 0x41, 0x01, 0x20, 0x00, 0xd1, 0x1b, 0x6a, 0x20, 0x00,
            ],
        );

        // Body 3 (func 4, "ref_edge"): (funcref, i32) -> i32
        // mode 0: ref.as_non_null(arg0) -> returns 7 (traps on null!)
        // mode 1: table.get 1[99] -> traps OOB!
        // mode 2: table.set 1[99] = ref.func 3 -> traps OOB!
        // mode 3: ref.eq on eqref nulls + ref.func + ref.is_null
        let body3 = body(
            &[(1, 0x70)],
            &[
                // if mode == 0: ref.as_non_null(local 0); return 7
                0x20, 0x01, 0x45, 0x04, 0x40, 0x20, 0x00, 0xd4, 0x1a, 0x41, 0x07, 0x0f, 0x0b,
                // if mode == 1: table.get 1[99]; drop; return 1
                0x20, 0x01, 0x41, 0x01, 0x46, 0x04, 0x40, 0x41, 0xe3, 0x00, 0x25, 0x01, 0x1a, 0x41,
                0x01, 0x0f, 0x0b,
                // if mode == 2: table.set 1[99] = ref.func 3; return 2
                0x20, 0x01, 0x41, 0x02, 0x46, 0x04, 0x40, 0x41, 0xe3, 0x00, 0xd2, 0x03, 0x26, 0x01,
                0x41, 0x02, 0x0f, 0x0b,
                // mode 3: ref.func 3 -> local 2, check !is_null && ref.eq(null eq, null eq)
                0xd2, 0x03, 0x21, 0x02, 0x20, 0x02, 0xd1, 0x45, // 1
                0xd0, 0x6d, 0xd0, 0x6d, 0xd3, // ref.eq(ref.null eq, ref.null eq) == 1
                0x6a, // 2
                0xd0, 0x71, 0xd0, 0x6d, 0xd3, // ref.eq(ref.null none, ref.null eq) == 1
                0x6a, // 3
            ],
        );

        let mut code_sec = vec![4];
        code_sec.extend_from_slice(&body0);
        code_sec.extend_from_slice(&body1);
        code_sec.extend_from_slice(&body2);
        code_sec.extend_from_slice(&body3);

        let wasm = wasm_mod(&[
            sec(1, &type_sec),
            sec(2, &imp_sec),
            sec(3, &func_sec),
            sec(4, &tab_sec),
            sec(6, &glob_sec),
            sec(7, &exp_sec),
            sec(9, &elem_sec),
            sec(10, &code_sec),
        ]);

        // 1) Verify admission AND GC root survival across host `store.collect()` + nested re-entry!
        let mut cfg = Config::default();
        cfg.features.gc = true;
        cfg.features.function_references = true;
        cfg.features.tail_call = true;
        let reg_engine =
            Engine::with_execution_policy(cfg.clone(), ExecutionPolicy::Register).unwrap();
        let reg_mod = reg_engine.compile(&wasm).unwrap();
        for b in 0..4 {
            assert!(
                is_admitted(&reg_mod, b),
                "ref-bearing body {b} must be admitted to register fast tier"
            );
        }

        let dropped = Arc::new(AtomicBool::new(false));
        let reentered = Arc::new(AtomicUsize::new(0));
        let nested_slot: Arc<std::sync::Mutex<Option<crate::Func>>> =
            Arc::new(std::sync::Mutex::new(None));
        let mut store = Store::new(&reg_engine);
        let mut linker = Linker::new(&reg_engine);
        let dropped_cb = Arc::clone(&dropped);
        let reentered_cb = Arc::clone(&reentered);
        let nested_cb = Arc::clone(&nested_slot);
        let host_gc = store
            .create_host_func_with_caller(
                wtime_types::FuncType::new(vec![ValType::I32], vec![ValType::I32]),
                move |caller, args| {
                    let Value::I32(x) = args[0] else {
                        return Err(trap("bad arg"));
                    };
                    // Trigger full GC while outer register frame holds owned externref in local + stack!
                    caller.store.collect()?;
                    assert!(
                        !dropped_cb.load(Ordering::SeqCst),
                        "owned externref rooted in register frame must NOT be collected during host callback"
                    );
                    // Re-enter a register-admitted ref function from inside the host callback!
                    if let Some(nf) = *nested_cb.lock().unwrap() {
                        let tmp = caller.store.create_extern_ref(999)?;
                        let out = caller.store.invoke(nf, &[tmp])?;
                        assert_eq!(out, RunOutcome::Returned(vec![tmp]));
                        assert!(
                            caller.store.fast_ref_bank.is_empty(),
                            "nested fast::run must clear fast_ref_bank before returning"
                        );
                        reentered_cb.fetch_add(1, Ordering::SeqCst);
                    }
                    Ok(vec![Value::I32(x + 5)])
                },
            )
            .unwrap();
        linker.define("env", "gc", External::Func(host_gc)).unwrap();
        let inst = linker.instantiate(&mut store, &reg_mod).unwrap();
        let External::Func(f_entry) = inst.export(&store, "entry").unwrap() else {
            panic!()
        };
        let External::Func(f_nested) = inst.export(&store, "nested").unwrap() else {
            panic!()
        };
        *nested_slot.lock().unwrap() = Some(f_nested);

        let owned = store
            .create_owned_extern_ref(77, DropFlag(Arc::clone(&dropped)))
            .unwrap();
        // Release published roots so ONLY the invocation argument / active frame keeps `owned` alive!
        store.release_published(&[owned]);
        let out = store.invoke(f_entry, &[owned, Value::I32(10)]).unwrap();
        assert_eq!(
            out,
            RunOutcome::Returned(vec![Value::I32(16), owned]),
            "entry must return exact (i32, externref)"
        );
        assert_eq!(reentered.load(Ordering::SeqCst), 1);
        assert!(
            !dropped.load(Ordering::SeqCst),
            "returned published externref must still be live"
        );
        assert!(
            store.fast_ref_bank.is_empty(),
            "pooled fast_ref_bank must be empty after return"
        );
        // Now release published results and collect: proves zero leaked refs in `fast_ref_bank`!
        store.release_published(&[owned]);
        store.collect().unwrap();
        assert!(
            dropped.load(Ordering::SeqCst),
            "owned externref must be reclaimed once published results are released (no stale pool leak)"
        );

        // 2) Fuel-prefix parity across `Decoded` vs `Register` for `ref_edge` (all modes including null & OOB traps)!
        for (mode, fref) in [
            (0, Value::Ref(RefValue::Null(crate::HeapType::Func))),
            (1, Value::Ref(RefValue::Null(crate::HeapType::Func))),
            (2, Value::Ref(RefValue::Null(crate::HeapType::Func))),
            (3, Value::Ref(RefValue::Null(crate::HeapType::Func))),
        ] {
            for fuel_limit in 0u64..=40 {
                let mut pair = Vec::with_capacity(2);
                for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                    let mut fcfg = Config::default();
                    fcfg.features.gc = true;
                    fcfg.features.function_references = true;
                    fcfg.features.tail_call = true;
                    fcfg.limits.fuel = Some(fuel_limit);
                    let eng = Engine::with_execution_policy(fcfg, policy).unwrap();
                    let m = eng.compile(&wasm).unwrap();
                    let mut st = Store::new(&eng);
                    let mut lnk = Linker::new(&eng);
                    let hfn = st
                        .create_host_func_with_caller(
                            wtime_types::FuncType::new(vec![ValType::I32], vec![ValType::I32]),
                            |_c, a| Ok(vec![a[0]]),
                        )
                        .unwrap();
                    lnk.define("env", "gc", External::Func(hfn)).unwrap();
                    let ins = lnk.instantiate(&mut st, &m).unwrap();
                    let External::Func(fe) = ins.export(&st, "ref_edge").unwrap() else {
                        panic!()
                    };
                    let r = st.invoke(fe, &[fref, Value::I32(mode)]);
                    if policy == ExecutionPolicy::Register {
                        assert!(st.fast_ref_bank.is_empty());
                    }
                    pair.push((
                        r.map_err(|e| (e.kind, e.trap_code, e.message)),
                        st.current_fuel,
                    ));
                }
                assert_eq!(
                    pair[0], pair[1],
                    "ref_edge parity mismatch for mode={mode}, fuel={fuel_limit}"
                );
            }
        }
    }

    #[test]
    fn fast_register_superinstructions_and_pure_leaf_parity() {
        // Types:
        //  0: (f64, f64, f64) -> f64         ("fma_check": acc + a * b)
        //  1: (f64, f64, f64) -> f64         ("fma_rev": (a * b) + acc)
        //  2: (i32, i32, i32) -> i32         ("scaled_mem": tests AddShl/ShlImm wrap + local mutation flush)
        //  3: (i32, i32) -> i64              ("scaled_i64": tests LoadI64Shl3Add/StoreI64Shl3Add/LoadI64AddAddr)
        //  4: (i32) -> (i32, i64)            ("multi_leaf": pure_leaf multi-result)
        //  5: () -> ()                       ("void_leaf": pure_leaf 0-result)
        let mut type_sec = vec![6];
        type_sec.extend_from_slice(&[0x60, 3, 0x7c, 0x7c, 0x7c, 1, 0x7c]); // 0
        type_sec.extend_from_slice(&[0x60, 3, 0x7c, 0x7c, 0x7c, 1, 0x7c]); // 1
        type_sec.extend_from_slice(&[0x60, 3, 0x7f, 0x7f, 0x7f, 1, 0x7f]); // 2
        type_sec.extend_from_slice(&[0x60, 2, 0x7f, 0x7f, 1, 0x7e]); // 3
        type_sec.extend_from_slice(&[0x60, 1, 0x7f, 2, 0x7f, 0x7e]); // 4
        type_sec.extend_from_slice(&[0x60, 0, 0]); // 5

        let func_sec = vec![6, 0, 1, 2, 3, 4, 5];
        let mem_sec = vec![1, 0x00, 1]; // 1 page (64 KiB)

        let mut exp_sec = vec![6];
        for (idx, name) in [
            "fma_check",
            "fma_rev",
            "scaled_mem",
            "scaled_i64",
            "multi_leaf",
            "void_leaf",
        ]
        .iter()
        .enumerate()
        {
            exp_sec.push(name.len() as u8);
            exp_sec.extend_from_slice(name.as_bytes());
            exp_sec.push(0x00);
            exp_sec.push(idx as u8);
        }

        // func 0 ("fma_check"): local.get 0 + (local.get 1 * local.get 2) -> exercises F64MulAdd + pure_leaf!
        let b0 = body(&[], &[0x20, 0x00, 0x20, 0x01, 0x20, 0x02, 0xa2, 0xa0]);
        // func 1 ("fma_rev"): (local.get 1 * local.get 2) + local.get 0 -> must preserve prod-first NaN order!
        let b1 = body(&[], &[0x20, 0x01, 0x20, 0x02, 0xa2, 0x20, 0x00, 0xa0]);
        // func 2 ("scaled_mem"): (base: i32, idx: i32, val: i32) -> i32
        //  - stores val at (base + (idx << 2)) via StoreI32Shl2Add
        //  - stores (val ^ 0x55) at (idx * 4 + offset 4) via StoreI32Shl2
        //  - pushes lazy (base + (idx << 2)) onto stack, MUTATES base & idx BEFORE i32.load -> tests flush_local!
        //  - returns sum of both loads!
        let b2 = body(
            &[(1, 0x7f)],
            &[
                // StoreI32Shl2Add: mem[base + (idx << 2)] = val
                0x20, 0x00, 0x20, 0x01, 0x41, 0x02, 0x74, 0x6a, 0x20, 0x02, 0x36, 0x02, 0x00,
                // StoreI32Shl2 (via i32.mul 4): mem[(idx * 4) + 4] = val ^ 0x55
                0x20, 0x01, 0x41, 0x04, 0x6c, 0x20, 0x02, 0x41, 0xd5, 0x00, 0x73, 0x36, 0x02, 0x04,
                // Push lazy AddShl(base, idx, 2) onto stack, THEN mutate local 0 and local 1!
                0x20, 0x00, 0x20, 0x01, 0x41, 0x02, 0x74, 0x6a, 0x41, 0x7f, 0x21,
                0x00, // local.set 0 = 127 (must flush lazy AddShl first!)
                0x41, 0x7f, 0x21, 0x01, // local.set 1 = 127
                0x28, 0x02, 0x00, // i32.load from flushed original address!
                0x21, 0x03,
                // LoadI32Shl2Add on mutated locals (127 + (127 << 2) = 635) + original loaded value
                0x20, 0x00, 0x20, 0x01, 0x41, 0x02, 0x74, 0x6a, 0x28, 0x02, 0x00, 0x20, 0x03, 0x6a,
            ],
        );
        // func 3 ("scaled_i64"): (base: i32, idx: i32) -> i64
        // Exercises StoreI64Shl3Add, LoadI64Shl3Add, and LoadI64AddAddr!
        let b3 = body(
            &[],
            &[
                // mem64[base + (idx << 3)] = 0x1122334455667788
                0x20, 0x00, 0x20, 0x01, 0x41, 0x03, 0x74, 0x6a, 0x42, 0x88, 0xef, 0x99, 0xab, 0xc4,
                0xe6, 0xa8, 0x89, 0x11, // i64.const
                0x37, 0x03, 0x00, // LoadI64Shl3Add
                0x20, 0x00, 0x20, 0x01, 0x41, 0x03, 0x74, 0x6a, 0x29, 0x03, 0x00,
                // LoadI64AddAddr (base + (idx * 8) where idx*8 is materialized via i32.mul 16 >> 1)
                0x20, 0x00, 0x20, 0x01, 0x41, 0x10, 0x6c, 0x41, 0x01, 0x76, 0x6a, 0x29, 0x03, 0x00,
                0x7c, // i64.add
            ],
        );
        // func 4 ("multi_leaf"): (x: i32) -> (i32, i64)
        let b4 = body(
            &[],
            &[
                0x20, 0x00, 0x41, 0x07, 0x6a, 0x20, 0x00, 0xac, 0x42, 0x03, 0x7c,
            ],
        );
        // func 5 ("void_leaf"): () -> ()
        let b5 = body(&[], &[]);

        let mut code_sec = vec![6];
        for b in [&b0, &b1, &b2, &b3, &b4, &b5] {
            code_sec.extend_from_slice(b);
        }

        let wasm = wasm_mod(&[
            sec(1, &type_sec),
            sec(3, &func_sec),
            sec(5, &mem_sec),
            sec(7, &exp_sec),
            sec(10, &code_sec),
        ]);

        // Verify `pure_leaf` classification on unfueled Register engine!
        let reg_eng =
            Engine::with_execution_policy(Config::default(), ExecutionPolicy::Register).unwrap();
        let reg_mod = reg_eng.compile(&wasm).unwrap();
        assert!(get_or_compile_ref(&reg_mod, 0).unwrap().pure_leaf);
        assert!(get_or_compile_ref(&reg_mod, 1).unwrap().pure_leaf);
        assert!(!get_or_compile_ref(&reg_mod, 2).unwrap().pure_leaf);
        assert!(get_or_compile_ref(&reg_mod, 4).unwrap().pure_leaf);
        assert!(get_or_compile_ref(&reg_mod, 5).unwrap().pure_leaf);

        // 1) F64MulAdd two-rounding IEEE-754 + signed zero + dual-NaN payload parity!
        let one_plus_eps = f64::from_bits(1.0_f64.to_bits() + 1);
        let one_minus_eps = f64::from_bits(1.0_f64.to_bits() - 1);
        let qnan1 = 0x7ff8_0000_0000_0001_u64;
        let qnan2 = 0x7ff8_0000_0000_0002_u64;
        let fma_cases = [
            (
                (-1.0_f64).to_bits(),
                one_plus_eps.to_bits(),
                one_minus_eps.to_bits(),
            ),
            (
                (-0.0_f64).to_bits(),
                (-0.0_f64).to_bits(),
                1.0_f64.to_bits(),
            ),
            (qnan1, qnan2, 1.0_f64.to_bits()),
            (qnan2, qnan1, 1.0_f64.to_bits()),
        ];
        for fname in ["fma_check", "fma_rev"] {
            for (acc, a, b) in fma_cases {
                let mut pair = Vec::new();
                for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                    let eng = Engine::with_execution_policy(Config::default(), policy).unwrap();
                    let m = eng.compile(&wasm).unwrap();
                    let mut st = Store::new(&eng);
                    let ins = Linker::new(&eng).instantiate(&mut st, &m).unwrap();
                    let External::Func(f) = ins.export(&st, fname).unwrap() else {
                        panic!()
                    };
                    pair.push(
                        st.invoke(f, &[Value::F64(acc), Value::F64(a), Value::F64(b)])
                            .unwrap(),
                    );
                }
                assert_eq!(pair[0], pair[1], "{fname} bit-exact parity mismatch");
            }
        }

        // 2) Scaled memory superinstructions: 32-bit wrap-around + local mutation flush + OOB fuel prefix!
        let mem_cases = [
            (0i32, 3i32, 0x1234_5678i32),
            // 32-bit wrap-around: base = 0xffff_fffc (-4), idx = 1 => (-4 + 4) as u32 = 0!
            (-4i32, 1i32, 0x0bad_f00di32),
            // OOB trap case: base = 65536, idx = 1
            (65536i32, 1i32, 99i32),
        ];
        for (base, idx, val) in mem_cases {
            for fuel in [
                None,
                Some(0),
                Some(1),
                Some(5),
                Some(12),
                Some(20),
                Some(30),
                Some(60),
            ] {
                let mut pair = Vec::new();
                for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                    let mut c = Config::default();
                    c.limits.fuel = fuel;
                    let eng = Engine::with_execution_policy(c, policy).unwrap();
                    let m = eng.compile(&wasm).unwrap();
                    let mut st = Store::new(&eng);
                    let ins = Linker::new(&eng).instantiate(&mut st, &m).unwrap();
                    let External::Func(f2) = ins.export(&st, "scaled_mem").unwrap() else {
                        panic!()
                    };
                    let External::Func(f3) = ins.export(&st, "scaled_i64").unwrap() else {
                        panic!()
                    };
                    let r2 = st
                        .invoke(f2, &[Value::I32(base), Value::I32(idx), Value::I32(val)])
                        .map_err(|e| (e.kind, e.trap_code, e.message));
                    let f2_rem = st.current_fuel;
                    let r3 = st
                        .invoke(f3, &[Value::I32(base), Value::I32(idx)])
                        .map_err(|e| (e.kind, e.trap_code, e.message));
                    let f3_rem = st.current_fuel;
                    pair.push((r2, f2_rem, r3, f3_rem));
                }
                assert_eq!(
                    pair[0], pair[1],
                    "scaled mem parity mismatch for base={base}, idx={idx}, fuel={fuel:?}"
                );
            }
        }

        // 3) Pure leaf zero-frame entry: multi-result, void-result, error parity, and cancellation!
        let mut st = Store::new(&reg_eng);
        let ins = Linker::new(&reg_eng)
            .instantiate(&mut st, &reg_mod)
            .unwrap();
        let External::Func(f_multi) = ins.export(&st, "multi_leaf").unwrap() else {
            panic!()
        };
        let External::Func(f_void) = ins.export(&st, "void_leaf").unwrap() else {
            panic!()
        };
        assert_eq!(
            st.invoke(f_multi, &[Value::I32(-10)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(-3), Value::I64(-7)])
        );
        assert_eq!(
            st.invoke(f_void, &[]).unwrap(),
            RunOutcome::Returned(vec![])
        );
        // Wrong argument type / count must fall through to canonical error!
        assert_eq!(
            st.invoke(f_multi, &[Value::I64(1)]).unwrap_err().kind,
            ErrorKind::Host
        );
        assert_eq!(st.invoke(f_multi, &[]).unwrap_err().kind, ErrorKind::Host);
        // Cancellation must still be honored before leaf execution!
        st.cancellation_token().cancel();
        assert_eq!(
            st.invoke(f_multi, &[Value::I32(1)]).unwrap_err().kind,
            ErrorKind::ResourceLimit
        );
    }

    #[test]
    fn fast_register_store_peephole_lhs_imm_no_clobber_and_exact_func_import() {
        // 1) Regression for commutative LHS immediate in StoreI32Shl2 / StoreI32AddImm:
        //    `i32.const 4; local.get 0; i32.const 1; i32.add; i32.mul; i32.const 99; i32.store`
        //    and `i32.const 16; local.get 0; i32.const 1; i32.add; i32.add; i32.const 77; i32.store`
        //    must not let `mat(val)` overwrite the popped `idx` / `base_reg` temp slot!
        let type_sec = vec![1, 0x60, 1, 0x7f, 0];
        let func_sec = vec![1, 0];
        let mem_sec = vec![1, 0x00, 1];
        let exp_sec = vec![2, 3, b'b', b'u', b'g', 0x00, 0, 1, b'm', 0x02, 0];
        let b0 = body(
            &[],
            &[
                // mem[((x + 1) * 4)] = 99
                0x41, 0x04, 0x20, 0x00, 0x41, 0x01, 0x6a, 0x6c, 0x41, 0xe3, 0x00, 0x36, 0x02, 0x00,
                // mem[(16 + (x + 1))] = 77
                0x41, 0x10, 0x20, 0x00, 0x41, 0x01, 0x6a, 0x6a, 0x41, 0xcd, 0x00, 0x36, 0x02, 0x00,
            ],
        );
        let mut code_sec = vec![1];
        code_sec.extend_from_slice(&b0);
        let wasm = wasm_mod(&[
            sec(1, &type_sec),
            sec(3, &func_sec),
            sec(5, &mem_sec),
            sec(7, &exp_sec),
            sec(10, &code_sec),
        ]);
        let mut imgs = Vec::new();
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
            let eng = Engine::with_execution_policy(Config::default(), policy).unwrap();
            let m = eng.compile(&wasm).unwrap();
            let mut st = Store::new(&eng);
            let ins = Linker::new(&eng).instantiate(&mut st, &m).unwrap();
            let External::Func(f) = ins.export(&st, "bug").unwrap() else {
                panic!()
            };
            let External::Memory(mem) = ins.export(&st, "m").unwrap() else {
                panic!()
            };
            st.invoke(f, &[Value::I32(0)]).unwrap();
            let bytes = st.read_memory(mem, 0, 512).unwrap();
            assert_eq!(&bytes[4..8], &99i32.to_le_bytes());
            assert_eq!(&bytes[17..21], &77i32.to_le_bytes());
            imgs.push(bytes);
        }
        assert_eq!(imgs[0], imgs[1]);

        // 2) Regression for `declared_func_type` with `ImportKind::ExactFunc`:
        //    Import 0 is exact func type 0 `(i32) -> i32`, defined func 1 is type 1 `(i64) -> i64`.
        //    Calling func 0 and taking `ref.func 1` must resolve exact signatures!
        let exact_wasm = wasm_mod(&[
            sec(1, &[2, 0x60, 1, 0x7f, 1, 0x7f, 0x60, 1, 0x7e, 1, 0x7e]),
            // Import 0: env.f : exact func (0x20 <typeidx=0>)
            sec(2, &[1, 3, b'e', b'n', b'v', 1, b'f', 0x20, 0x00]),
            sec(3, &[1, 1]), // defined func 1 -> type 1: (i64) -> i64
            sec(7, &[1, 1, b'g', 0x00, 1]),
            sec(9, &[1, 0x03, 0x00, 1, 1]), // declarative elem for ref.func 1
            sec(10, &{
                let mut c = vec![1];
                // call 0 with i32(41), extend to i64, add arg0, check !ref.is_null(ref.func 1)
                c.extend_from_slice(&body(
                    &[],
                    &[
                        0x41, 0x29, 0x10, 0x00, 0xac, 0x20, 0x00, 0x7c, 0xd2, 0x01, 0xd1, 0x45,
                        0xac, 0x7c,
                    ],
                ));
                c
            }),
        ]);
        let mut cfg = Config::default();
        cfg.features.custom_descriptors = true;
        cfg.features.gc = true;
        cfg.features.function_references = true;
        let eng = Engine::with_execution_policy(cfg, ExecutionPolicy::Register).unwrap();
        let m = eng.compile(&exact_wasm).unwrap();
        assert!(
            is_admitted(&m, 0),
            "function referencing ExactFunc import and ref.func 1 must be admitted"
        );
        let mut st = Store::new(&eng);
        let h = st
            .create_host_func_with_caller(
                wtime_types::FuncType::new(vec![ValType::I32], vec![ValType::I32]),
                |_c, a| {
                    let Value::I32(n) = a[0] else {
                        return Err(trap("arg"));
                    };
                    Ok(vec![Value::I32(n + 1)])
                },
            )
            .unwrap();
        let mut lnk = Linker::new(&eng);
        lnk.define("env", "f", External::Func(h)).unwrap();
        let ins = lnk.instantiate(&mut st, &m).unwrap();
        let External::Func(fg) = ins.export(&st, "g").unwrap() else {
            panic!()
        };
        assert_eq!(
            st.invoke(fg, &[Value::I64(10)]).unwrap(),
            RunOutcome::Returned(vec![Value::I64(53)])
        );
    }

    fn emit_v128_const(code: &mut Vec<u8>, bytes: [u8; 16]) {
        code.extend_from_slice(&[0xfd, 0x0c]);
        code.extend_from_slice(&bytes);
    }

    fn v128_from_f32s(lanes: [u32; 4]) -> [u8; 16] {
        let mut out = [0u8; 16];
        for (i, bits) in lanes.iter().enumerate() {
            out[i * 4..(i + 1) * 4].copy_from_slice(&bits.to_le_bytes());
        }
        out
    }

    fn v128_from_f64s(lanes: [u64; 2]) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[0..8].copy_from_slice(&lanes[0].to_le_bytes());
        out[8..16].copy_from_slice(&lanes[1].to_le_bytes());
        out
    }

    fn simd_op(code: &mut Vec<u8>, sub: u32) {
        code.push(0xfd);
        leb_u32(code, sub);
    }

    #[test]
    fn fast_register_unboxed_v128_fast_ops_nan_oob_fuel_and_mixed_ref_parity() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct DropFlag(Arc<AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        // Types:
        //  0: (i32) -> i32                  (host callback "gc")
        //  1: (v128, v128, i32) -> v128     ("simd_ops": exercises all new unboxed v128 RegOps + NaN)
        //  2: (i32, v128) -> v128           ("simd_mem": v128.store + v128.load, OOB traps + fuel)
        //  3: (externref, i32, v128) -> (v128, externref, i32) ("mixed_ref": scalar + v128 + GC ref)
        //  4: (v128, v128, v128) -> v128    ("simd_fma": f32x4.mul + f32x4.add -> F32x4MulAdd)
        let mut type_sec = vec![5];
        type_sec.extend_from_slice(&[0x60, 1, 0x7f, 1, 0x7f]); // 0
        type_sec.extend_from_slice(&[0x60, 3, 0x7b, 0x7b, 0x7f, 1, 0x7b]); // 1
        type_sec.extend_from_slice(&[0x60, 2, 0x7f, 0x7b, 1, 0x7b]); // 2
        type_sec.extend_from_slice(&[0x60, 3, 0x6f, 0x7f, 0x7b, 3, 0x7b, 0x6f, 0x7f]); // 3
        type_sec.extend_from_slice(&[0x60, 3, 0x7b, 0x7b, 0x7b, 1, 0x7b]); // 4

        let imp_sec = vec![1, 3, b'e', b'n', b'v', 2, b'g', b'c', 0x00, 0];
        let func_sec = vec![4, 1, 2, 3, 4];
        let mem_sec = vec![1, 0x00, 1]; // 1 page (64 KiB)
        // Active data segment so mem0 starts with written storage and promotes dense!
        let mut data_sec = vec![1, 0x00, 0x41, 0x00, 0x0b, 4, 1, 2, 3, 4];
        let _ = &mut data_sec;

        let mut exp_sec = vec![4];
        for (idx, name) in ["simd_ops", "simd_mem", "mixed_ref", "simd_fma"]
            .iter()
            .enumerate()
        {
            exp_sec.push(name.len() as u8);
            exp_sec.extend_from_slice(name.as_bytes());
            exp_sec.push(0x00);
            exp_sec.push((idx + 1) as u8); // func 0 is import
        }

        // Body 0 ("simd_ops"): (a: v128, b: v128, mode: i32) -> v128
        // Dispatch by `mode` across every unboxed v128 fast op so each op's exact bit pattern
        // (including NaN canonicalization / sNaN payload preservation) is tested individually!
        let mut s_code = Vec::new();
        let mut emit_case = |mode: i32, f: &dyn Fn(&mut Vec<u8>)| {
            s_code.extend_from_slice(&[0x20, 0x02, 0x41]);
            leb_u32(&mut s_code, mode as u32);
            s_code.extend_from_slice(&[0x46, 0x04, 0x40]); // if local2 == mode
            f(&mut s_code);
            s_code.extend_from_slice(&[0x0f, 0x0b]); // return; end
        };
        // 0..=3: f32x4.{add, sub, mul, div}
        for (m, sub) in [(0, 228u32), (1, 229), (2, 230), (3, 231)] {
            emit_case(m, &|c| {
                c.extend_from_slice(&[0x20, 0x00, 0x20, 0x01]);
                simd_op(c, sub);
            });
        }
        // 4..=5: f32x4.{abs, neg}
        for (m, sub) in [(4, 224u32), (5, 225)] {
            emit_case(m, &|c| {
                c.extend_from_slice(&[0x20, 0x00]);
                simd_op(c, sub);
            });
        }
        // 6..=9: f32x4.{min, max, pmin, pmax}
        for (m, sub) in [(6, 232u32), (7, 233), (8, 234), (9, 235)] {
            emit_case(m, &|c| {
                c.extend_from_slice(&[0x20, 0x00, 0x20, 0x01]);
                simd_op(c, sub);
            });
        }
        // 10..=15: f32x4.{eq, ne, lt, gt, le, ge}
        for (m, sub) in [
            (10, 65u32),
            (11, 66),
            (12, 67),
            (13, 68),
            (14, 69),
            (15, 70),
        ] {
            emit_case(m, &|c| {
                c.extend_from_slice(&[0x20, 0x00, 0x20, 0x01]);
                simd_op(c, sub);
            });
        }
        // 16..=19: f64x2.{add, sub, mul, div}
        for (m, sub) in [(16, 240u32), (17, 241), (18, 242), (19, 243)] {
            emit_case(m, &|c| {
                c.extend_from_slice(&[0x20, 0x00, 0x20, 0x01]);
                simd_op(c, sub);
            });
        }
        // 20..=22: i32x4.{add, sub, mul}
        for (m, sub) in [(20, 174u32), (21, 177), (22, 181)] {
            emit_case(m, &|c| {
                c.extend_from_slice(&[0x20, 0x00, 0x20, 0x01]);
                simd_op(c, sub);
            });
        }
        // 23..=24: i64x2.{add, sub}
        for (m, sub) in [(23, 206u32), (24, 209)] {
            emit_case(m, &|c| {
                c.extend_from_slice(&[0x20, 0x00, 0x20, 0x01]);
                simd_op(c, sub);
            });
        }
        // 25..=29: v128.{not, and, andnot, or, xor}
        emit_case(25, &|c| {
            c.extend_from_slice(&[0x20, 0x00]);
            simd_op(c, 77);
        });
        for (m, sub) in [(26, 78u32), (27, 79), (28, 80), (29, 81)] {
            emit_case(m, &|c| {
                c.extend_from_slice(&[0x20, 0x00, 0x20, 0x01]);
                simd_op(c, sub);
            });
        }
        // 30: v128.bitselect(a, b, mask_const)
        emit_case(30, &|c| {
            c.extend_from_slice(&[0x20, 0x00, 0x20, 0x01]);
            emit_v128_const(
                c,
                [
                    0xff, 0x00, 0xaa, 0x55, 0x0f, 0xf0, 0xcc, 0x33, 0xff, 0xff, 0x00, 0x00, 0x5a,
                    0xa5, 0x81, 0x7e,
                ],
            );
            simd_op(c, 82);
        });
        // 31: i8x16.shuffle(a, b) crossing both vectors
        emit_case(31, &|c| {
            c.extend_from_slice(&[0x20, 0x00, 0x20, 0x01, 0xfd, 0x0d]);
            c.extend_from_slice(&[31, 0, 16, 15, 20, 3, 28, 7, 17, 14, 25, 2, 19, 11, 30, 8]);
        });
        // 32: i32x4.splat(i32.const 0x12345678)
        emit_case(32, &|c| {
            c.extend_from_slice(&[0x41, 0xf8, 0xac, 0xd1, 0x91, 0x01]);
            simd_op(c, 17);
        });
        // 33: f32x4.splat(f32.const sNaN 0x7f800007) -> must preserve exact sNaN bits!
        emit_case(33, &|c| {
            c.push(0x43);
            c.extend_from_slice(&0x7f80_0007_u32.to_le_bytes());
            simd_op(c, 19);
        });
        // 34: i64x2.splat(i64.const -0x123456789)
        emit_case(34, &|c| {
            c.extend_from_slice(&[0x42, 0xf7, 0xd4, 0xd6, 0x9b, 0x6d]);
            simd_op(c, 18);
        });
        // 35: f64x2.splat(f64.const sNaN 0x7ff0000000000009) -> must preserve exact sNaN bits!
        emit_case(35, &|c| {
            c.push(0x44);
            c.extend_from_slice(&0x7ff0_0000_0000_0009_u64.to_le_bytes());
            simd_op(c, 20);
        });
        // default fallback: v128.const
        emit_v128_const(&mut s_code, [0x42; 16]);
        let b0 = body(&[], &s_code);

        // Body 1 ("simd_mem"): (addr: i32, v: v128) -> v128
        // Stores `v` at `addr` (offset 0), loads `v128` at `addr` (offset 0), adds `i32x4.splat(1)`.
        let mut m_code = Vec::new();
        m_code.extend_from_slice(&[0x20, 0x00, 0x20, 0x01, 0xfd, 0x0b, 0x00, 0x00]); // v128.store
        m_code.extend_from_slice(&[0x20, 0x00, 0xfd, 0x00, 0x00, 0x00]); // v128.load
        m_code.extend_from_slice(&[0x41, 0x01]);
        simd_op(&mut m_code, 17); // i32x4.splat(1)
        simd_op(&mut m_code, 174); // i32x4.add
        let b1 = body(&[], &m_code);

        // Body 2 ("mixed_ref"): (r: externref, n: i32, v: v128) -> (v128, externref, i32)
        // Locals: local 3 = v128, local 4 = externref
        let mut r_code = Vec::new();
        // local 3 = v + (v * splat_f32(2.0)) -> exercises F32x4MulAdd alongside ref local!
        r_code.extend_from_slice(&[0x20, 0x02, 0x20, 0x02, 0x43]);
        r_code.extend_from_slice(&2.0_f32.to_bits().to_le_bytes());
        simd_op(&mut r_code, 19); // f32x4.splat
        simd_op(&mut r_code, 230); // f32x4.mul
        simd_op(&mut r_code, 228); // f32x4.add
        r_code.extend_from_slice(&[0x21, 0x03]); // local.set 3
        // local 4 = r; store local 3 to mem[64] (SLEB128 +64 = 0xc0, 0x00)
        r_code.extend_from_slice(&[0x20, 0x00, 0x21, 0x04]);
        r_code.extend_from_slice(&[0x41, 0xc0, 0x00, 0x20, 0x03, 0xfd, 0x0b, 0x00, 0x00]);
        // push r & local 3 onto operand stack across host GC call!
        r_code.extend_from_slice(&[0x41, 0xc0, 0x00, 0xfd, 0x00, 0x00, 0x00]); // v128 on stack
        r_code.extend_from_slice(&[0x20, 0x04]); // externref on stack
        r_code.extend_from_slice(&[0x20, 0x01, 0x10, 0x00]); // call host gc(n) -> n + 3
        let b2 = body(&[(1, 0x7b), (1, 0x6f)], &r_code);

        // Body 3 ("simd_fma"): (acc: v128, a: v128, b: v128) -> v128
        // acc + (a * b) -> compiles to F32x4MulAdd!
        let mut fma_code = vec![0x20, 0x00, 0x20, 0x01, 0x20, 0x02];
        simd_op(&mut fma_code, 230); // f32x4.mul
        simd_op(&mut fma_code, 228); // f32x4.add
        let b3 = body(&[], &fma_code);

        let mut code_sec = vec![4];
        for b in [&b0, &b1, &b2, &b3] {
            code_sec.extend_from_slice(b);
        }

        let wasm = wasm_mod(&[
            sec(1, &type_sec),
            sec(2, &imp_sec),
            sec(3, &func_sec),
            sec(5, &mem_sec),
            sec(7, &exp_sec),
            sec(10, &code_sec),
            sec(11, &data_sec),
        ]);

        // 1) Verify compilation emits dedicated unboxed v128 RegOps (not SimdUnary/Binary/Ternary)!
        let mut cfg = Config::default();
        cfg.features.simd = true;
        cfg.features.reference_types = true;
        cfg.features.gc = true;
        let reg_eng =
            Engine::with_execution_policy(cfg.clone(), ExecutionPolicy::Register).unwrap();
        let reg_mod = reg_eng.compile(&wasm).unwrap();
        let f0 = get_or_compile_ref(&reg_mod, 0).expect("simd_ops admitted");
        assert!(
            f0.ops.iter().all(|op| !matches!(
                op,
                RegOp::SimdUnary { .. } | RegOp::SimdBinary { .. } | RegOp::SimdTernary { .. }
            )),
            "all hot v128 ops in simd_ops must lower to dedicated unboxed RegOps"
        );
        assert!(
            f0.ops
                .iter()
                .any(|op| matches!(op, RegOp::V128Splat32 { .. }))
        );
        assert!(
            f0.ops
                .iter()
                .any(|op| matches!(op, RegOp::V128Splat64 { .. }))
        );
        assert!(f0.ops.iter().any(|op| matches!(op, RegOp::F32x4Min { .. })));
        assert!(f0.ops.iter().any(|op| matches!(op, RegOp::F32x4Cmp { .. })));
        assert!(
            f0.ops
                .iter()
                .any(|op| matches!(op, RegOp::V128Bitselect { .. }))
        );
        assert!(
            f0.ops
                .iter()
                .any(|op| matches!(op, RegOp::SimdShuffle { .. }))
        );
        let f1 = get_or_compile_ref(&reg_mod, 1).expect("simd_mem admitted");
        assert!(f1.ops.iter().any(|op| matches!(op, RegOp::LoadV128 { .. })));
        assert!(
            f1.ops
                .iter()
                .any(|op| matches!(op, RegOp::StoreV128 { .. }))
        );
        let f3 = get_or_compile_ref(&reg_mod, 3).expect("simd_fma admitted");
        assert!(
            f3.ops
                .iter()
                .any(|op| matches!(op, RegOp::F32x4MulAdd { .. }))
        );

        // 2) Bit-exact parity across ALL 36 modes with tricky NaN, -0.0, +0.0, inf, overflow payloads!
        let f32_a = v128_from_f32s([
            0x7f80_0001,          // +sNaN
            0xffc0_1234,          // -qNaN payload
            (-0.0_f32).to_bits(), // -0.0
            f32::INFINITY.to_bits(),
        ]);
        let f32_b = v128_from_f32s([
            0x7fc0_5678,       // +qNaN payload
            1.5_f32.to_bits(), // finite
            0.0_f32.to_bits(), // +0.0
            f32::NEG_INFINITY.to_bits(),
        ]);
        let f32_c = v128_from_f32s([
            (-3.25_f32).to_bits(),
            2.5_f32.to_bits(),
            0x0000_0001, // subnormal
            1.0_f32.to_bits(),
        ]);
        let f64_a = v128_from_f64s([0x7ff0_0000_0000_0001, (-0.0_f64).to_bits()]);
        let f64_b = v128_from_f64s([3.5_f64.to_bits(), f64::INFINITY.to_bits()]);
        let int_a = v128_from_f32s([0x7fff_ffff, 0x0000_0000, 0x0001_0001, 0xffff_ffff]);
        let int_b = v128_from_f32s([0x0000_0001, 0x0000_0001, 0x0001_0001, 0x8000_0000]);

        for (va, vb) in [
            (f32_a, f32_b),
            (f32_b, f32_c),
            (f64_a, f64_b),
            (int_a, int_b),
        ] {
            for mode in 0..=36 {
                let mut pair = Vec::with_capacity(2);
                for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                    let eng = Engine::with_execution_policy(cfg.clone(), policy).unwrap();
                    let m = eng.compile(&wasm).unwrap();
                    let mut st = Store::new(&eng);
                    let h = st
                        .create_host_func_with_caller(
                            wtime_types::FuncType::new(vec![ValType::I32], vec![ValType::I32]),
                            |_c, a| Ok(vec![a[0]]),
                        )
                        .unwrap();
                    let mut lnk = Linker::new(&eng);
                    lnk.define("env", "gc", External::Func(h)).unwrap();
                    let ins = lnk.instantiate(&mut st, &m).unwrap();
                    let External::Func(f) = ins.export(&st, "simd_ops").unwrap() else {
                        panic!()
                    };
                    pair.push(
                        st.invoke(
                            f,
                            &[
                                Value::V128(V128(va)),
                                Value::V128(V128(vb)),
                                Value::I32(mode),
                            ],
                        )
                        .unwrap(),
                    );
                }
                assert_eq!(
                    pair[0], pair[1],
                    "simd_ops bit-exact parity mismatch for mode={mode}"
                );
            }
        }

        // 3) F32x4MulAdd two-rounding IEEE-754 + dual-NaN payload parity!
        let f32_one_plus = f32::from_bits(1.0_f32.to_bits() + 1).to_bits();
        let f32_one_minus = f32::from_bits(1.0_f32.to_bits() - 1).to_bits();
        let fma_acc = v128_from_f32s([
            (-1.0_f32).to_bits(),
            0x7fc0_0001,
            (-0.0_f32).to_bits(),
            10.0_f32.to_bits(),
        ]);
        let fma_a = v128_from_f32s([
            f32_one_plus,
            0x7fc0_0002,
            (-0.0_f32).to_bits(),
            3.5_f32.to_bits(),
        ]);
        let fma_b = v128_from_f32s([
            f32_one_minus,
            1.0_f32.to_bits(),
            1.0_f32.to_bits(),
            2.0_f32.to_bits(),
        ]);
        {
            let mut pair = Vec::with_capacity(2);
            for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                let eng = Engine::with_execution_policy(cfg.clone(), policy).unwrap();
                let m = eng.compile(&wasm).unwrap();
                let mut st = Store::new(&eng);
                let h = st
                    .create_host_func_with_caller(
                        wtime_types::FuncType::new(vec![ValType::I32], vec![ValType::I32]),
                        |_c, a| Ok(vec![a[0]]),
                    )
                    .unwrap();
                let mut lnk = Linker::new(&eng);
                lnk.define("env", "gc", External::Func(h)).unwrap();
                let ins = lnk.instantiate(&mut st, &m).unwrap();
                let External::Func(f) = ins.export(&st, "simd_fma").unwrap() else {
                    panic!()
                };
                pair.push(
                    st.invoke(
                        f,
                        &[
                            Value::V128(V128(fma_acc)),
                            Value::V128(V128(fma_a)),
                            Value::V128(V128(fma_b)),
                        ],
                    )
                    .unwrap(),
                );
            }
            assert_eq!(pair[0], pair[1], "F32x4MulAdd bit-exact parity mismatch");
        }

        // 4) v128.load/v128.store valid + OOB traps + mid-block fuel exhaustion parity!
        for addr in [0i32, 65520i32, 65521i32, 65536i32, -1i32] {
            for fuel in 0u64..=12 {
                let mut pair = Vec::with_capacity(2);
                for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                    let mut fcfg = cfg.clone();
                    fcfg.limits.fuel = Some(fuel);
                    let eng = Engine::with_execution_policy(fcfg, policy).unwrap();
                    let m = eng.compile(&wasm).unwrap();
                    let mut st = Store::new(&eng);
                    let h = st
                        .create_host_func_with_caller(
                            wtime_types::FuncType::new(vec![ValType::I32], vec![ValType::I32]),
                            |_c, a| Ok(vec![a[0]]),
                        )
                        .unwrap();
                    let mut lnk = Linker::new(&eng);
                    lnk.define("env", "gc", External::Func(h)).unwrap();
                    let ins = lnk.instantiate(&mut st, &m).unwrap();
                    let External::Func(f) = ins.export(&st, "simd_mem").unwrap() else {
                        panic!()
                    };
                    let res = st
                        .invoke(f, &[Value::I32(addr), Value::V128(V128(int_a))])
                        .map_err(|e| (e.kind, e.trap_code, e.message));
                    pair.push((res, st.current_fuel));
                }
                assert_eq!(
                    pair[0], pair[1],
                    "simd_mem parity mismatch for addr={addr}, fuel={fuel}"
                );
            }
        }

        // 5) Mixed scalar + v128 + GC externref frame: GC root survival + parity!
        let dropped = Arc::new(AtomicBool::new(false));
        let mut st = Store::new(&reg_eng);
        let dropped_cb = Arc::clone(&dropped);
        let h_gc = st
            .create_host_func_with_caller(
                wtime_types::FuncType::new(vec![ValType::I32], vec![ValType::I32]),
                move |caller, a| {
                    let Value::I32(x) = a[0] else {
                        return Err(trap("bad arg"));
                    };
                    caller.store.collect()?;
                    assert!(
                        !dropped_cb.load(Ordering::SeqCst),
                        "externref in mixed scalar+v128+ref frame must survive GC"
                    );
                    Ok(vec![Value::I32(x + 3)])
                },
            )
            .unwrap();
        let mut lnk = Linker::new(&reg_eng);
        lnk.define("env", "gc", External::Func(h_gc)).unwrap();
        let ins = lnk.instantiate(&mut st, &reg_mod).unwrap();
        let External::Func(f_mix) = ins.export(&st, "mixed_ref").unwrap() else {
            panic!()
        };
        let owned = st
            .create_owned_extern_ref(42, DropFlag(Arc::clone(&dropped)))
            .unwrap();
        st.release_published(&[owned]);
        let in_v = v128_from_f32s([
            1.0_f32.to_bits(),
            2.0_f32.to_bits(),
            3.0_f32.to_bits(),
            4.0_f32.to_bits(),
        ]);
        let exp_v = v128_from_f32s([
            3.0_f32.to_bits(),
            6.0_f32.to_bits(),
            9.0_f32.to_bits(),
            12.0_f32.to_bits(),
        ]);
        let out = st
            .invoke(f_mix, &[owned, Value::I32(10), Value::V128(V128(in_v))])
            .unwrap();
        assert_eq!(
            out,
            RunOutcome::Returned(vec![Value::V128(V128(exp_v)), owned, Value::I32(13)])
        );
        st.release_published(&[owned]);
        st.collect().unwrap();
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn fast_register_fused_loop_load_store_superinstructions_parity() {
        // Types:
        //  0: (i32, i32) -> i32      ("loop_ne": I32IncNeBrIf + StoreI32Imm + StoreI32_8Imm + LoadI32Add + LoadI32Shl2AddAcc)
        //  1: (i32) -> i32           ("loop_countdown": I32IncBrIfNonZero + I32IncCmpImmBrIf)
        //  2: (i32, i32, f32) -> f32 ("f32_load_add": LoadF32Add + F32MulAdd, including OOB & NaN)
        //  3: (i32, i32, f64) -> f64 ("f64_shl3_add": LoadF64Shl3AddAcc, including OOB & NaN)
        //  4: (f32, f32, f32) -> f32 ("f32_fma_leaf": F32MulAdd pure_leaf)
        let mut type_sec = vec![5];
        type_sec.extend_from_slice(&[0x60, 2, 0x7f, 0x7f, 1, 0x7f]); // 0
        type_sec.extend_from_slice(&[0x60, 1, 0x7f, 1, 0x7f]); // 1
        type_sec.extend_from_slice(&[0x60, 3, 0x7f, 0x7f, 0x7d, 1, 0x7d]); // 2
        type_sec.extend_from_slice(&[0x60, 3, 0x7f, 0x7f, 0x7c, 1, 0x7c]); // 3
        type_sec.extend_from_slice(&[0x60, 3, 0x7d, 0x7d, 0x7d, 1, 0x7d]); // 4

        let func_sec = vec![5, 0, 1, 2, 3, 4];
        let mem_sec = vec![1, 0x00, 1]; // 1 page (64 KiB)
        let data_sec = vec![1, 0x00, 0x41, 0x00, 0x0b, 4, 0x10, 0x20, 0x30, 0x40];

        let mut exp_sec = vec![5];
        for (idx, name) in [
            "loop_ne",
            "loop_countdown",
            "f32_load_add",
            "f64_shl3_add",
            "f32_fma_leaf",
        ]
        .iter()
        .enumerate()
        {
            exp_sec.push(name.len() as u8);
            exp_sec.extend_from_slice(name.as_bytes());
            exp_sec.push(0x00);
            exp_sec.push(idx as u8);
        }

        // Body 0 ("loop_ne"): (base: i32, count: i32) -> i32
        // Locals: local 2 = i, local 3 = sum, local 4 = ptr
        let b0 = body(
            &[(3, 0x7f)],
            &[
                // StoreI32Imm (const 0) at base+0 and StoreI32Imm (const 77) at base+4
                0x20, 0x00, 0x41, 0x00, 0x36, 0x02, 0x00, // mem[base+0] = 0
                0x20, 0x00, 0x41, 0xcd, 0x00, 0x36, 0x02, 0x04, // mem[base+4] = 77
                // StoreI32_8Imm at base+8 = 0x5a
                0x20, 0x00, 0x41, 0xda, 0x00, 0x3a, 0x00, 0x08, // mem8[base+8] = 0x5a
                // Loop 1: for (i = 0; i != count; i += 1) { sum += mem32[base + (i << 2)]; }
                // Exercises LoadI32Shl2AddAcc + I32IncNeBrIf!
                0x41, 0x00, 0x21, 0x02, // i = 0
                0x03, 0x40, // loop
                0x20, 0x03, 0x20, 0x00, 0x20, 0x02, 0x41, 0x02, 0x74, 0x6a, 0x28, 0x02, 0x00, 0x6a,
                0x21, 0x03, // sum += mem32[base + (i << 2)]
                0x20, 0x02, 0x41, 0x01, 0x6a, 0x22, 0x02, 0x20, 0x01, 0x47, 0x0d,
                0x00, // i += 1; br_if (i != count)
                0x0b, // end loop
                // Direct LoadI32Add: sum += mem32[base + 4]
                0x20, 0x00, 0x21, 0x04, // ptr = base
                0x20, 0x03, 0x20, 0x04, 0x28, 0x02, 0x04, 0x6a, 0x21,
                0x03, // sum += mem32[ptr + 4]
                0x20, 0x03,
            ],
        );

        // Body 1 ("loop_countdown"): (n: i32) -> i32
        // Locals: local 1 = acc, local 2 = i
        // Loop 1: countdown `n -= 1; br_if $loop` -> I32IncBrIfNonZero!
        // Loop 2: constant-bound `i += 2; i <u 12; br_if $loop` -> I32IncCmpImmBrIf!
        // Loop 3: signed constant-bound `i -= 1; i >=s -3; br_if $loop` -> I32IncCmpImmBrIf (signed)!
        let b1 = body(
            &[(2, 0x7f)],
            &[
                0x03, 0x40, // loop 1
                0x20, 0x01, 0x20, 0x00, 0x6a, 0x21, 0x01, // acc += n
                0x20, 0x00, 0x41, 0x01, 0x6b, 0x22, 0x00, 0x0d, 0x00, // n -= 1; br_if n
                0x0b, // end loop 1
                0x41, 0x00, 0x21, 0x02, // i = 0
                0x03, 0x40, // loop 2
                0x20, 0x01, 0x20, 0x02, 0x6a, 0x21, 0x01, // acc += i
                0x20, 0x02, 0x41, 0x02, 0x6a, 0x22, 0x02, 0x41, 0x0c, 0x49, 0x0d,
                0x00, // i += 2; br_if (i <u 12)
                0x0b, // end loop 2
                0x41, 0x02, 0x21, 0x02, // i = 2
                0x03, 0x40, // loop 3
                0x20, 0x01, 0x20, 0x02, 0x6a, 0x21, 0x01, // acc += i
                0x20, 0x02, 0x41, 0x01, 0x6b, 0x22, 0x02, 0x41, 0x7d, 0x4e, 0x0d,
                0x00, // i -= 1; br_if (i >=s -3)
                0x0b, // end loop 3
                0x20, 0x01,
            ],
        );

        // Body 2 ("f32_load_add"): (addr: i32, store_bits: i32, acc: f32) -> f32
        // Writes store_bits at addr (unless addr is OOB), then `acc = acc + f32.load(addr)` (LoadF32Add)!
        let b2 = body(
            &[],
            &[
                0x20, 0x00, 0x20, 0x01, 0x36, 0x02, 0x00, // i32.store(addr, store_bits)
                0x20, 0x02, 0x20, 0x00, 0x2a, 0x02, 0x00, 0x92, 0x21,
                0x02, // acc += f32.load(addr)
                0x20, 0x02,
            ],
        );

        // Body 3 ("f64_shl3_add"): (base: i32, idx: i32, acc: f64) -> f64
        // Writes 2.5_f64 at base + (idx << 3) (when in bounds), then `acc += f64.load(base + (idx << 3))`!
        let mut f64_code = vec![0x20, 0x00, 0x20, 0x01, 0x41, 0x03, 0x74, 0x6a, 0x44];
        f64_code.extend_from_slice(&2.5_f64.to_bits().to_le_bytes());
        f64_code.extend_from_slice(&[
            0x39, 0x03, 0x00, // f64.store
            0x20, 0x02, 0x20, 0x00, 0x20, 0x01, 0x41, 0x03, 0x74, 0x6a, 0x2b, 0x03, 0x00, 0xa0,
            0x21, 0x02, // acc += f64.load(base + (idx << 3))
            0x20, 0x02,
        ]);
        let b3 = body(&[], &f64_code);

        // Body 4 ("f32_fma_leaf"): (acc: f32, a: f32, b: f32) -> f32 (F32MulAdd + pure_leaf)
        let b4 = body(&[], &[0x20, 0x00, 0x20, 0x01, 0x20, 0x02, 0x94, 0x92]);

        let mut code_sec = vec![5];
        for b in [&b0, &b1, &b2, &b3, &b4] {
            code_sec.extend_from_slice(b);
        }

        let wasm = wasm_mod(&[
            sec(1, &type_sec),
            sec(3, &func_sec),
            sec(5, &mem_sec),
            sec(7, &exp_sec),
            sec(10, &code_sec),
            sec(11, &data_sec),
        ]);

        // 1) Verify every new fused superinstruction is actually emitted!
        let reg_eng =
            Engine::with_execution_policy(Config::default(), ExecutionPolicy::Register).unwrap();
        let reg_mod = reg_eng.compile(&wasm).unwrap();
        let f0 = get_or_compile_ref(&reg_mod, 0).expect("loop_ne admitted");
        assert!(
            f0.ops
                .iter()
                .any(|op| matches!(op, RegOp::StoreI32Imm { .. }))
        );
        assert!(
            f0.ops
                .iter()
                .any(|op| matches!(op, RegOp::StoreI32_8Imm { .. }))
        );
        assert!(
            f0.ops
                .iter()
                .any(|op| matches!(op, RegOp::LoadI32Shl2AddAcc { .. }))
        );
        assert!(
            f0.ops
                .iter()
                .any(|op| matches!(op, RegOp::I32IncNeBrIf { .. }))
        );
        assert!(
            f0.ops
                .iter()
                .any(|op| matches!(op, RegOp::LoadI32Add { .. }))
        );
        let f1 = get_or_compile_ref(&reg_mod, 1).expect("loop_countdown admitted");
        assert!(
            f1.ops
                .iter()
                .any(|op| matches!(op, RegOp::I32IncBrIfNonZero { .. }))
        );
        assert!(
            f1.ops
                .iter()
                .any(|op| matches!(op, RegOp::I32IncCmpImmBrIf { .. }))
        );
        let f2 = get_or_compile_ref(&reg_mod, 2).expect("f32_load_add admitted");
        assert!(
            f2.ops
                .iter()
                .any(|op| matches!(op, RegOp::LoadF32Add { .. }))
        );
        let f3 = get_or_compile_ref(&reg_mod, 3).expect("f64_shl3_add admitted");
        assert!(
            f3.ops
                .iter()
                .any(|op| matches!(op, RegOp::LoadF64Shl3AddAcc { .. }))
        );
        let f4 = get_or_compile_ref(&reg_mod, 4).expect("f32_fma_leaf admitted");
        assert!(f4.pure_leaf);
        assert!(
            f4.ops
                .iter()
                .any(|op| matches!(op, RegOp::F32MulAdd { .. }))
        );

        // 2) Exhaustive fuel-prefix parity across normal + OOB trap + dual-NaN cases!
        for (base, count) in [(16i32, 4i32), (65532i32, 2i32), (65536i32, 1i32)] {
            for fuel in 0u64..=90 {
                let mut pair = Vec::with_capacity(2);
                for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                    let mut c = Config::default();
                    c.limits.fuel = Some(fuel);
                    let eng = Engine::with_execution_policy(c, policy).unwrap();
                    let m = eng.compile(&wasm).unwrap();
                    let mut st = Store::new(&eng);
                    let ins = Linker::new(&eng).instantiate(&mut st, &m).unwrap();
                    let External::Func(f) = ins.export(&st, "loop_ne").unwrap() else {
                        panic!()
                    };
                    let res = st
                        .invoke(f, &[Value::I32(base), Value::I32(count)])
                        .map_err(|e| (e.kind, e.trap_code, e.message));
                    pair.push((res, st.current_fuel));
                }
                assert_eq!(
                    pair[0], pair[1],
                    "loop_ne parity mismatch for base={base}, count={count}, fuel={fuel}"
                );
            }
        }

        // Countdown + constant-bound loops with fuel sweep!
        for fuel in 0u64..=120 {
            let mut pair = Vec::with_capacity(2);
            for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                let mut c = Config::default();
                c.limits.fuel = Some(fuel);
                let eng = Engine::with_execution_policy(c, policy).unwrap();
                let m = eng.compile(&wasm).unwrap();
                let mut st = Store::new(&eng);
                let ins = Linker::new(&eng).instantiate(&mut st, &m).unwrap();
                let External::Func(f) = ins.export(&st, "loop_countdown").unwrap() else {
                    panic!()
                };
                let res = st
                    .invoke(f, &[Value::I32(5)])
                    .map_err(|e| (e.kind, e.trap_code, e.message));
                pair.push((res, st.current_fuel));
            }
            assert_eq!(
                pair[0], pair[1],
                "loop_countdown parity mismatch for fuel={fuel}"
            );
        }

        // LoadF32Add + LoadF64Shl3AddAcc: dual-NaN payload order + OOB trap + fuel sweep!
        for (addr, bits, acc_f32) in [
            (32i32, 1.25_f32.to_bits() as i32, 2.5_f32.to_bits()),
            (32i32, 0x7fc0_0002_u32 as i32, 0x7fc0_0001_u32), // dual-NaN LHS payload
            (65534i32, 0, 1.0_f32.to_bits()),                 // OOB trap
        ] {
            for fuel in 0u64..=20 {
                let mut pair = Vec::with_capacity(2);
                for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                    let mut c = Config::default();
                    c.limits.fuel = Some(fuel);
                    let eng = Engine::with_execution_policy(c, policy).unwrap();
                    let m = eng.compile(&wasm).unwrap();
                    let mut st = Store::new(&eng);
                    let ins = Linker::new(&eng).instantiate(&mut st, &m).unwrap();
                    let External::Func(f) = ins.export(&st, "f32_load_add").unwrap() else {
                        panic!()
                    };
                    let res = st
                        .invoke(
                            f,
                            &[Value::I32(addr), Value::I32(bits), Value::F32(acc_f32)],
                        )
                        .map_err(|e| (e.kind, e.trap_code, e.message));
                    pair.push((res, st.current_fuel));
                }
                assert_eq!(
                    pair[0], pair[1],
                    "f32_load_add parity mismatch for addr={addr}, fuel={fuel}"
                );
            }
        }

        for (base, idx, acc_f64) in [
            (64i32, 3i32, 1.5_f64.to_bits()),
            (64i32, 3i32, 0x7ff8_0000_0000_0009_u64),
            (65536i32, 1i32, 1.0_f64.to_bits()), // OOB trap
        ] {
            for fuel in 0u64..=25 {
                let mut pair = Vec::with_capacity(2);
                for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                    let mut c = Config::default();
                    c.limits.fuel = Some(fuel);
                    let eng = Engine::with_execution_policy(c, policy).unwrap();
                    let m = eng.compile(&wasm).unwrap();
                    let mut st = Store::new(&eng);
                    let ins = Linker::new(&eng).instantiate(&mut st, &m).unwrap();
                    let External::Func(f) = ins.export(&st, "f64_shl3_add").unwrap() else {
                        panic!()
                    };
                    let res = st
                        .invoke(f, &[Value::I32(base), Value::I32(idx), Value::F64(acc_f64)])
                        .map_err(|e| (e.kind, e.trap_code, e.message));
                    pair.push((res, st.current_fuel));
                }
                assert_eq!(
                    pair[0], pair[1],
                    "f64_shl3_add parity mismatch for base={base}, idx={idx}, fuel={fuel}"
                );
            }
        }

        // F32MulAdd two-rounding IEEE-754 + dual-NaN parity on pure_leaf!
        let f32_p = f32::from_bits(1.0_f32.to_bits() + 1).to_bits();
        let f32_m = f32::from_bits(1.0_f32.to_bits() - 1).to_bits();
        for (acc, a, b) in [
            ((-1.0_f32).to_bits(), f32_p, f32_m),
            (0x7fc0_0001, 0x7fc0_0002, 1.0_f32.to_bits()),
            (
                (-0.0_f32).to_bits(),
                (-0.0_f32).to_bits(),
                1.0_f32.to_bits(),
            ),
        ] {
            let mut pair = Vec::with_capacity(2);
            for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                let eng = Engine::with_execution_policy(Config::default(), policy).unwrap();
                let m = eng.compile(&wasm).unwrap();
                let mut st = Store::new(&eng);
                let ins = Linker::new(&eng).instantiate(&mut st, &m).unwrap();
                let External::Func(f) = ins.export(&st, "f32_fma_leaf").unwrap() else {
                    panic!()
                };
                pair.push(
                    st.invoke(f, &[Value::F32(acc), Value::F32(a), Value::F32(b)])
                        .unwrap(),
                );
            }
            assert_eq!(pair[0], pair[1], "f32_fma_leaf parity mismatch");
        }
    }

    #[test]
    fn fast_register_dense_mem0_hoisting_across_grow_and_cross_instance_parity() {
        // Module B: own 1-page dense mem0 (active data "BBBB" at 0).
        // Exports:
        //  - "mem_b": memory 0
        //  - "grow_and_rw": (grow_pages: i32, v: i32) -> i32
        //      Reads i32 at 0, grows mem0 by `grow_pages`, writes `v` as i32 + v128 at 65536 (new page!),
        //      reads back both, and returns sum.
        //  - "tail_b": (x: i32) -> i32
        //      Reads i32 at 0 of B's mem0, adds `x`, returns.
        let mut b_type = vec![2];
        b_type.extend_from_slice(&[0x60, 2, 0x7f, 0x7f, 1, 0x7f]); // 0
        b_type.extend_from_slice(&[0x60, 1, 0x7f, 1, 0x7f]); // 1
        let b_func = vec![2, 0, 1];
        let b_mem = vec![1, 0x00, 1]; // 1 page
        let b_data = vec![1, 0x00, 0x41, 0x00, 0x0b, 4, 0x02, 0x00, 0x00, 0x00]; // mem_b[0] = 2
        let mut b_exp = vec![3];
        for (name, kind, idx) in [
            ("mem_b", 0x02u8, 0u8),
            ("grow_and_rw", 0x00, 0),
            ("tail_b", 0x00, 1),
        ] {
            b_exp.push(name.len() as u8);
            b_exp.extend_from_slice(name.as_bytes());
            b_exp.push(kind);
            b_exp.push(idx);
        }
        let mut b_body0_code = Vec::new();
        // local 2 = mem_b[0]
        b_body0_code.extend_from_slice(&[0x41, 0x00, 0x28, 0x02, 0x00, 0x21, 0x02]);
        // old_pages = memory.grow(grow_pages); drop
        b_body0_code.extend_from_slice(&[0x20, 0x00, 0x40, 0x00, 0x1a]);
        // mem_b[65536] = v (i32.store at new page!)
        b_body0_code.extend_from_slice(&[0x41, 0x80, 0x80, 0x04, 0x20, 0x01, 0x36, 0x02, 0x00]);
        // mem_b[65552] = i32x4.splat(v + 1) (v128.store at new page!)
        b_body0_code.extend_from_slice(&[0x41, 0x90, 0x80, 0x04, 0x20, 0x01, 0x41, 0x01, 0x6a]);
        simd_op(&mut b_body0_code, 17); // i32x4.splat
        b_body0_code.extend_from_slice(&[0xfd, 0x0b, 0x00, 0x00]); // v128.store
        // return local 2 + mem_b[65536] + i32x4.extract_lane 3 (v128.load 65552)
        b_body0_code.extend_from_slice(&[0x20, 0x02]);
        b_body0_code.extend_from_slice(&[0x41, 0x80, 0x80, 0x04, 0x28, 0x02, 0x00, 0x6a]);
        b_body0_code.extend_from_slice(&[0x41, 0x90, 0x80, 0x04, 0xfd, 0x00, 0x00, 0x00]);
        simd_op(&mut b_body0_code, 27); // i32x4.extract_lane
        b_body0_code.push(3);
        b_body0_code.push(0x6a);
        let b_body0 = body(&[(1, 0x7f)], &b_body0_code);
        let b_body1 = body(&[], &[0x41, 0x00, 0x28, 0x02, 0x00, 0x20, 0x00, 0x6a]);
        let mut b_code = vec![2];
        b_code.extend_from_slice(&b_body0);
        b_code.extend_from_slice(&b_body1);
        let wasm_b = wasm_mod(&[
            sec(1, &b_type),
            sec(3, &b_func),
            sec(5, &b_mem),
            sec(7, &b_exp),
            sec(10, &b_code),
            sec(11, &b_data),
        ]);

        // Module A: own 1-page dense mem0 (active data: mem_a[0] = 100).
        // Imports: b.grow_and_rw (func 0), b.tail_b (func 1).
        // Exports: "mem_a", "run_a": (grow_a: i32, grow_b: i32, v: i32) -> i32, "tail_to_b": (x: i32) -> i32
        let mut a_type = vec![3];
        a_type.extend_from_slice(&[0x60, 2, 0x7f, 0x7f, 1, 0x7f]); // 0: (i32, i32) -> i32
        a_type.extend_from_slice(&[0x60, 1, 0x7f, 1, 0x7f]); // 1: (i32) -> i32
        a_type.extend_from_slice(&[0x60, 3, 0x7f, 0x7f, 0x7f, 1, 0x7f]); // 2: (i32, i32, i32) -> i32
        let mut a_imp = vec![2];
        a_imp.extend_from_slice(&[
            1, b'b', 11, b'g', b'r', b'o', b'w', b'_', b'a', b'n', b'd', b'_', b'r', b'w', 0x00, 0,
        ]);
        a_imp.extend_from_slice(&[1, b'b', 6, b't', b'a', b'i', b'l', b'_', b'b', 0x00, 1]);
        let a_func = vec![2, 2, 1];
        let a_mem = vec![1, 0x00, 1];
        let a_data = vec![1, 0x00, 0x41, 0x00, 0x0b, 4, 100, 0, 0, 0]; // mem_a[0] = 100
        let mut a_exp = vec![3];
        for (name, kind, idx) in [
            ("mem_a", 0x02u8, 0u8),
            ("run_a", 0x00, 2),
            ("tail_to_b", 0x00, 3),
        ] {
            a_exp.push(name.len() as u8);
            a_exp.extend_from_slice(name.as_bytes());
            a_exp.push(kind);
            a_exp.push(idx);
        }
        // run_a(grow_a, grow_b, v):
        //  1) write mem_a[16] = v * 2
        //  2) r_b = call b.grow_and_rw(grow_b, v) (cross-instance call + grow in B + SwitchInst back to A!)
        //  3) verify mem_a[0] is still 100 and mem_a[16] is still v * 2!
        //  4) grow mem_a by `grow_a` pages; write i32(v * 3) at mem_a[65536] and read back!
        //  5) return mem_a[0] + mem_a[16] + r_b + mem_a[65536]
        let a_body0 = body(
            &[(1, 0x7f)],
            &[
                // mem_a[16] = v * 2
                0x41, 0x10, 0x20, 0x02, 0x41, 0x02, 0x6c, 0x36, 0x02, 0x00,
                // local 3 = call b.grow_and_rw(grow_b, v)
                0x20, 0x01, 0x20, 0x02, 0x10, 0x00, 0x21, 0x03,
                // memory.grow(grow_a); drop
                0x20, 0x00, 0x40, 0x00, 0x1a, // mem_a[65536] = v * 3
                0x41, 0x80, 0x80, 0x04, 0x20, 0x02, 0x41, 0x03, 0x6c, 0x36, 0x02, 0x00,
                // return mem_a[0] + mem_a[16] + local 3 + mem_a[65536]
                0x41, 0x00, 0x28, 0x02, 0x00, 0x41, 0x10, 0x28, 0x02, 0x00, 0x6a, 0x20, 0x03, 0x6a,
                0x41, 0x80, 0x80, 0x04, 0x28, 0x02, 0x00, 0x6a,
            ],
        );
        // tail_to_b(x): reads mem_a[0], adds x, then return_call b.tail_b!
        let a_body1 = body(
            &[],
            &[0x41, 0x00, 0x28, 0x02, 0x00, 0x20, 0x00, 0x6a, 0x12, 0x01],
        );
        let mut a_code = vec![2];
        a_code.extend_from_slice(&a_body0);
        a_code.extend_from_slice(&a_body1);
        let wasm_a = wasm_mod(&[
            sec(1, &a_type),
            sec(2, &a_imp),
            sec(3, &a_func),
            sec(5, &a_mem),
            sec(7, &a_exp),
            sec(10, &a_code),
            sec(11, &a_data),
        ]);

        // 1) Cross-instance grow + dense slice refresh + cross-instance tail call across fuel sweep!
        for fuel in [
            None,
            Some(0),
            Some(1),
            Some(5),
            Some(15),
            Some(25),
            Some(40),
            Some(80),
        ] {
            let mut pair = Vec::with_capacity(2);
            for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                let mut cfg = Config::default();
                cfg.features.simd = true;
                cfg.features.tail_call = true;
                cfg.limits.fuel = fuel;
                let eng = Engine::with_execution_policy(cfg, policy).unwrap();
                let mb = eng.compile(&wasm_b).unwrap();
                let ma = eng.compile(&wasm_a).unwrap();
                let mut st = Store::new(&eng);
                let ins_b = Linker::new(&eng).instantiate(&mut st, &mb).unwrap();
                let mut lnk = Linker::new(&eng);
                lnk.define(
                    "b",
                    "grow_and_rw",
                    ins_b.export(&st, "grow_and_rw").unwrap(),
                )
                .unwrap();
                lnk.define("b", "tail_b", ins_b.export(&st, "tail_b").unwrap())
                    .unwrap();
                let ins_a = lnk.instantiate(&mut st, &ma).unwrap();
                let External::Func(f_run_a) = ins_a.export(&st, "run_a").unwrap() else {
                    panic!()
                };
                let External::Func(f_tail) = ins_a.export(&st, "tail_to_b").unwrap() else {
                    panic!()
                };
                let r1 = st
                    .invoke(f_run_a, &[Value::I32(2), Value::I32(1), Value::I32(10)])
                    .map_err(|e| (e.kind, e.trap_code, e.message));
                let f1 = st.current_fuel;
                if fuel.is_none() && policy == ExecutionPolicy::Register {
                    let External::Memory(ma_h) = ins_a.export(&st, "mem_a").unwrap() else {
                        panic!()
                    };
                    let External::Memory(mb_h) = ins_b.export(&st, "mem_b").unwrap() else {
                        panic!()
                    };
                    assert!(
                        st.mems[ma_h.0.index].bytes.is_dense(),
                        "mem_a must stay dense after growing to 3 pages"
                    );
                    assert!(
                        st.mems[mb_h.0.index].bytes.is_dense(),
                        "mem_b must stay dense after growing to 2 pages"
                    );
                    assert_eq!(r1, Ok(RunOutcome::Returned(vec![Value::I32(173)])));
                }
                let r2 = st
                    .invoke(f_tail, &[Value::I32(7)])
                    .map_err(|e| (e.kind, e.trap_code, e.message));
                let f2 = st.current_fuel;
                if fuel.is_none() {
                    // mem_a[0] (100) + 7 = 107 -> tail_b adds mem_b[0] (2) -> 109!
                    assert_eq!(r2, Ok(RunOutcome::Returned(vec![Value::I32(109)])));
                }
                pair.push((r1, f1, r2, f2));
            }
            assert_eq!(
                pair[0], pair[1],
                "cross-instance dense grow parity mismatch for fuel={fuel:?}"
            );
        }

        // 2) Grow dense mem0 past DENSE_MAX_BYTES mid-function (demotion from dense -> sparse)!
        let demote_pages = (crate::linear::DENSE_MAX_BYTES / 65536) as i32 + 2;
        let mut pair = Vec::with_capacity(2);
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
            let mut cfg = Config::default();
            cfg.features.simd = true;
            cfg.features.tail_call = true;
            cfg.limits.max_memory_pages = (demote_pages + 8) as u64;
            cfg.limits.max_memory_bytes = ((demote_pages + 8) as u64) * 65536;
            cfg.limits.max_store_memory_bytes = cfg.limits.max_memory_bytes * 2;
            let eng = Engine::with_execution_policy(cfg, policy).unwrap();
            let mb = eng.compile(&wasm_b).unwrap();
            let ma = eng.compile(&wasm_a).unwrap();
            let mut st = Store::new(&eng);
            let ins_b = Linker::new(&eng).instantiate(&mut st, &mb).unwrap();
            let mut lnk = Linker::new(&eng);
            lnk.define(
                "b",
                "grow_and_rw",
                ins_b.export(&st, "grow_and_rw").unwrap(),
            )
            .unwrap();
            lnk.define("b", "tail_b", ins_b.export(&st, "tail_b").unwrap())
                .unwrap();
            let ins_a = lnk.instantiate(&mut st, &ma).unwrap();
            let External::Func(f_run_a) = ins_a.export(&st, "run_a").unwrap() else {
                panic!()
            };
            let res = st
                .invoke(
                    f_run_a,
                    &[Value::I32(demote_pages), Value::I32(1), Value::I32(11)],
                )
                .unwrap();
            if policy == ExecutionPolicy::Register {
                let External::Memory(ma_h) = ins_a.export(&st, "mem_a").unwrap() else {
                    panic!()
                };
                assert!(
                    !st.mems[ma_h.0.index].bytes.is_dense(),
                    "mem_a must demote cleanly to sparse when grown past DENSE_MAX_BYTES"
                );
            }
            pair.push(res);
        }
        assert_eq!(pair[0], pair[1]);
    }

    #[test]
    fn register_sparse_empty_mem_zero_len_bulk_oob_traps() {
        // `(memory 0)` stays sparse (`p0_slice`/`p1_slice` empty, `p1_base = 65536`);
        // zero-length `memory.fill(65536, 0, 0)` and `memory.copy(65536, 65536, 0)` MUST trap OOB!
        let fill_body = body(
            &[],
            &[0x41, 0x80, 0x80, 0x04, 0x41, 0, 0x41, 0, 0xfc, 11, 0],
        );
        let copy_body = body(
            &[],
            &[
                0x41, 0x80, 0x80, 0x04, 0x41, 0x80, 0x80, 0x04, 0x41, 0, 0xfc, 10, 0, 0,
            ],
        );
        let mut code = vec![2];
        code.extend_from_slice(&fill_body);
        code.extend_from_slice(&copy_body);

        let wasm = wasm_mod(&[
            sec(1, &[1, 0x60, 0, 0]),
            sec(3, &[2, 0, 0]),
            sec(5, &[1, 0x00, 0]),
            sec(
                7,
                &[
                    2, 4, b'f', b'i', b'l', b'l', 0, 0, 4, b'c', b'o', b'p', b'y', 0, 1,
                ],
            ),
            sec(10, &code),
        ]);
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
            let eng = Engine::with_execution_policy(Config::default(), policy).unwrap();
            let module = eng.compile(&wasm).unwrap();
            let mut st = Store::new(&eng);
            let ins = Linker::new(&eng).instantiate(&mut st, &module).unwrap();
            for name in ["fill", "copy"] {
                let External::Func(f) = ins.export(&st, name).unwrap() else {
                    panic!()
                };
                let err = st.invoke(f, &[]).expect_err("expected OOB trap");
                assert_eq!(
                    err.trap_code,
                    Some(wtime_types::TrapCode::MemoryOutOfBounds),
                    "{name} policy={policy:?} got {err:?}"
                );
            }
        }
    }

    #[test]
    fn fast_register_unboxed_w3_relaxed_fma_and_hot_simd_parity() {
        fn push_sleb(out: &mut Vec<u8>, mut v: i32) {
            loop {
                let mut byte = (v & 0x7f) as u8;
                v >>= 7;
                let done = (v == 0 && (byte & 0x40) == 0) || (v == -1 && (byte & 0x40) != 0);
                if !done {
                    byte |= 0x80;
                }
                out.push(byte);
                if done {
                    break;
                }
            }
        }
        let emit_op = |code: &mut Vec<u8>, op: u16| {
            let sub = if op >= 0xfd00 {
                u32::from(op - 0xfd00)
            } else {
                u32::from(op - 0xfa00 + 0x100)
            };
            simd_op(code, sub);
        };
        let emit_case =
            |code: &mut Vec<u8>, mode_local: u8, mode: i32, f: &dyn Fn(&mut Vec<u8>)| {
                code.extend_from_slice(&[0x20, mode_local, 0x41]);
                push_sleb(code, mode);
                code.extend_from_slice(&[0x46, 0x04, 0x40]); // if local[mode_local] == mode
                f(code);
                code.extend_from_slice(&[0x0f, 0x0b]); // return; end
            };

        // Types:
        //  0: (v128, v128, v128, i32) -> v128 ("w3_fma": relaxed FMA + F64x2MulAdd + aliasing + guards)
        //  1: (v128, v128, i32, i32) -> v128  ("hot_simd": f32/f64/i8/i16/i32/i64 hot unboxed ops)
        //  2: (v128, i32) -> i32              ("masks_extract": any_true, bitmask, extract_lane)
        //  3: (i32, v128, v128, v128) -> v128 ("fma_loop": counted FMA+hot SIMD loop for fuel parity)
        let mut type_sec = vec![4];
        type_sec.extend_from_slice(&[0x60, 4, 0x7b, 0x7b, 0x7b, 0x7f, 1, 0x7b]); // 0
        type_sec.extend_from_slice(&[0x60, 4, 0x7b, 0x7b, 0x7f, 0x7f, 1, 0x7b]); // 1
        type_sec.extend_from_slice(&[0x60, 2, 0x7b, 0x7f, 1, 0x7f]); // 2
        type_sec.extend_from_slice(&[0x60, 4, 0x7f, 0x7b, 0x7b, 0x7b, 1, 0x7b]); // 3

        let func_sec = vec![4, 0, 1, 2, 3];
        let mut exp_sec = vec![4];
        for (idx, name) in ["w3_fma", "hot_simd", "masks_extract", "fma_loop"]
            .iter()
            .enumerate()
        {
            exp_sec.push(name.len() as u8);
            exp_sec.extend_from_slice(name.as_bytes());
            exp_sec.push(0x00);
            exp_sec.push(idx as u8);
        }

        // Body 0 ("w3_fma"): (a: v128, b: v128, c: v128, mode: i32) -> v128, local 4: v128
        let mut c0 = Vec::new();
        // Modes 0..=3: 0xfa05..=0xfa08 (f32x4/f64x2 relaxed madd/nmadd)
        for (m, op) in [(0, 0xfa05u16), (1, 0xfa06), (2, 0xfa07), (3, 0xfa08)] {
            emit_case(&mut c0, 3, m, &|c| {
                c.extend_from_slice(&[0x20, 0, 0x20, 1, 0x20, 2]);
                emit_op(c, op);
            });
        }
        // Modes 4..=7: Register aliasing dst == a / dst == b / dst == c / dst == a == b == c!
        emit_case(&mut c0, 3, 4, &|c| {
            c.extend_from_slice(&[0x20, 0, 0x20, 1, 0x20, 2]);
            emit_op(c, 0xfa05);
            c.extend_from_slice(&[0x22, 0]); // local.tee 0 (dst == a)
        });
        emit_case(&mut c0, 3, 5, &|c| {
            c.extend_from_slice(&[0x20, 0, 0x20, 1, 0x20, 2]);
            emit_op(c, 0xfa06);
            c.extend_from_slice(&[0x22, 1]); // local.tee 1 (dst == b)
        });
        emit_case(&mut c0, 3, 6, &|c| {
            c.extend_from_slice(&[0x20, 0, 0x20, 1, 0x20, 2]);
            emit_op(c, 0xfa07);
            c.extend_from_slice(&[0x22, 2]); // local.tee 2 (dst == c)
        });
        emit_case(&mut c0, 3, 7, &|c| {
            c.extend_from_slice(&[0x20, 0, 0x20, 0, 0x20, 0]);
            emit_op(c, 0xfa08);
            c.extend_from_slice(&[0x22, 0]); // local.tee 0 (dst == a == b == c)
        });
        // Mode 8: fused two-rounding F64x2MulAdd: c + (a * b)
        emit_case(&mut c0, 3, 8, &|c| {
            c.extend_from_slice(&[0x20, 2, 0x20, 0, 0x20, 1]);
            emit_op(c, 0xfdf2); // f64x2.mul
            emit_op(c, 0xfdf0); // f64x2.add -> F64x2MulAdd
        });
        // Mode 9: F64x2MulAdd with aliasing dst == a (`local.set 0`)
        emit_case(&mut c0, 3, 9, &|c| {
            c.extend_from_slice(&[0x20, 2, 0x20, 0, 0x20, 1]);
            emit_op(c, 0xfdf2);
            emit_op(c, 0xfdf0);
            c.extend_from_slice(&[0x22, 0]);
        });
        // Mode 10: `f64x2.mul; local.tee 4; f64x2.add` (must NOT fuse away local 4 write!)
        emit_case(&mut c0, 3, 10, &|c| {
            c.extend_from_slice(&[0x20, 2, 0x20, 0, 0x20, 1]);
            emit_op(c, 0xfdf2);
            c.extend_from_slice(&[0x22, 4]); // local.tee 4
            emit_op(c, 0xfdf0);
            c.extend_from_slice(&[0x20, 4]);
            emit_op(c, 0xfd51); // xor with local 4 so clobbering local 4 would fail
        });
        // Mode 11: `f64x2.mul` across block boundary before `f64x2.add` (block-boundary guard!)
        emit_case(&mut c0, 3, 11, &|c| {
            c.extend_from_slice(&[0x20, 2, 0x02, 0x7b, 0x20, 0, 0x20, 1]);
            emit_op(c, 0xfdf2);
            c.push(0x0b); // end block
            emit_op(c, 0xfdf0);
        });
        c0.extend_from_slice(&[0x20, 0]);
        let b0 = body(&[(1, 0x7b)], &c0);

        // Body 1 ("hot_simd"): (a: v128, b: v128, sh: i32, mode: i32) -> v128
        let mut c1 = Vec::new();
        let mut mode = 0i32;
        // Unary v128 -> v128 ops:
        for op in [
            0xfde3u16, // f32x4.sqrt
            0xfdec,    // f64x2.abs
            0xfded,    // f64x2.neg
            0xfdef,    // f64x2.sqrt
            0xfdfa,    // f32x4.convert_i32x4_s
            0xfdfb,    // f32x4.convert_i32x4_u
        ] {
            emit_case(&mut c1, 3, mode, &|c| {
                c.extend_from_slice(&[0x20, 0]);
                emit_op(c, op);
                c.extend_from_slice(&[0x22, 0]); // also exercises dst == src aliasing!
            });
            mode += 1;
        }
        // Splat8 & Splat16 from scalar `sh` (local 2):
        for op in [0xfd0fu16, 0xfd10] {
            emit_case(&mut c1, 3, mode, &|c| {
                c.extend_from_slice(&[0x20, 2]);
                emit_op(c, op);
            });
            mode += 1;
        }
        // Binary v128 x v128 -> v128 ops:
        let bin_ops: &[u16] = &[
            0xfdf4, 0xfa0f, 0xfdf5, 0xfa10, 0xfdf6, 0xfdf7, // f64x2 min/max/pmin/pmax
            0xfd47, 0xfd48, 0xfd49, 0xfd4a, 0xfd4b, 0xfd4c, // f64x2 cmp
            0xfd6e, 0xfd71, // i8x16 add/sub
            0xfd8e, 0xfd91, 0xfd95, // i16x8 add/sub/mul
            0xfdd5, // i64x2 mul
            0xfd37, 0xfd38, 0xfd39, 0xfd3a, 0xfd3b, 0xfd3c, 0xfd3d, 0xfd3e, 0xfd3f,
            0xfd40, // i32x4 cmp (all 10)
        ];
        for &op in bin_ops {
            emit_case(&mut c1, 3, mode, &|c| {
                c.extend_from_slice(&[0x20, 0, 0x20, 1]);
                emit_op(c, op);
                c.extend_from_slice(&[0x22, 0]); // exercises dst == a aliasing!
            });
            mode += 1;
        }
        // Binary v128 x i32 -> v128 shifts (i32x4.{shl, shr_s, shr_u}):
        for op in [0xfdabu16, 0xfdac, 0xfdad] {
            emit_case(&mut c1, 3, mode, &|c| {
                c.extend_from_slice(&[0x20, 0, 0x20, 2]);
                emit_op(c, op);
                c.extend_from_slice(&[0x22, 0]); // exercises dst == a aliasing!
            });
            mode += 1;
        }
        let hot_simd_modes = mode;
        c1.extend_from_slice(&[0x20, 0]);
        let b1 = body(&[], &c1);

        // Body 2 ("masks_extract"): (v: v128, mode: i32) -> i32
        let mut c2 = Vec::new();
        // Modes 0..=2: v128.any_true, i8x16.bitmask, i32x4.bitmask
        for (m, op) in [(0, 0xfd53u16), (1, 0xfd64), (2, 0xfda4)] {
            emit_case(&mut c2, 1, m, &|c| {
                c.extend_from_slice(&[0x20, 0]);
                emit_op(c, op);
            });
        }
        // Modes 3..=6: i32x4.extract_lane 0..=3
        for lane in 0u8..4 {
            emit_case(&mut c2, 1, 3 + i32::from(lane), &|c| {
                c.extend_from_slice(&[0x20, 0]);
                emit_op(c, 0xfd1b);
                c.push(lane);
            });
        }
        // Modes 7..=10: f32x4.extract_lane 0..=3 + i32.reinterpret_f32 (exact bit equality!)
        for lane in 0u8..4 {
            emit_case(&mut c2, 1, 7 + i32::from(lane), &|c| {
                c.extend_from_slice(&[0x20, 0]);
                emit_op(c, 0xfd1f);
                c.push(lane);
                c.push(0xbc); // i32.reinterpret_f32
            });
        }
        c2.extend_from_slice(&[0x41, 0x00]);
        let b2 = body(&[], &c2);

        // Body 3 ("fma_loop"): (n: i32, acc: v128, a: v128, b: v128) -> v128
        // Counted loop exercising relaxed FMA + F64x2MulAdd + cmp/bitmask/any_true/extract under fuel!
        let mut c3 = Vec::new();
        c3.extend_from_slice(&[0x03, 0x40]); // loop
        c3.extend_from_slice(&[0x20, 0, 0x45, 0x04, 0x40, 0x20, 1, 0x0f, 0x0b]); // if n == 0 return acc
        // acc = f32x4.relaxed_madd(a, b, acc)
        c3.extend_from_slice(&[0x20, 2, 0x20, 3, 0x20, 1]);
        emit_op(&mut c3, 0xfa05);
        c3.extend_from_slice(&[0x21, 1]);
        // acc = f64x2.relaxed_nmadd(acc, a, b)
        c3.extend_from_slice(&[0x20, 1, 0x20, 2, 0x20, 3]);
        emit_op(&mut c3, 0xfa08);
        c3.extend_from_slice(&[0x21, 1]);
        // acc = acc + (a * b) (f64x2 -> F64x2MulAdd)
        c3.extend_from_slice(&[0x20, 1, 0x20, 2, 0x20, 3]);
        emit_op(&mut c3, 0xfdf2);
        emit_op(&mut c3, 0xfdf0);
        c3.extend_from_slice(&[0x21, 1]);
        // mix in i32x4.cmp + i32x4.bitmask + v128.any_true + i32x4.extract_lane + i32x4.shl
        c3.extend_from_slice(&[0x20, 1, 0x20, 2]);
        emit_op(&mut c3, 0xfd39); // i32x4.lt_s
        emit_op(&mut c3, 0xfda4); // i32x4.bitmask
        c3.extend_from_slice(&[0x20, 1]);
        emit_op(&mut c3, 0xfd53); // v128.any_true
        c3.push(0x6a); // i32.add
        c3.extend_from_slice(&[0x20, 1]);
        emit_op(&mut c3, 0xfd1b);
        c3.push(2); // i32x4.extract_lane 2
        c3.push(0x73); // i32.xor
        emit_op(&mut c3, 0xfd0f); // i8x16.splat
        c3.extend_from_slice(&[0x20, 0]);
        emit_op(&mut c3, 0xfdab); // i32x4.shl
        c3.extend_from_slice(&[0x20, 1]);
        emit_op(&mut c3, 0xfd51); // v128.xor
        c3.extend_from_slice(&[0x21, 1]);
        // n -= 1; br 0
        c3.extend_from_slice(&[0x20, 0, 0x41, 1, 0x6b, 0x21, 0, 0x0c, 0x00, 0x0b]);
        c3.extend_from_slice(&[0x20, 1]);
        let b3 = body(&[], &c3);

        let mut code_sec = vec![4];
        for b in [&b0, &b1, &b2, &b3] {
            code_sec.extend_from_slice(b);
        }
        let wasm = wasm_mod(&[
            sec(1, &type_sec),
            sec(3, &func_sec),
            sec(7, &exp_sec),
            sec(10, &code_sec),
        ]);

        let mut cfg = Config::default();
        cfg.features.simd = true;
        cfg.features.relaxed_simd = true;

        // 1) Verify EVERY SINGLE ONE of the 30 new RegOp variants is emitted and NO boxing fallback!
        let reg_eng =
            Engine::with_execution_policy(cfg.clone(), ExecutionPolicy::Register).unwrap();
        let reg_mod = reg_eng.compile(&wasm).unwrap();
        let funcs: Vec<_> = (0..4)
            .map(|i| get_or_compile_ref(&reg_mod, i).expect("body admitted"))
            .collect();
        for (i, f) in funcs.iter().enumerate() {
            assert!(
                f.ops.iter().all(|op| !matches!(
                    op,
                    RegOp::SimdUnary { .. } | RegOp::SimdBinary { .. } | RegOp::SimdTernary { .. }
                )),
                "body {i} must not emit boxed SimdUnary/SimdBinary/SimdTernary"
            );
        }
        let all_ops: Vec<RegOp> = funcs.iter().flat_map(|f| f.ops.iter().copied()).collect();
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::F32x4RelaxedMadd { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::F32x4RelaxedNmadd { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::F64x2RelaxedMadd { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::F64x2RelaxedNmadd { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::F32x4Sqrt { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::F64x2Abs { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::F64x2Neg { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::F64x2Sqrt { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::F64x2Min { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::F64x2Max { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::F64x2PMin { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::F64x2PMax { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::F64x2Cmp { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::F64x2MulAdd { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::F32x4ConvertI32x4S { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::F32x4ConvertI32x4U { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::V128Splat8 { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::V128Splat16 { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::I8x16Add { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::I8x16Sub { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::I16x8Add { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::I16x8Sub { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::I16x8Mul { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::I64x2Mul { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::I32x4Cmp { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::I32x4Shift { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::V128AnyTrue { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::I8x16Bitmask { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::I32x4Bitmask { .. }))
        );
        assert!(
            all_ops
                .iter()
                .any(|op| matches!(op, RegOp::SimdExtract32 { .. }))
        );

        // Instantiate both Decoded and Register once for fast parity loops.
        let dec_eng = Engine::with_execution_policy(cfg.clone(), ExecutionPolicy::Decoded).unwrap();
        let dec_mod = dec_eng.compile(&wasm).unwrap();
        let mut st_dec = Store::new(&dec_eng);
        let ins_dec = Linker::new(&dec_eng)
            .instantiate(&mut st_dec, &dec_mod)
            .unwrap();
        let mut st_reg = Store::new(&reg_eng);
        let ins_reg = Linker::new(&reg_eng)
            .instantiate(&mut st_reg, &reg_mod)
            .unwrap();

        let invoke_pair = |st_d: &mut Store,
                           st_r: &mut Store,
                           name: &str,
                           args: &[Value]|
         -> (RunOutcome, RunOutcome) {
            let External::Func(fd) = ins_dec.export(st_d, name).unwrap() else {
                panic!()
            };
            let External::Func(fr) = ins_reg.export(st_r, name).unwrap() else {
                panic!()
            };
            (
                st_d.invoke(fd, args).unwrap(),
                st_r.invoke(fr, args).unwrap(),
            )
        };

        // 2) Relaxed FMA (`0xfa05..=0xfa08`) + F64x2MulAdd + aliasing + NaN / -0.0 / single-vs-two-rounding parity!
        let f32_1p = 1.0_f32.to_bits() + 1;
        let f32_1m = 1.0_f32.to_bits() - 1;
        let f64_1p = 1.0_f64.to_bits() + 1;
        let f64_1m = 1.0_f64.to_bits() - 1;
        let fma_triples: [([u8; 16], [u8; 16], [u8; 16]); 4] = [
            // Single-rounding vs two-rounding f32 discriminator + -0.0 + subnormal + finite
            (
                v128_from_f32s([
                    f32_1p,
                    (-0.0_f32).to_bits(),
                    0x0000_0001,
                    (-3.25_f32).to_bits(),
                ]),
                v128_from_f32s([
                    f32_1m,
                    0.0_f32.to_bits(),
                    1.5_f32.to_bits(),
                    2.5_f32.to_bits(),
                ]),
                v128_from_f32s([
                    (-1.0_f32).to_bits(),
                    (-0.0_f32).to_bits(),
                    0.0_f32.to_bits(),
                    7.75_f32.to_bits(),
                ]),
            ),
            // f32 sNaN / +qNaN / -qNaN / ±inf
            (
                v128_from_f32s([
                    0x7f80_0001,
                    0xffc0_1234,
                    f32::INFINITY.to_bits(),
                    (-0.0_f32).to_bits(),
                ]),
                v128_from_f32s([
                    0x7fc0_5678,
                    2.0_f32.to_bits(),
                    0.0_f32.to_bits(),
                    f32::NEG_INFINITY.to_bits(),
                ]),
                v128_from_f32s([
                    1.0_f32.to_bits(),
                    0x7fc0_9999,
                    (-1.0_f32).to_bits(),
                    0.0_f32.to_bits(),
                ]),
            ),
            // Single-rounding vs two-rounding f64 discriminator + -0.0
            (
                v128_from_f64s([f64_1p, (-0.0_f64).to_bits()]),
                v128_from_f64s([f64_1m, 0.0_f64.to_bits()]),
                v128_from_f64s([(-1.0_f64).to_bits(), (-0.0_f64).to_bits()]),
            ),
            // f64 sNaN / -qNaN / dual-NaN payload ordering
            (
                v128_from_f64s([0x7ff0_0000_0000_0001, 0xfff8_0000_0000_1234]),
                v128_from_f64s([0x7ff8_0000_0000_5678, 3.5_f64.to_bits()]),
                v128_from_f64s([0x7ff8_0000_0000_0009, f64::INFINITY.to_bits()]),
            ),
        ];
        for (va, vb, vc) in fma_triples {
            for m in 0..=11 {
                let (d, r) = invoke_pair(
                    &mut st_dec,
                    &mut st_reg,
                    "w3_fma",
                    &[
                        Value::V128(V128(va)),
                        Value::V128(V128(vb)),
                        Value::V128(V128(vc)),
                        Value::I32(m),
                    ],
                );
                assert_eq!(d, r, "w3_fma bit-exact parity mismatch for mode={m}");
            }
        }

        // 3) Hot f64x2 / f32x4 / i8x16 / i16x8 / i32x4 / i64x2 ops parity!
        let pairs: [([u8; 16], [u8; 16]); 5] = [
            (
                v128_from_f64s([0x7ff0_0000_0000_0001, (-0.0_f64).to_bits()]),
                v128_from_f64s([0xfff8_0000_0000_1234, 0.0_f64.to_bits()]),
            ),
            (
                v128_from_f64s([(-4.0_f64).to_bits(), 9.0_f64.to_bits()]),
                v128_from_f64s([f64::NEG_INFINITY.to_bits(), f64::INFINITY.to_bits()]),
            ),
            (
                v128_from_f32s([
                    (-0.0_f32).to_bits(),
                    (-1.0_f32).to_bits(),
                    0x7f80_0003,
                    16.0_f32.to_bits(),
                ]),
                v128_from_f32s([0x8000_0000, 0x7fff_ffff, 0xffff_ffff, 0x0000_0000]),
            ),
            // Byte/word wrapping boundaries (0xff+1, 0x00-1, 0x7f+1, 0x80-1, 0xffff*0xffff)
            (
                [
                    0xff, 0x00, 0x7f, 0x80, 0x01, 0xfe, 0xaa, 0x55, 0xff, 0xff, 0x00, 0x00, 0xff,
                    0x7f, 0x00, 0x80,
                ],
                [
                    0x01, 0x01, 0x01, 0x01, 0xff, 0x03, 0x56, 0xab, 0x01, 0x00, 0x01, 0x00, 0x01,
                    0x00, 0x01, 0x00,
                ],
            ),
            // i32x4 signed vs unsigned boundaries + i64x2 wrapping mul
            (
                v128_from_f32s([0x8000_0000, 0x7fff_ffff, 0xffff_ffff, 0x0000_0000]),
                v128_from_f32s([0x7fff_ffff, 0x8000_0000, 0x0000_0001, 0xffff_ffff]),
            ),
        ];
        let shifts = [0i32, 1, 7, 31, 32, 33, -1, 63, 0x1234_abcd_u32 as i32];
        for (va, vb) in pairs {
            for m in 0..hot_simd_modes {
                let sh_list: &[i32] = if m >= hot_simd_modes - 3 || m == 6 || m == 7 {
                    &shifts
                } else {
                    &[5]
                };
                for &sh in sh_list {
                    let (d, r) = invoke_pair(
                        &mut st_dec,
                        &mut st_reg,
                        "hot_simd",
                        &[
                            Value::V128(V128(va)),
                            Value::V128(V128(vb)),
                            Value::I32(sh),
                            Value::I32(m),
                        ],
                    );
                    assert_eq!(d, r, "hot_simd parity mismatch for mode={m}, sh={sh}");
                }
            }
        }

        // 4) v128.any_true, i8x16.bitmask, i32x4.bitmask, i32x4/f32x4.extract_lane parity!
        let mut mask_vecs: Vec<[u8; 16]> = vec![
            [0u8; 16],
            [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
            [0x7f; 16],
            [0x80; 16],
            [
                0x80, 0x7f, 0xff, 0x00, 0x81, 0x01, 0x7e, 0xfe, 0x00, 0x80, 0x7f, 0xff, 0x01, 0x81,
                0xaa, 0x55,
            ],
            v128_from_f32s([
                0x7f80_0005,          // +sNaN
                (-0.0_f32).to_bits(), // -0.0
                0xffc0_4321,          // -qNaN
                0xffff_ffff,          // -1 / NaN
            ]),
        ];
        // Cover all 16 combinations of i32x4 lane sign bits (with non-zero low 31 bits!).
        for bits in 0u32..16 {
            mask_vecs.push(v128_from_f32s([
                ((bits & 1) << 31) | 0x1234_5678,
                (((bits >> 1) & 1) << 31) | 0x7fff_ffff,
                (((bits >> 2) & 1) << 31) | 0x0000_0001,
                (((bits >> 3) & 1) << 31),
            ]));
        }
        for v in mask_vecs {
            for m in 0..=10 {
                let (d, r) = invoke_pair(
                    &mut st_dec,
                    &mut st_reg,
                    "masks_extract",
                    &[Value::V128(V128(v)), Value::I32(m)],
                );
                assert_eq!(
                    d, r,
                    "masks_extract parity mismatch for mode={m}, v={v:02x?}"
                );
            }
        }

        // 5) Exact fuel exhaustion & remaining-fuel parity on counted FMA + hot SIMD loop!
        let acc0 = v128_from_f32s([
            1.0_f32.to_bits(),
            (-2.5_f32).to_bits(),
            0.25_f32.to_bits(),
            (-0.0_f32).to_bits(),
        ]);
        let a0 = v128_from_f32s([
            1.5_f32.to_bits(),
            0.5_f32.to_bits(),
            (-1.25_f32).to_bits(),
            3.0_f32.to_bits(),
        ]);
        let b0_v = v128_from_f32s([
            (-0.75_f32).to_bits(),
            2.0_f32.to_bits(),
            0.125_f32.to_bits(),
            1.25_f32.to_bits(),
        ]);
        for fuel in (0u64..=80).chain([250]) {
            let mut pair = Vec::with_capacity(2);
            for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Register] {
                let mut fcfg = cfg.clone();
                fcfg.limits.fuel = Some(fuel);
                let eng = Engine::with_execution_policy(fcfg, policy).unwrap();
                let m = eng.compile(&wasm).unwrap();
                let mut st = Store::new(&eng);
                let ins = Linker::new(&eng).instantiate(&mut st, &m).unwrap();
                let External::Func(f) = ins.export(&st, "fma_loop").unwrap() else {
                    panic!()
                };
                let res = st
                    .invoke(
                        f,
                        &[
                            Value::I32(4),
                            Value::V128(V128(acc0)),
                            Value::V128(V128(a0)),
                            Value::V128(V128(b0_v)),
                        ],
                    )
                    .map_err(|e| (e.kind, e.trap_code, e.message));
                pair.push((res, st.current_fuel));
            }
            assert_eq!(
                pair[0], pair[1],
                "fma_loop fuel parity mismatch for fuel={fuel}"
            );
        }
    }
}
