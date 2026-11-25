//! Compact block-parameter SSA IR, single-pass Wasm -> SSA lowering, and fast O(N) optimizations.

#![allow(dead_code)]

use std::collections::HashMap;

use wtime_types::ValType;
use wtime_validate::{DecodedModule, Immediate, Instruction};

use super::module::{
    CompileBudget, MAX_BLOCKS_PER_FN, MAX_FRAME_SCAN_WRITES, MAX_IF_SNAPSHOT_ENTRIES,
    MAX_IR_VALS_PER_FN, MAX_VAL_POOL_PER_FN, X64ConstVal, X64Module,
};
use super::{TRAP_INDIRECT_CALL, TRAP_OOB};

pub type Res<T> = Result<T, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Val(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    I32,
    I64,
    F32,
    F64,
    V128,
}

impl Ty {
    pub fn from_val(v: ValType) -> Self {
        match v {
            ValType::I32 => Ty::I32,
            ValType::I64 | ValType::Ref(_) => Ty::I64,
            ValType::F32 => Ty::F32,
            ValType::F64 => Ty::F64,
            ValType::V128 => Ty::V128,
        }
    }

    #[inline]
    pub fn is_fp(self) -> bool {
        matches!(self, Ty::F32 | Ty::F64 | Ty::V128)
    }

    #[inline]
    pub fn is_gp(self) -> bool {
        matches!(self, Ty::I32 | Ty::I64)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntBinOp {
    Add,
    Sub,
    Mul,
    DivS,
    DivU,
    /// Unsigned division whose divisor is statically proven non-zero (traps-free; omits `div0` check).
    DivUNz,
    RemS,
    RemU,
    /// Unsigned remainder whose divisor is statically proven non-zero (traps-free; omits `div0` check).
    RemUNz,
    And,
    Or,
    Xor,
    Shl,
    ShrS,
    ShrU,
    Rotl,
    Rotr,
    /// High 64 bits of signed 64×64 -> 128-bit multiply (`i64.mul_wide_s` upper limb, `I64` only).
    MulHiS,
    /// High 64 bits of unsigned 64×64 -> 128-bit multiply (`i64.mul_wide_u` upper limb, `I64` only).
    MulHiU,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CmpOp {
    Eq,
    Ne,
    LtS,
    LtU,
    GtS,
    GtU,
    LeS,
    LeU,
    GeS,
    GeU,
}

impl CmpOp {
    pub fn swap(self) -> Self {
        match self {
            CmpOp::Eq => CmpOp::Eq,
            CmpOp::Ne => CmpOp::Ne,
            CmpOp::LtS => CmpOp::GtS,
            CmpOp::LtU => CmpOp::GtU,
            CmpOp::GtS => CmpOp::LtS,
            CmpOp::GtU => CmpOp::LtU,
            CmpOp::LeS => CmpOp::GeS,
            CmpOp::LeU => CmpOp::GeU,
            CmpOp::GeS => CmpOp::LeS,
            CmpOp::GeU => CmpOp::LeU,
        }
    }

    pub fn invert_int(self) -> Self {
        match self {
            CmpOp::Eq => CmpOp::Ne,
            CmpOp::Ne => CmpOp::Eq,
            CmpOp::LtS => CmpOp::GeS,
            CmpOp::LtU => CmpOp::GeU,
            CmpOp::GtS => CmpOp::LeS,
            CmpOp::GtU => CmpOp::LeU,
            CmpOp::LeS => CmpOp::GtS,
            CmpOp::LeU => CmpOp::GtU,
            CmpOp::GeS => CmpOp::LtS,
            CmpOp::GeU => CmpOp::LtU,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FloatUnOp {
    Abs,
    Neg,
    Ceil,
    Floor,
    Trunc,
    Nearest,
    Sqrt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FloatBinOp {
    Add,
    Sub,
    Mul,
    Div,
    Min,
    Max,
    Copysign,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TruncOp {
    I32F32S = 0,
    I32F32U = 1,
    I32F64S = 2,
    I32F64U = 3,
    I64F32S = 4,
    I64F32U = 5,
    I64F64S = 6,
    I64F64U = 7,
    I32F32SatS = 8,
    I32F32SatU = 9,
    I32F64SatS = 10,
    I32F64SatU = 11,
    I64F32SatS = 12,
    I64F32SatU = 13,
    I64F64SatS = 14,
    I64F64SatU = 15,
}

impl TruncOp {
    pub fn result_ty(self) -> Ty {
        match self {
            TruncOp::I32F32S
            | TruncOp::I32F32U
            | TruncOp::I32F64S
            | TruncOp::I32F64U
            | TruncOp::I32F32SatS
            | TruncOp::I32F32SatU
            | TruncOp::I32F64SatS
            | TruncOp::I32F64SatU => Ty::I32,
            _ => Ty::I64,
        }
    }

    pub fn can_trap(self) -> bool {
        (self as u8) < 8
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LoadKind {
    I32,
    I64,
    F32,
    F64,
    I32Load8S,
    I32Load8U,
    I32Load16S,
    I32Load16U,
    I64Load8S,
    I64Load8U,
    I64Load16S,
    I64Load16U,
    I64Load32S,
    I64Load32U,
    V128,
    V128Load8x8S,
    V128Load8x8U,
    V128Load16x4S,
    V128Load16x4U,
    V128Load32x2S,
    V128Load32x2U,
    V128Load8Splat,
    V128Load16Splat,
    V128Load32Splat,
    V128Load64Splat,
    V128Load32Zero,
    V128Load64Zero,
}

impl LoadKind {
    pub fn bytes(self) -> u64 {
        match self {
            LoadKind::I32Load8S
            | LoadKind::I32Load8U
            | LoadKind::I64Load8S
            | LoadKind::I64Load8U
            | LoadKind::V128Load8Splat => 1,
            LoadKind::I32Load16S
            | LoadKind::I32Load16U
            | LoadKind::I64Load16S
            | LoadKind::I64Load16U
            | LoadKind::V128Load16Splat => 2,
            LoadKind::I32
            | LoadKind::F32
            | LoadKind::I64Load32S
            | LoadKind::I64Load32U
            | LoadKind::V128Load32Splat
            | LoadKind::V128Load32Zero => 4,
            LoadKind::I64
            | LoadKind::F64
            | LoadKind::V128Load8x8S
            | LoadKind::V128Load8x8U
            | LoadKind::V128Load16x4S
            | LoadKind::V128Load16x4U
            | LoadKind::V128Load32x2S
            | LoadKind::V128Load32x2U
            | LoadKind::V128Load64Splat
            | LoadKind::V128Load64Zero => 8,
            LoadKind::V128 => 16,
        }
    }

    pub fn ty(self) -> Ty {
        match self {
            LoadKind::I32
            | LoadKind::I32Load8S
            | LoadKind::I32Load8U
            | LoadKind::I32Load16S
            | LoadKind::I32Load16U => Ty::I32,
            LoadKind::I64
            | LoadKind::I64Load8S
            | LoadKind::I64Load8U
            | LoadKind::I64Load16S
            | LoadKind::I64Load16U
            | LoadKind::I64Load32S
            | LoadKind::I64Load32U => Ty::I64,
            LoadKind::F32 => Ty::F32,
            LoadKind::F64 => Ty::F64,
            LoadKind::V128
            | LoadKind::V128Load8x8S
            | LoadKind::V128Load8x8U
            | LoadKind::V128Load16x4S
            | LoadKind::V128Load16x4U
            | LoadKind::V128Load32x2S
            | LoadKind::V128Load32x2U
            | LoadKind::V128Load8Splat
            | LoadKind::V128Load16Splat
            | LoadKind::V128Load32Splat
            | LoadKind::V128Load64Splat
            | LoadKind::V128Load32Zero
            | LoadKind::V128Load64Zero => Ty::V128,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SimdSplatKind {
    I8x16,
    I16x8,
    I32x4,
    I64x2,
    F32x4,
    F64x2,
}

impl SimdSplatKind {
    pub fn scalar_ty(self) -> Ty {
        match self {
            SimdSplatKind::I8x16 | SimdSplatKind::I16x8 | SimdSplatKind::I32x4 => Ty::I32,
            SimdSplatKind::I64x2 => Ty::I64,
            SimdSplatKind::F32x4 => Ty::F32,
            SimdSplatKind::F64x2 => Ty::F64,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SimdLaneKind {
    I8x16S,
    I8x16U,
    I16x8S,
    I16x8U,
    I32x4,
    I64x2,
    F32x4,
    F64x2,
}

impl SimdLaneKind {
    pub fn scalar_ty(self) -> Ty {
        match self {
            SimdLaneKind::I8x16S
            | SimdLaneKind::I8x16U
            | SimdLaneKind::I16x8S
            | SimdLaneKind::I16x8U
            | SimdLaneKind::I32x4 => Ty::I32,
            SimdLaneKind::I64x2 => Ty::I64,
            SimdLaneKind::F32x4 => Ty::F32,
            SimdLaneKind::F64x2 => Ty::F64,
        }
    }

    pub fn num_lanes(self) -> u8 {
        match self {
            SimdLaneKind::I8x16S | SimdLaneKind::I8x16U => 16,
            SimdLaneKind::I16x8S | SimdLaneKind::I16x8U => 8,
            SimdLaneKind::I32x4 | SimdLaneKind::F32x4 => 4,
            SimdLaneKind::I64x2 | SimdLaneKind::F64x2 => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreKind {
    I32,
    I64,
    F32,
    F64,
    I8,
    I16,
    V128,
}

impl StoreKind {
    pub fn bytes(self) -> u64 {
        match self {
            StoreKind::I8 => 1,
            StoreKind::I16 => 2,
            StoreKind::I32 | StoreKind::F32 => 4,
            StoreKind::I64 | StoreKind::F64 => 8,
            StoreKind::V128 => 16,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValRange {
    pub start: u32,
    pub len: u32,
}

impl ValRange {
    pub const EMPTY: Self = Self { start: 0, len: 0 };
}

#[derive(Clone, Copy, Debug)]
pub enum Op {
    ConstI32(i32),
    ConstI64(i64),
    ConstF32(u32),
    ConstF64(u64),
    ConstV128(u32), // index into IrFunc::v128_consts

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
    I32BinImm(IntBinOp, Val, i32),
    I64Bin(IntBinOp, Val, Val),
    I64BinImm(IntBinOp, Val, i32),
    /// Scaled integer addition `base + (idx << shift)` (`shift ∈ 1..=3`), lowered to a single
    /// 1-cycle `lea` on x64 and `add rd, rn, rm, lsl #shift` on AArch64.
    /// When `!is64`, `base` and `idx` may be `Ty::I32` or `Ty::I64` (low 32 bits used,
    /// zero-extended into 64 bits, folding `I32WrapI64` directly into the address op).
    AddScaled {
        is64: bool,
        base: Val,
        idx: Val,
        shift: u8,
    },

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
    Trunc(TruncOp, Val),

    Select {
        cond: Val,
        a: Val,
        b: Val,
    },
    /// Fused compare + select (`a cmp b ? tv : fv`; `b` unused for `*Imm`).
    CmpSelect {
        kind: CmpBranchKind,
        a: Val,
        b: Val,
        tv: Val,
        fv: Val,
    },

    GlobalGet(u32),
    GlobalSet(u32, Val),
    TableGet {
        table: u32,
        idx: Val,
    },
    TableSet {
        table: u32,
        idx: Val,
        val: Val,
    },

    BoundsCheck {
        addr: Val,
        end_off: u64,
        /// `true` when emitted to guard an atomic bulk-memory op (`MemCopyConst` forwarding)
        /// so standalone guard-page BCE must still execute this check before any stores.
        for_bulk: bool,
    },
    Load {
        kind: LoadKind,
        addr: Val,
        offset: u64,
        checked: bool,
    },
    /// Non-wrapping, already-bounds-checked scaled load (`[mem_base + zero_ext(addr)*scale + offset]`,
    /// `scale ∈ {2, 4, 8}`, Wasm32 only). Emitted late after vectorization/GVN.
    LoadScaled {
        kind: LoadKind,
        addr: Val,
        scale: u8,
        offset: u64,
    },
    Store {
        kind: StoreKind,
        addr: Val,
        val: Val,
        offset: u64,
        checked: bool,
    },
    /// Non-wrapping, already-bounds-checked scaled store (`[mem_base + zero_ext(addr)*scale + offset]`,
    /// `scale ∈ {2, 4, 8}`, Wasm32 only). Emitted late after vectorization/GVN.
    StoreScaled {
        kind: StoreKind,
        addr: Val,
        val: Val,
        scale: u8,
        offset: u64,
    },
    V128LoadLane {
        width: u8,
        addr: Val,
        vec: Val,
        offset: u64,
        lane: u8,
        checked: bool,
    },
    V128StoreLane {
        width: u8,
        addr: Val,
        vec: Val,
        offset: u64,
        lane: u8,
        checked: bool,
    },
    MemSize,
    MemGrow(Val),
    MemCopy {
        dst: Val,
        src: Val,
        len: Val,
    },
    MemFill {
        dst: Val,
        val: Val,
        len: Val,
    },
    /// Multi-memory load targeting memory `mem >= 1` (bounds-checked against `[Ctx + desc_off + 8]`).
    LoadMem {
        mem: u32,
        kind: LoadKind,
        addr: Val,
        offset: u64,
    },
    /// Multi-memory store targeting memory `mem >= 1`.
    StoreMem {
        mem: u32,
        kind: StoreKind,
        addr: Val,
        val: Val,
        offset: u64,
    },
    /// Multi-memory SIMD lane load targeting memory `mem >= 1`.
    V128LoadLaneMem {
        mem: u32,
        width: u8,
        addr: Val,
        vec: Val,
        offset: u64,
        lane: u8,
    },
    /// Multi-memory SIMD lane store targeting memory `mem >= 1`.
    V128StoreLaneMem {
        mem: u32,
        width: u8,
        addr: Val,
        vec: Val,
        offset: u64,
        lane: u8,
    },
    /// `memory.size m` for `m >= 1`.
    MemSizeIdx(u32),
    /// `memory.grow m` for `m >= 1`.
    MemGrowIdx {
        mem: u32,
        delta: Val,
    },
    /// `memory.copy dst_mem src_mem` when `dst_mem != 0 || src_mem != 0`.
    MemCopyIdx {
        dst_mem: u32,
        src_mem: u32,
        dst: Val,
        src: Val,
        len: Val,
    },
    /// `memory.fill m` for `m >= 1`.
    MemFillIdx {
        mem: u32,
        dst: Val,
        val: Val,
        len: Val,
    },
    /// `memory.init data mem` (`0xfc08`).
    MemInit {
        data: u32,
        mem: u32,
        dst: Val,
        src: Val,
        len: Val,
    },
    /// `data.drop data` (`0xfc09`).
    DataDrop(u32),
    /// `table.init elem table` (`0xfc0c`).
    TableInit {
        elem: u32,
        table: u32,
        dst: Val,
        src: Val,
        len: Val,
    },
    /// `elem.drop elem` (`0xfc0d`).
    ElemDrop(u32),
    /// `table.copy dst_table src_table` (`0xfc0e`).
    TableCopy {
        dst_table: u32,
        src_table: u32,
        dst: Val,
        src: Val,
        len: Val,
    },
    /// `table.grow table` (`0xfc0f`): `(init_val: i64, delta: i32) -> i32`.
    TableGrow {
        table: u32,
        init: Val,
        delta: Val,
    },
    /// `table.size table` (`0xfc10`) -> `i32`.
    TableSize(u32),
    /// `table.fill table` (`0xfc11`): `(dst: i32, val: i64, len: i32)`.
    TableFill {
        table: u32,
        dst: Val,
        val: Val,
        len: Val,
    },
    /// Inline small constant-length bulk copy (`len <= 64`, zero GP clobbers).
    MemCopyConst {
        dst: Val,
        src: Val,
        len: u32,
        dst_checked: bool,
        src_checked: bool,
    },
    /// Inline small constant-length bulk fill (`len <= 64`, zero GP clobbers).
    MemFillConst {
        dst: Val,
        val: Val,
        len: u32,
        checked: bool,
    },
    /// Bounds-checked-by-caller unsigned byte-sum reduction over `mem[base + offset .. + count]`
    /// (`count` is a multiple of 16; returns `i32` sum mod `2^32`, lowered to AVX2/SSE2/NEON).
    MemByteSum {
        base: Val,
        count: Val,
        offset: u64,
    },
    /// Bounds-checked-by-caller, non-overlapping 3-tap unsigned byte blur + constant unsigned
    /// division over `k in 0..count` (`count` is a multiple of 16):
    /// `mem[base + dst_off + k] = ((u32(mem[base + src_off + k]) + u32(mem[base + src_off + k + 1]) + u32(mem[base + src_off + k + 2])) /u divisor) as u8`.
    /// Lowered to 16B/iter AVX2 `vpmovzxbw`/`vpaddw`/`vpmulhuw`/`vpackuswb` (or 8B/iter SSE2).
    MemByteBlur3Div {
        base: Val,
        count: Val,
        src_off: u64,
        dst_off: u64,
        divisor: u16,
    },
    /// Bounds-checked-by-caller 4×`f64` (256-bit) strictly bit-exact IEEE-754 elementwise map
    /// (`mem[dst + dst_off .. +32] = op(mem[lhs + lhs_off .. +32], rhs_mem_or_splat)`).
    /// Lowered to 256-bit AVX2 `vmovupd`/`v{add,sub,mul,div}pd` (`ymm`) + `vzeroupper` when
    /// `isa.has_avx2`, or 2×128-bit SSE2 / NEON otherwise (zero FMA contraction).
    F64x4BinMem {
        op: FloatBinOp,
        dst: Val,
        dst_off: u64,
        lhs: Val,
        lhs_off: u64,
        rhs: Val,
        rhs_off: u64,
        rhs_is_splat: bool,
    },
    /// Bounds-checked-by-caller, non-overlapping (or exact-same-base) `f64` AXPY loop over
    /// `k in 0..count` (`count: I32` elements):
    /// `mem[dst + k*8] = mem[dst + k*8] (+/-) (mem[src + k*8] * scale)` (strictly bit-exact
    /// IEEE-754 `mul` followed by `add`/`sub`, zero FMA contraction).
    /// Lowered to 2×-unrolled 256-bit AVX2 (8× `f64`/iter) on `x64-v3`, 128-bit SSE2 on `x64-v2`,
    /// or 128-bit NEON on `aarch64`, with scalar tails.
    F64AxpyLoop {
        dst: Val,
        src: Val,
        count: Val,
        scale: Val,
        is_sub: bool,
    },

    V128Splat {
        kind: SimdSplatKind,
        val: Val,
    },
    V128ExtractLane {
        kind: SimdLaneKind,
        v: Val,
        lane: u8,
    },
    V128ReplaceLane {
        kind: SimdLaneKind,
        v: Val,
        x: Val,
        lane: u8,
    },
    V128Shuffle {
        a: Val,
        b: Val,
        lanes_idx: u32, // index into IrFunc::v128_consts
    },
    V128Un {
        op: u16, // Wasm 0xfdXX opcode
        a: Val,
    },
    V128Bin {
        op: u16, // Wasm 0xfdXX opcode
        a: Val,
        b: Val,
    },
    V128Ternary {
        op: u16, // Wasm 0xfd52 (bitselect)
        a: Val,
        b: Val,
        c: Val,
    },
    V128ToI32 {
        op: u16, // any_true, all_true, bitmask
        a: Val,
    },
    V128Shift {
        op: u16, // shl, shr_s, shr_u
        v: Val,
        shift: Val,
    },

    Call {
        func: u32,
        args: ValRange,
        rets: ValRange,
    },
    CallIndirect {
        ci_id: u32,
        idx: Val,
        args: ValRange,
        rets: ValRange,
    },

    /// Allocates an exception record in the demand-paged exception slab (`I64` handle).
    ExnAlloc {
        tag: u32,
        args: ValRange,
    },
    /// Stores the active exception handle (`[Ctx + exn_state_off + 0] = handle`).
    ExnSetActive(Val),
    /// Loads the active exception handle (`I64`) from `[Ctx + exn_state_off + 0]`.
    ExnGetActive,
    /// Clears `[Ctx + exn_state_off + 0] = 0`.
    ExnClearActive,
    /// Loads the `u32` tag index (`I32`) of exception handle `h`.
    ExnGetTag(Val),
    /// Loads payload field `field` (`ty`) of exception handle `h` for `tag`.
    ExnGetPayload {
        handle: Val,
        tag: u32,
        field: u32,
        ty: Ty,
    },

    Trap(u8),
}

impl Op {
    pub fn has_side_effect(&self) -> bool {
        match *self {
            Op::I32Bin(op, _, _) | Op::I64Bin(op, _, _) => {
                matches!(
                    op,
                    IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU
                )
            }
            Op::I32BinImm(op, _, imm) | Op::I64BinImm(op, _, imm) => {
                matches!(
                    op,
                    IntBinOp::DivS | IntBinOp::DivU | IntBinOp::RemS | IntBinOp::RemU
                ) && (imm == 0 || (op == IntBinOp::DivS && imm == -1))
            }
            Op::Trunc(t, _) => t.can_trap(),
            Op::GlobalSet(_, _)
            | Op::TableGet { .. }
            | Op::TableSet { .. }
            | Op::BoundsCheck { .. }
            | Op::Load { checked: false, .. }
            | Op::Store { .. }
            | Op::StoreScaled { .. }
            | Op::V128LoadLane { checked: false, .. }
            | Op::V128StoreLane { .. }
            | Op::MemGrow(_)
            | Op::MemCopy { .. }
            | Op::MemFill { .. }
            | Op::LoadMem { .. }
            | Op::StoreMem { .. }
            | Op::V128LoadLaneMem { .. }
            | Op::V128StoreLaneMem { .. }
            | Op::MemGrowIdx { .. }
            | Op::MemCopyIdx { .. }
            | Op::MemFillIdx { .. }
            | Op::MemInit { .. }
            | Op::DataDrop(_)
            | Op::TableInit { .. }
            | Op::ElemDrop(_)
            | Op::TableCopy { .. }
            | Op::TableGrow { .. }
            | Op::TableFill { .. }
            | Op::MemCopyConst { .. }
            | Op::MemFillConst { .. }
            | Op::MemByteBlur3Div { .. }
            | Op::F64x4BinMem { .. }
            | Op::F64AxpyLoop { .. }
            | Op::Call { .. }
            | Op::CallIndirect { .. }
            | Op::ExnAlloc { .. }
            | Op::ExnSetActive(_)
            | Op::ExnGetActive
            | Op::ExnClearActive
            | Op::ExnGetTag(_)
            | Op::ExnGetPayload { .. }
            | Op::Trap(_) => true,
            _ => false,
        }
    }

    pub fn for_each_use(&self, pool: &[Val], mut f: impl FnMut(Val)) {
        match *self {
            Op::ConstI32(_)
            | Op::ConstI64(_)
            | Op::ConstF32(_)
            | Op::ConstF64(_)
            | Op::ConstV128(_)
            | Op::GlobalGet(_)
            | Op::MemSize
            | Op::MemSizeIdx(_)
            | Op::DataDrop(_)
            | Op::ElemDrop(_)
            | Op::TableSize(_)
            | Op::ExnGetActive
            | Op::ExnClearActive
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
            | Op::LoadScaled { addr: a, .. }
            | Op::LoadMem { addr: a, .. }
            | Op::MemGrow(a)
            | Op::MemGrowIdx { delta: a, .. }
            | Op::V128Splat { val: a, .. }
            | Op::V128ExtractLane { v: a, .. }
            | Op::V128Un { a, .. }
            | Op::V128ToI32 { a, .. }
            | Op::ExnSetActive(a)
            | Op::ExnGetTag(a)
            | Op::ExnGetPayload { handle: a, .. } => f(a),
            Op::I32Bin(_, a, b)
            | Op::I64Bin(_, a, b)
            | Op::AddScaled {
                base: a, idx: b, ..
            }
            | Op::I32Cmp(_, a, b)
            | Op::I64Cmp(_, a, b)
            | Op::F32Cmp(_, a, b)
            | Op::F64Cmp(_, a, b)
            | Op::F32Bin(_, a, b)
            | Op::F64Bin(_, a, b)
            | Op::TableSet { idx: a, val: b, .. }
            | Op::TableGrow {
                init: a, delta: b, ..
            }
            | Op::Store {
                addr: a, val: b, ..
            }
            | Op::StoreScaled {
                addr: a, val: b, ..
            }
            | Op::StoreMem {
                addr: a, val: b, ..
            }
            | Op::V128LoadLane {
                addr: a, vec: b, ..
            }
            | Op::V128LoadLaneMem {
                addr: a, vec: b, ..
            }
            | Op::V128StoreLane {
                addr: a, vec: b, ..
            }
            | Op::V128StoreLaneMem {
                addr: a, vec: b, ..
            }
            | Op::V128ReplaceLane { v: a, x: b, .. }
            | Op::V128Shuffle { a, b, .. }
            | Op::V128Bin { a, b, .. }
            | Op::V128Shift { v: a, shift: b, .. } => {
                f(a);
                f(b);
            }
            Op::V128Ternary { a, b, c, .. } => {
                f(a);
                f(b);
                f(c);
            }
            Op::Select { cond, a, b } => {
                f(cond);
                f(a);
                f(b);
            }
            Op::CmpSelect { kind, a, b, tv, fv } => {
                f(a);
                if !matches!(kind, CmpBranchKind::I32Imm(..) | CmpBranchKind::I64Imm(..)) {
                    f(b);
                }
                f(tv);
                f(fv);
            }
            Op::MemCopy { dst, src, len }
            | Op::MemCopyIdx { dst, src, len, .. }
            | Op::MemInit { dst, src, len, .. }
            | Op::TableInit { dst, src, len, .. }
            | Op::TableCopy { dst, src, len, .. } => {
                f(dst);
                f(src);
                f(len);
            }
            Op::MemFill { dst, val, len }
            | Op::MemFillIdx { dst, val, len, .. }
            | Op::TableFill { dst, val, len, .. } => {
                f(dst);
                f(val);
                f(len);
            }
            Op::MemCopyConst { dst, src, .. } => {
                f(dst);
                f(src);
            }
            Op::MemFillConst { dst, val, .. } => {
                f(dst);
                f(val);
            }
            Op::MemByteSum { base, count, .. } | Op::MemByteBlur3Div { base, count, .. } => {
                f(base);
                f(count);
            }
            Op::F64x4BinMem { dst, lhs, rhs, .. } => {
                f(dst);
                f(lhs);
                f(rhs);
            }
            Op::F64AxpyLoop {
                dst,
                src,
                count,
                scale,
                ..
            } => {
                f(dst);
                f(src);
                f(count);
                f(scale);
            }
            Op::Call { args, .. } | Op::ExnAlloc { args, .. } => {
                for &v in &pool[args.start as usize..args.start as usize + args.len as usize] {
                    f(v);
                }
            }
            Op::CallIndirect { idx, args, .. } => {
                for &v in &pool[args.start as usize..args.start as usize + args.len as usize] {
                    f(v);
                }
                f(idx);
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Inst {
    pub res: Option<Val>,
    pub op: Op,
}

#[derive(Clone, Copy, Debug)]
pub enum CmpBranchKind {
    I32(CmpOp),
    I32Imm(CmpOp, i32),
    I64(CmpOp),
    I64Imm(CmpOp, i32),
    F32(CmpOp),
    F64(CmpOp),
}

#[derive(Clone, Copy, Debug)]
pub enum Term {
    Jump {
        target: BlockId,
        args: ValRange,
    },
    BranchIf {
        cond: Val,
        then_bb: BlockId,
        then_args: ValRange,
        else_bb: BlockId,
        else_args: ValRange,
    },
    CmpBranch {
        kind: CmpBranchKind,
        a: Val,
        b: Val, // unused for *Imm
        then_bb: BlockId,
        then_args: ValRange,
        else_bb: BlockId,
        else_args: ValRange,
    },
    BrTable {
        idx: Val,
        table_idx: u32,
    },
    Return {
        vals: ValRange,
    },
    ReturnCall {
        func: u32,
        args: ValRange,
    },
    ReturnCallIndirect {
        ci_id: u32,
        idx: Val,
        args: ValRange,
    },
    /// Direct call inside a `try_table`; normal returns populate `ok_bb.params`, Wasm exceptions branch to `catch_bb(catch_args)`.
    CallCatch {
        func: u32,
        args: ValRange,
        ok_bb: BlockId,
        catch_bb: BlockId,
        catch_args: ValRange,
    },
    /// Indirect call inside a `try_table`; normal returns populate `ok_bb.params`, Wasm exceptions branch to `catch_bb(catch_args)`.
    CallIndirectCatch {
        ci_id: u32,
        idx: Val,
        args: ValRange,
        ok_bb: BlockId,
        catch_bb: BlockId,
        catch_args: ValRange,
    },
    /// Unwinds out of the current function with `INTERNAL_EXN_SIGNAL` (`0xff`) for an active Wasm exception.
    ThrowActive,
    Unreachable,
}

impl Term {
    pub fn for_each_use(&self, pool: &[Val], tables: &[BrTableData], mut f: impl FnMut(Val)) {
        fn emit_range(pool: &[Val], r: ValRange, f: &mut impl FnMut(Val)) {
            for &v in &pool[r.start as usize..r.start as usize + r.len as usize] {
                f(v);
            }
        }
        match *self {
            Term::Jump { args, .. } => emit_range(pool, args, &mut f),
            Term::BranchIf {
                cond,
                then_args,
                else_args,
                ..
            } => {
                f(cond);
                emit_range(pool, then_args, &mut f);
                emit_range(pool, else_args, &mut f);
            }
            Term::CmpBranch {
                kind,
                a,
                b,
                then_args,
                else_args,
                ..
            } => {
                f(a);
                if !matches!(
                    kind,
                    CmpBranchKind::I32Imm(_, _) | CmpBranchKind::I64Imm(_, _)
                ) {
                    f(b);
                }
                emit_range(pool, then_args, &mut f);
                emit_range(pool, else_args, &mut f);
            }
            Term::BrTable { idx, table_idx } => {
                f(idx);
                let bt = &tables[table_idx as usize];
                for &(_, args) in &bt.unique_edges {
                    emit_range(pool, args, &mut f);
                }
            }
            Term::Return { vals } => emit_range(pool, vals, &mut f),
            Term::ReturnCall { args, .. } => emit_range(pool, args, &mut f),
            Term::ReturnCallIndirect { idx, args, .. } => {
                f(idx);
                emit_range(pool, args, &mut f);
            }
            Term::CallCatch {
                args, catch_args, ..
            } => {
                emit_range(pool, args, &mut f);
                emit_range(pool, catch_args, &mut f);
            }
            Term::CallIndirectCatch {
                idx,
                args,
                catch_args,
                ..
            } => {
                f(idx);
                emit_range(pool, args, &mut f);
                emit_range(pool, catch_args, &mut f);
            }
            Term::ThrowActive | Term::Unreachable => {}
        }
    }

    pub fn for_each_succ(&self, tables: &[BrTableData], mut f: impl FnMut(BlockId)) {
        match *self {
            Term::Jump { target, .. } => f(target),
            Term::BranchIf {
                then_bb, else_bb, ..
            }
            | Term::CmpBranch {
                then_bb, else_bb, ..
            } => {
                f(then_bb);
                f(else_bb);
            }
            Term::BrTable { table_idx, .. } => {
                let bt = &tables[table_idx as usize];
                for &(bb, _) in &bt.unique_edges {
                    f(bb);
                }
            }
            Term::CallCatch {
                ok_bb, catch_bb, ..
            }
            | Term::CallIndirectCatch {
                ok_bb, catch_bb, ..
            } => {
                f(ok_bb);
                f(catch_bb);
            }
            Term::Return { .. }
            | Term::ReturnCall { .. }
            | Term::ReturnCallIndirect { .. }
            | Term::ThrowActive
            | Term::Unreachable => {}
        }
    }
}

#[derive(Clone)]
pub struct BrTableData {
    /// Raw dispatch entries in table-index order (used only for jump-table codegen).
    pub targets: Vec<(BlockId, ValRange)>,
    pub default: (BlockId, ValRange),
    /// Deduplicated `(target_bb, args)` edges across `targets` + `default` (used by liveness, CFG, and DCE).
    pub unique_edges: Vec<(BlockId, ValRange)>,
}

#[derive(Clone)]
pub struct Block {
    pub params: Vec<Val>,
    pub insts: Vec<Inst>,
    pub term: Term,
    pub is_loop_header: bool,
    pub preds: u32,
}

impl Block {
    fn new(is_loop_header: bool) -> Self {
        Self {
            params: Vec::new(),
            insts: Vec::new(),
            term: Term::Unreachable,
            is_loop_header,
            preds: 0,
        }
    }
}

#[derive(Clone)]
pub struct IrFunc {
    pub func_index: usize,
    pub memory64: bool,
    pub params: Vec<Val>,
    pub result_tys: Vec<Ty>,
    pub val_tys: Vec<Ty>,
    pub blocks: Vec<Block>,
    pub val_pool: Vec<Val>,
    pub v128_consts: Vec<[u8; 16]>,
    pub br_tables: Vec<BrTableData>,
    pub needs_stack_check: bool,
    /// Extra native stack headroom reserved by a stack-check-retaining caller on behalf of
    /// direct stack-check-elided callees (Step 8a).
    pub elided_callee_stack_bytes: i32,
    /// Early-prune state set by `prune_early_block_params` (`0` = not early-pruned / mutated by
    /// inlining, `1` = CFG compacted & params pruned with changes so Stage 2 still needs one
    /// `optimize_ir` pass, `2` = CFG compacted & params pruned with zero changes).
    pub early_pruned: u8,
}

impl IrFunc {
    /// Fallibly clones the entire `IrFunc` using `try_reserve_exact` on all owned buffers,
    /// returning `None` on allocation failure so optional incremental cache work can fall back cleanly.
    pub fn try_clone(&self) -> Option<Self> {
        fn try_copy<T: Copy>(src: &[T]) -> Option<Vec<T>> {
            let mut out = Vec::new();
            out.try_reserve_exact(src.len()).ok()?;
            out.extend_from_slice(src);
            Some(out)
        }
        let params = try_copy(&self.params)?;
        let result_tys = try_copy(&self.result_tys)?;
        let val_tys = try_copy(&self.val_tys)?;
        let val_pool = try_copy(&self.val_pool)?;
        let v128_consts = try_copy(&self.v128_consts)?;

        let mut blocks = Vec::new();
        blocks.try_reserve_exact(self.blocks.len()).ok()?;
        for bb in &self.blocks {
            blocks.push(Block {
                params: try_copy(&bb.params)?,
                insts: try_copy(&bb.insts)?,
                term: bb.term,
                is_loop_header: bb.is_loop_header,
                preds: bb.preds,
            });
        }

        let mut br_tables = Vec::new();
        br_tables.try_reserve_exact(self.br_tables.len()).ok()?;
        for bt in &self.br_tables {
            br_tables.push(BrTableData {
                targets: try_copy(&bt.targets)?,
                default: bt.default,
                unique_edges: try_copy(&bt.unique_edges)?,
            });
        }

        Some(Self {
            func_index: self.func_index,
            memory64: self.memory64,
            params,
            result_tys,
            val_tys,
            blocks,
            val_pool,
            v128_consts,
            br_tables,
            needs_stack_check: self.needs_stack_check,
            elided_callee_stack_bytes: self.elided_callee_stack_bytes,
            early_pruned: self.early_pruned,
        })
    }

    pub fn push_val(&mut self, ty: Ty) -> Val {
        let id = self.val_tys.len() as u32;
        self.val_tys.push(ty);
        Val(id)
    }

    pub fn push_pool(&mut self, vals: &[Val]) -> Res<ValRange> {
        let new_len = self.val_pool.len().saturating_add(vals.len());
        if new_len > MAX_VAL_POOL_PER_FN {
            return Err(format!(
                "x64: function {} val_pool ({new_len}) exceeds cap {MAX_VAL_POOL_PER_FN}",
                self.func_index
            ));
        }
        let start =
            u32::try_from(self.val_pool.len()).map_err(|_| "x64: val_pool exceeded u32::MAX")?;
        let len = u32::try_from(vals.len()).map_err(|_| "x64: val range exceeded u32::MAX")?;
        let _ = start.checked_add(len).ok_or("x64: val_pool overflow")?;
        self.val_pool.extend_from_slice(vals);
        Ok(ValRange { start, len })
    }

    #[inline]
    pub fn pool_slice(&self, r: ValRange) -> &[Val] {
        &self.val_pool[r.start as usize..r.start as usize + r.len as usize]
    }
}

/// Pre-scanned per-frame modified live locals so structured Wasm lowers cleanly in one pass.
struct FrameScan {
    /// Dense map `pc -> index in `frame_locals`` (`u32::MAX` if no carried locals at `pc`).
    pc_to_slot: Vec<u32>,
    frame_locals: Vec<Vec<u32>>,
}

impl FrameScan {
    #[inline]
    fn take_locals(&mut self, pc: usize) -> Vec<u32> {
        if let Some(&slot) = self.pc_to_slot.get(pc)
            && slot != u32::MAX
        {
            std::mem::take(&mut self.frame_locals[slot as usize])
        } else {
            Vec::new()
        }
    }
}

fn scan_frame_locals(
    code: &[Instruction],
    n_locals: usize,
    budget: Option<&CompileBudget>,
) -> Res<FrameScan> {
    // Fast check: count structured control frames (`block`/`loop`/`if`/`try_table`); skip all allocations if none!
    let mut n_frames = 0usize;
    for insn in code {
        if matches!(insn.opcode, 0x02..=0x04 | 0x1f) {
            n_frames += 1;
        }
    }
    if n_frames == 0 {
        return Ok(FrameScan {
            pc_to_slot: Vec::new(),
            frame_locals: Vec::new(),
        });
    }

    // Pass 1: assign dense `frame_id` (0..n_frames) and matching `end_pc` for every `block`/`loop`/`if`/`try_table`.
    let mut pc_to_slot = vec![u32::MAX; code.len()];
    let mut frame_open_pc = Vec::with_capacity(n_frames);
    let mut frame_end_pc = vec![0usize; n_frames];
    {
        let mut stk: Vec<u32> = Vec::new();
        for (pc, insn) in code.iter().enumerate() {
            if let Some(bg) = budget
                && (pc & 4095) == 4095
            {
                bg.check_deadline()?;
            }
            match insn.opcode {
                0x02..=0x04 | 0x1f => {
                    let fid = frame_open_pc.len() as u32;
                    pc_to_slot[pc] = fid;
                    frame_open_pc.push(pc);
                    stk.push(fid);
                }
                0x0b => {
                    if let Some(fid) = stk.pop() {
                        frame_end_pc[fid as usize] = pc;
                    }
                }
                _ => {}
            }
        }
    }

    let mut open_frames: Vec<(u32, bool)> = Vec::new(); // (frame_id, is_loop)
    let mut open_loops: Vec<u32> = Vec::new();
    let mut frame_writes: Vec<Vec<u32>> = vec![Vec::new(); n_frames];
    let mut last_read = vec![0usize; n_locals];
    let mut total_marks = 0usize;

    for (pc, insn) in code.iter().enumerate() {
        if let Some(bg) = budget
            && (pc & 4095) == 4095
        {
            bg.check_deadline()?;
        }
        match insn.opcode {
            0x02 | 0x04 | 0x1f => open_frames.push((pc_to_slot[pc], false)),
            0x03 => {
                let fid = pc_to_slot[pc];
                open_frames.push((fid, true));
                open_loops.push(fid);
            }
            0x0b => {
                if let Some((_, is_loop)) = open_frames.pop()
                    && is_loop
                {
                    open_loops.pop();
                }
            }
            0x20 => {
                if let Immediate::Index(x) = insn.immediate {
                    let xu = x as usize;
                    if xu < n_locals {
                        let eff_end = open_loops
                            .first()
                            .map_or(pc, |&outer| frame_end_pc[outer as usize].max(pc));
                        if eff_end > last_read[xu] {
                            last_read[xu] = eff_end;
                        }
                    }
                }
            }
            0x21 | 0x22 => {
                if let Immediate::Index(x) = insn.immediate {
                    total_marks = total_marks.saturating_add(open_frames.len());
                    if total_marks > MAX_FRAME_SCAN_WRITES {
                        return Err(format!(
                            "x64: nested frame local-write scan ({total_marks}) exceeds cap {MAX_FRAME_SCAN_WRITES}"
                        ));
                    }
                    for &(fid, _) in &open_frames {
                        frame_writes[fid as usize].push(x);
                    }
                }
            }
            _ => {}
        }
    }

    for (fid, locals) in frame_writes.iter_mut().enumerate() {
        if locals.is_empty() {
            continue;
        }
        locals.sort_unstable();
        locals.dedup();
        let fpc = frame_open_pc[fid];
        let is_loop = code[fpc].opcode == 0x03;
        let epc = frame_end_pc[fid];
        locals.retain(|&x| {
            let lr = last_read[x as usize];
            if is_loop { lr > fpc } else { lr > epc }
        });
    }
    Ok(FrameScan {
        pc_to_slot,
        frame_locals: frame_writes,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameKind {
    Function,
    Block,
    Loop,
    If,
    TryTable,
}

struct TryTableState {
    catch_bb: BlockId,
    catches: Vec<(u8, Option<u32>, u32)>,
    entry_locals: Vec<Val>,
}

struct BuildFrame {
    kind: FrameKind,
    height: usize,
    params_len: usize,
    results: Vec<Ty>,
    /// Target block for `br`: loop header (for Loop) or merge block (for Block/If/TryTable). Unused for Function.
    target_bb: BlockId,
    /// Locals carried into `target_bb` as trailing block params.
    carried_locals: Vec<u32>,
    /// For `If`: else block, entry operand values, undo-log mark into `local_undo`, whether `else` was seen.
    else_bb: Option<BlockId>,
    if_entry_params: Vec<Val>,
    if_undo_mark: usize,
    has_else: bool,
    unreachable: bool,
    /// For `TryTable`: catch dispatcher block, source-order catch clauses, and entry locals snapshot.
    try_state: Option<Box<TryTableState>>,
}

#[derive(Clone, Copy)]
enum KnownConst {
    I32(i32),
    I64(i64),
}

struct Builder {
    ir: IrFunc,
    cur_bb: BlockId,
    consts: Vec<Option<KnownConst>>,
    const_fold: bool,
}

impl Builder {
    fn new(
        func_index: usize,
        memory64: bool,
        result_tys: Vec<Ty>,
        needs_stack_check: bool,
        const_fold: bool,
        wasm_insns_hint: usize,
    ) -> Self {
        let est_vals = wasm_insns_hint.clamp(16, 4096);
        let mut ir = IrFunc {
            func_index,
            memory64,
            params: Vec::new(),
            result_tys,
            val_tys: Vec::with_capacity(est_vals),
            blocks: vec![Block::new(false)],
            val_pool: Vec::with_capacity(est_vals / 2),
            v128_consts: Vec::new(),
            br_tables: Vec::new(),
            needs_stack_check,
            elided_callee_stack_bytes: 0,
            early_pruned: 0,
        };
        ir.blocks[0].preds = 1;
        Self {
            ir,
            cur_bb: BlockId(0),
            consts: Vec::with_capacity(est_vals),
            const_fold,
        }
    }

    fn new_block(&mut self, is_loop_header: bool) -> Res<BlockId> {
        if self.ir.blocks.len() >= MAX_BLOCKS_PER_FN {
            return Err(format!(
                "x64: function {} basic blocks exceed cap {MAX_BLOCKS_PER_FN}",
                self.ir.func_index
            ));
        }
        let id = BlockId(self.ir.blocks.len() as u32);
        self.ir.blocks.push(Block::new(is_loop_header));
        Ok(id)
    }

    fn add_block_param(&mut self, bb: BlockId, ty: Ty) -> Val {
        let v = self.ir.push_val(ty);
        self.consts.push(None);
        self.ir.blocks[bb.0 as usize].params.push(v);
        v
    }

    fn emit(&mut self, ty: Ty, op: Op) -> Val {
        let k = match op {
            Op::ConstI32(c) => Some(KnownConst::I32(c)),
            Op::ConstI64(c) => Some(KnownConst::I64(c)),
            _ => None,
        };
        let v = self.ir.push_val(ty);
        self.consts.push(k);
        self.ir.blocks[self.cur_bb.0 as usize]
            .insts
            .push(Inst { res: Some(v), op });
        v
    }

    fn emit_void(&mut self, op: Op) {
        self.ir.blocks[self.cur_bb.0 as usize]
            .insts
            .push(Inst { res: None, op });
    }

    fn const_i32(&mut self, c: i32) -> Val {
        self.emit(Ty::I32, Op::ConstI32(c))
    }

    fn const_i64(&mut self, c: i64) -> Val {
        self.emit(Ty::I64, Op::ConstI64(c))
    }

    fn const_zero(&mut self, ty: Ty) -> Val {
        match ty {
            Ty::I32 => self.emit(Ty::I32, Op::ConstI32(0)),
            Ty::I64 => self.emit(Ty::I64, Op::ConstI64(0)),
            Ty::F32 => self.emit(Ty::F32, Op::ConstF32(0)),
            Ty::F64 => self.emit(Ty::F64, Op::ConstF64(0)),
            Ty::V128 => {
                let idx = self.ir.v128_consts.len() as u32;
                self.ir.v128_consts.push([0u8; 16]);
                self.emit(Ty::V128, Op::ConstV128(idx))
            }
        }
    }

    #[inline]
    fn as_i32(&self, v: Val) -> Option<i32> {
        if !self.const_fold {
            return None;
        }
        match self.consts.get(v.0 as usize).copied().flatten() {
            Some(KnownConst::I32(c)) => Some(c),
            _ => None,
        }
    }

    #[inline]
    fn as_i64(&self, v: Val) -> Option<i64> {
        if !self.const_fold {
            return None;
        }
        match self.consts.get(v.0 as usize).copied().flatten() {
            Some(KnownConst::I64(c)) => Some(c),
            _ => None,
        }
    }

    fn emit_i32_bin(&mut self, mut op: IntBinOp, mut a: Val, mut b: Val) -> Val {
        // Canonicalize commutative constants to RHS.
        if matches!(
            op,
            IntBinOp::Add | IntBinOp::Mul | IntBinOp::And | IntBinOp::Or | IntBinOp::Xor
        ) && self.as_i32(a).is_some()
            && self.as_i32(b).is_none()
        {
            std::mem::swap(&mut a, &mut b);
        }
        // Full constant folding (except trapping div/rem).
        if let (Some(ca), Some(cb)) = (self.as_i32(a), self.as_i32(b)) {
            let folded = match op {
                IntBinOp::Add => Some(ca.wrapping_add(cb)),
                IntBinOp::Sub => Some(ca.wrapping_sub(cb)),
                IntBinOp::Mul => Some(ca.wrapping_mul(cb)),
                IntBinOp::DivS if cb != 0 && !(ca == i32::MIN && cb == -1) => {
                    Some(ca.wrapping_div(cb))
                }
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
            };
            if let Some(v) = folded {
                return self.const_i32(v);
            }
        }
        if self.const_fold && a == b {
            match op {
                IntBinOp::Sub | IntBinOp::Xor => return self.const_i32(0),
                IntBinOp::And | IntBinOp::Or => return a,
                _ => {}
            }
        }
        if let Some(cb) = self.as_i32(b) {
            match op {
                IntBinOp::Add | IntBinOp::Sub | IntBinOp::Or | IntBinOp::Xor if cb == 0 => {
                    return a;
                }
                IntBinOp::And if cb == -1 => return a,
                IntBinOp::And | IntBinOp::Mul if cb == 0 => return self.const_i32(0),
                IntBinOp::Mul if cb == 1 => return a,
                IntBinOp::Mul if (cb as u32).is_power_of_two() => {
                    let sh = (cb as u32).trailing_zeros() as i32;
                    return self.emit(Ty::I32, Op::I32BinImm(IntBinOp::Shl, a, sh));
                }
                IntBinOp::DivS if cb == 1 => return a,
                IntBinOp::RemS if cb == 1 || cb == -1 => return self.const_i32(0),
                IntBinOp::DivU if cb == 1 => return a,
                IntBinOp::RemU if cb == 1 => return self.const_i32(0),
                IntBinOp::DivU if (cb as u32).is_power_of_two() => {
                    let sh = (cb as u32).trailing_zeros() as i32;
                    return self.emit(Ty::I32, Op::I32BinImm(IntBinOp::ShrU, a, sh));
                }
                IntBinOp::RemU if (cb as u32).is_power_of_two() => {
                    return self.emit(
                        Ty::I32,
                        Op::I32BinImm(IntBinOp::And, a, ((cb as u32) - 1) as i32),
                    );
                }
                IntBinOp::DivU | IntBinOp::RemU if cb != 0 => {
                    return self.emit(Ty::I32, Op::I32BinImm(op, a, cb));
                }
                IntBinOp::DivS | IntBinOp::RemS if cb != 0 && cb != -1 => {
                    return self.emit(Ty::I32, Op::I32BinImm(op, a, cb));
                }
                IntBinOp::Shl
                | IntBinOp::ShrS
                | IntBinOp::ShrU
                | IntBinOp::Rotl
                | IntBinOp::Rotr => {
                    let sh = (cb as u32) & 31;
                    if sh == 0 {
                        return a;
                    }
                    return self.emit(Ty::I32, Op::I32BinImm(op, a, sh as i32));
                }
                IntBinOp::Add
                | IntBinOp::Sub
                | IntBinOp::Mul
                | IntBinOp::And
                | IntBinOp::Or
                | IntBinOp::Xor => {
                    return self.emit(Ty::I32, Op::I32BinImm(op, a, cb));
                }
                _ => {}
            }
        }
        let _ = &mut op;
        self.emit(Ty::I32, Op::I32Bin(op, a, b))
    }

    fn emit_i64_bin(&mut self, op: IntBinOp, mut a: Val, mut b: Val) -> Val {
        if matches!(
            op,
            IntBinOp::Add
                | IntBinOp::Mul
                | IntBinOp::MulHiS
                | IntBinOp::MulHiU
                | IntBinOp::And
                | IntBinOp::Or
                | IntBinOp::Xor
        ) && self.as_i64(a).is_some()
            && self.as_i64(b).is_none()
        {
            std::mem::swap(&mut a, &mut b);
        }
        if let (Some(ca), Some(cb)) = (self.as_i64(a), self.as_i64(b)) {
            let folded = match op {
                IntBinOp::Add => Some(ca.wrapping_add(cb)),
                IntBinOp::Sub => Some(ca.wrapping_sub(cb)),
                IntBinOp::Mul => Some(ca.wrapping_mul(cb)),
                IntBinOp::MulHiS => Some(((i128::from(ca) * i128::from(cb)) >> 64) as i64),
                IntBinOp::MulHiU => {
                    Some(((u128::from(ca as u64) * u128::from(cb as u64)) >> 64) as i64)
                }
                IntBinOp::DivS if cb != 0 && !(ca == i64::MIN && cb == -1) => {
                    Some(ca.wrapping_div(cb))
                }
                IntBinOp::DivU if cb != 0 => Some(((ca as u64) / (cb as u64)) as i64),
                IntBinOp::RemS if cb != 0 => Some(if ca == i64::MIN && cb == -1 {
                    0
                } else {
                    ca.wrapping_rem(cb)
                }),
                IntBinOp::RemU if cb != 0 => Some(((ca as u64) % (cb as u64)) as i64),
                IntBinOp::And => Some(ca & cb),
                IntBinOp::Or => Some(ca | cb),
                IntBinOp::Xor => Some(ca ^ cb),
                IntBinOp::Shl => Some(ca.wrapping_shl(cb as u32)),
                IntBinOp::ShrS => Some(ca.wrapping_shr(cb as u32)),
                IntBinOp::ShrU => Some(((ca as u64).wrapping_shr(cb as u32)) as i64),
                IntBinOp::Rotl => Some((ca as u64).rotate_left((cb as u32) & 63) as i64),
                IntBinOp::Rotr => Some((ca as u64).rotate_right((cb as u32) & 63) as i64),
                _ => None,
            };
            if let Some(v) = folded {
                return self.const_i64(v);
            }
        }
        if self.const_fold && a == b {
            match op {
                IntBinOp::Sub | IntBinOp::Xor => return self.const_i64(0),
                IntBinOp::And | IntBinOp::Or => return a,
                _ => {}
            }
        }
        if let Some(cb) = self.as_i64(b) {
            match op {
                IntBinOp::Add | IntBinOp::Sub | IntBinOp::Or | IntBinOp::Xor if cb == 0 => {
                    return a;
                }
                IntBinOp::And if cb == -1 => return a,
                IntBinOp::And | IntBinOp::Mul if cb == 0 => return self.const_i64(0),
                IntBinOp::Mul if cb == 1 => return a,
                IntBinOp::Mul if (cb as u64).is_power_of_two() => {
                    let sh = (cb as u64).trailing_zeros() as i32;
                    return self.emit(Ty::I64, Op::I64BinImm(IntBinOp::Shl, a, sh));
                }
                IntBinOp::DivS if cb == 1 => return a,
                IntBinOp::RemS if cb == 1 || cb == -1 => return self.const_i64(0),
                IntBinOp::DivU if cb == 1 => return a,
                IntBinOp::RemU if cb == 1 => return self.const_i64(0),
                IntBinOp::DivU if (cb as u64).is_power_of_two() => {
                    let sh = (cb as u64).trailing_zeros() as i32;
                    return self.emit(Ty::I64, Op::I64BinImm(IntBinOp::ShrU, a, sh));
                }
                IntBinOp::RemU
                    if (cb as u64).is_power_of_two() && ((cb as u64) - 1) <= i32::MAX as u64 =>
                {
                    return self.emit(
                        Ty::I64,
                        Op::I64BinImm(IntBinOp::And, a, ((cb as u64) - 1) as i32),
                    );
                }
                IntBinOp::DivU | IntBinOp::RemU if cb >= 2 && cb <= i32::MAX as i64 => {
                    return self.emit(Ty::I64, Op::I64BinImm(op, a, cb as i32));
                }
                IntBinOp::DivS | IntBinOp::RemS if cb != 0 && cb != -1 => {
                    if let Ok(imm32) = i32::try_from(cb) {
                        return self.emit(Ty::I64, Op::I64BinImm(op, a, imm32));
                    }
                }
                IntBinOp::Shl
                | IntBinOp::ShrS
                | IntBinOp::ShrU
                | IntBinOp::Rotl
                | IntBinOp::Rotr => {
                    let sh = (cb as u32) & 63;
                    if sh == 0 {
                        return a;
                    }
                    return self.emit(Ty::I64, Op::I64BinImm(op, a, sh as i32));
                }
                IntBinOp::Add
                | IntBinOp::Sub
                | IntBinOp::Mul
                | IntBinOp::And
                | IntBinOp::Or
                | IntBinOp::Xor => {
                    if let Ok(imm32) = i32::try_from(cb) {
                        return self.emit(Ty::I64, Op::I64BinImm(op, a, imm32));
                    }
                }
                _ => {}
            }
        }
        self.emit(Ty::I64, Op::I64Bin(op, a, b))
    }

    fn emit_i32_cmp(&mut self, mut op: CmpOp, mut a: Val, mut b: Val) -> Val {
        if let (Some(ca), Some(cb)) = (self.as_i32(a), self.as_i32(b)) {
            let r = match op {
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
            };
            return self.const_i32(i32::from(r));
        }
        if self.as_i32(a).is_some() && self.as_i32(b).is_none() {
            std::mem::swap(&mut a, &mut b);
            op = op.swap();
        }
        if let Some(cb) = self.as_i32(b) {
            return self.emit(Ty::I32, Op::I32CmpImm(op, a, cb));
        }
        self.emit(Ty::I32, Op::I32Cmp(op, a, b))
    }

    fn emit_i64_cmp(&mut self, mut op: CmpOp, mut a: Val, mut b: Val) -> Val {
        if let (Some(ca), Some(cb)) = (self.as_i64(a), self.as_i64(b)) {
            let r = match op {
                CmpOp::Eq => ca == cb,
                CmpOp::Ne => ca != cb,
                CmpOp::LtS => ca < cb,
                CmpOp::LtU => (ca as u64) < (cb as u64),
                CmpOp::GtS => ca > cb,
                CmpOp::GtU => (ca as u64) > (cb as u64),
                CmpOp::LeS => ca <= cb,
                CmpOp::LeU => (ca as u64) <= (cb as u64),
                CmpOp::GeS => ca >= cb,
                CmpOp::GeU => (ca as u64) >= (cb as u64),
            };
            return self.const_i32(i32::from(r));
        }
        if self.as_i64(a).is_some() && self.as_i64(b).is_none() {
            std::mem::swap(&mut a, &mut b);
            op = op.swap();
        }
        if let Some(cb) = self.as_i64(b)
            && let Ok(imm32) = i32::try_from(cb)
        {
            return self.emit(Ty::I32, Op::I64CmpImm(op, a, imm32));
        }
        self.emit(Ty::I32, Op::I64Cmp(op, a, b))
    }

    fn terminate(&mut self, term: Term) {
        self.ir.blocks[self.cur_bb.0 as usize].term = term;
    }

    fn add_pred(&mut self, bb: BlockId) {
        self.ir.blocks[bb.0 as usize].preds += 1;
    }
}

fn block_type_tys(module: &X64Module, imm: &Immediate) -> Res<(Vec<Ty>, Vec<Ty>)> {
    Ok(match imm {
        Immediate::Block(-64) => (vec![], vec![]),
        Immediate::Block(-1) => (vec![], vec![Ty::I32]),
        Immediate::Block(-2) => (vec![], vec![Ty::I64]),
        Immediate::Block(-3) => (vec![], vec![Ty::F32]),
        Immediate::Block(-4) => (vec![], vec![Ty::F64]),
        Immediate::Block(-5) => (vec![], vec![Ty::V128]),
        Immediate::Block(-16) | Immediate::Block(-17) | Immediate::Block(-23) => {
            (vec![], vec![Ty::I64])
        }
        Immediate::BlockType(t) => (vec![], vec![Ty::from_val(*t)]),
        Immediate::Block(i) if *i >= 0 => {
            let t = module
                .module_types
                .get(*i as usize)
                .ok_or("block type index")?;
            (
                t.params.iter().map(|&v| Ty::from_val(v)).collect(),
                t.results.iter().map(|&v| Ty::from_val(v)).collect(),
            )
        }
        _ => return Err("unexpected block type immediate".into()),
    })
}

/// Lowers defined Wasm function `func_index` (`>= module.imports.len()`) into optimized SSA IR.
pub fn lower_wasm_to_ir(
    decoded: &DecodedModule,
    module: &X64Module,
    func_index: usize,
) -> Res<IrFunc> {
    lower_wasm_to_ir_with_budget(decoded, module, func_index, true, None)
}

/// Lowers defined Wasm function `func_index` (`>= module.imports.len()`) into raw unoptimized SSA IR
/// (disabling construction-time constant/algebraic/select/fixed-memsize folding and skipping `optimize_ir`),
/// enabling differential verification of unoptimized vs optimized lowering.
pub fn lower_wasm_to_ir_unopt(
    decoded: &DecodedModule,
    module: &X64Module,
    func_index: usize,
) -> Res<IrFunc> {
    lower_wasm_to_ir_with_budget(decoded, module, func_index, false, None)
}

/// Lowers defined Wasm function `func_index` into SSA IR with cooperative deadline checks and block caps.
pub fn lower_wasm_to_ir_with_budget(
    decoded: &DecodedModule,
    module: &X64Module,
    func_index: usize,
    opt: bool,
    budget: Option<&CompileBudget>,
) -> Res<IrFunc> {
    let mut opt_scratch = OptIrScratch::default();
    lower_wasm_to_ir_with_budget_and_scratch(
        decoded,
        module,
        func_index,
        opt,
        budget,
        &mut opt_scratch,
    )
}

/// Same as `lower_wasm_to_ir_with_budget`, reusing worker-local `OptIrScratch` buffers.
pub fn lower_wasm_to_ir_with_budget_and_scratch(
    decoded: &DecodedModule,
    module: &X64Module,
    func_index: usize,
    opt: bool,
    budget: Option<&CompileBudget>,
    opt_scratch: &mut OptIrScratch,
) -> Res<IrFunc> {
    if let Some(bg) = budget {
        bg.check_deadline()?;
    }
    let body_idx = func_index - module.imports.len();
    let body = &decoded.code[body_idx];
    let ty = module
        .func_type(func_index)
        .ok_or("function type index out of range")?;
    let result_tys: Vec<Ty> = ty.results.iter().map(|&v| Ty::from_val(v)).collect();

    let mut local_tys: Vec<Ty> = ty.params.iter().map(|&v| Ty::from_val(v)).collect();
    for &vt in &body.locals {
        local_tys.push(Ty::from_val(vt));
    }

    let mut scan = scan_frame_locals(&body.instructions, local_tys.len(), budget)?;
    let mut local_undo: Vec<(u32, Val)> = Vec::new();
    let mut active_if_then_depth: usize = 0;
    let mut total_if_snapshot_entries: usize = 0;
    // Stage 1 checks prospective stack depth on every frame; Stage 2 can elide only based on
    // allocated native frame bytes and call depth.
    let mut b = Builder::new(
        func_index,
        module.memory64,
        result_tys.clone(),
        true,
        opt,
        body.instructions.len(),
    );

    // Initialize params and zero locals in entry block (bb0).
    let mut locals: Vec<Val> = Vec::with_capacity(local_tys.len());
    for &t in &local_tys[..ty.params.len()] {
        let v = b.add_block_param(BlockId(0), t);
        b.ir.params.push(v);
        locals.push(v);
    }
    let mut cached_zeros: [Option<Val>; 5] = [None; 5];
    for &t in &local_tys[ty.params.len()..] {
        let idx = match t {
            Ty::I32 => 0,
            Ty::I64 => 1,
            Ty::F32 => 2,
            Ty::F64 => 3,
            Ty::V128 => 4,
        };
        let z = match cached_zeros[idx] {
            Some(v) => v,
            None => {
                let v = b.const_zero(t);
                cached_zeros[idx] = Some(v);
                v
            }
        };
        locals.push(z);
    }

    let mut stack: Vec<Val> = Vec::with_capacity(32);
    let mut frames: Vec<BuildFrame> = vec![BuildFrame {
        kind: FrameKind::Function,
        height: 0,
        params_len: 0,
        results: result_tys,
        target_bb: BlockId(0),
        carried_locals: Vec::new(),
        else_bb: None,
        if_entry_params: Vec::new(),
        if_undo_mark: 0,
        has_else: false,
        unreachable: false,
        try_state: None,
    }];
    let mut dead: usize = 0;

    let set_local = |locals: &mut [Val],
                     local_undo: &mut Vec<(u32, Val)>,
                     active_if_then_depth: usize,
                     total_if_snapshot_entries: &mut usize,
                     lx: usize,
                     new_val: Val|
     -> Res<()> {
        let old = locals[lx];
        if active_if_then_depth > 0 && old != new_val {
            *total_if_snapshot_entries = total_if_snapshot_entries.saturating_add(1);
            if *total_if_snapshot_entries > MAX_IF_SNAPSHOT_ENTRIES {
                return Err(format!(
                    "x64: function {func_index} if-nesting snapshot budget ({}) exceeds cap {MAX_IF_SNAPSHOT_ENTRIES}",
                    *total_if_snapshot_entries
                ));
            }
            local_undo.push((lx as u32, old));
        }
        locals[lx] = new_val;
        Ok(())
    };

    let build_branch_args =
        |b: &mut Builder, stack: &[Val], locals: &[Val], frame: &BuildFrame| -> Res<ValRange> {
            let arity = if frame.kind == FrameKind::Loop {
                frame.params_len
            } else {
                frame.results.len()
            };
            let count = arity + frame.carried_locals.len();
            if count == 0 {
                return Ok(ValRange::EMPTY);
            }
            let new_len = b.ir.val_pool.len().saturating_add(count);
            if new_len > MAX_VAL_POOL_PER_FN {
                return Err(format!(
                    "x64: function {} val_pool ({new_len}) exceeds cap {MAX_VAL_POOL_PER_FN}",
                    b.ir.func_index
                ));
            }
            let start = u32::try_from(b.ir.val_pool.len())
                .map_err(|_| "x64: val_pool exceeded u32::MAX")?;
            let len = u32::try_from(count).map_err(|_| "x64: val range exceeded u32::MAX")?;
            let _ = start.checked_add(len).ok_or("x64: val_pool overflow")?;
            let base = stack.len() - arity;
            b.ir.val_pool.reserve(count);
            b.ir.val_pool.extend_from_slice(&stack[base..]);
            for &lx in &frame.carried_locals {
                b.ir.val_pool.push(locals[lx as usize]);
            }
            Ok(ValRange { start, len })
        };

    for (pc, insn) in body.instructions.iter().enumerate() {
        if let Some(bg) = budget
            && (pc & 4095) == 0
        {
            bg.check_deadline()?;
        }
        if b.ir.blocks.len() > MAX_BLOCKS_PER_FN {
            return Err(format!(
                "x64: function {func_index} blocks ({}) exceeds cap {MAX_BLOCKS_PER_FN}",
                b.ir.blocks.len()
            ));
        }
        if b.ir.val_tys.len() > MAX_IR_VALS_PER_FN {
            return Err(format!(
                "x64: function {func_index} SSA values ({}) exceeds cap {MAX_IR_VALS_PER_FN}",
                b.ir.val_tys.len()
            ));
        }
        let op = insn.opcode;
        if frames.last().is_some_and(|f| f.unreachable) {
            match op {
                0x02..=0x04 | 0x1f => {
                    dead += 1;
                    continue;
                }
                0x05 if dead > 0 => continue,
                0x0b if dead > 0 => {
                    dead -= 1;
                    continue;
                }
                0x05 | 0x0b => {}
                _ => continue,
            }
        }

        let imm_idx = || match insn.immediate {
            Immediate::Index(x) => Ok(x),
            _ => Err(format!("fn {func_index} pc {pc}: expected index")),
        };

        match op {
            0x00 => {
                // unreachable
                b.terminate(Term::Unreachable);
                frames.last_mut().unwrap().unreachable = true;
            }
            0x01 => {} // nop
            0x02 => {
                // block
                let (params, results) = block_type_tys(module, &insn.immediate)?;
                let height = stack.len() - params.len();
                let merge_bb = b.new_block(false)?;
                let carried = scan.take_locals(pc);
                let n_new = results.len().saturating_add(carried.len());
                if b.ir.val_tys.len().saturating_add(n_new) > MAX_IR_VALS_PER_FN {
                    return Err(format!(
                        "x64: function {func_index} SSA values exceed cap {MAX_IR_VALS_PER_FN}"
                    ));
                }
                for &rt in &results {
                    b.add_block_param(merge_bb, rt);
                }
                for &lx in &carried {
                    b.add_block_param(merge_bb, local_tys[lx as usize]);
                }
                frames.push(BuildFrame {
                    kind: FrameKind::Block,
                    height,
                    params_len: params.len(),
                    results,
                    target_bb: merge_bb,
                    carried_locals: carried,
                    else_bb: None,
                    if_entry_params: Vec::new(),
                    if_undo_mark: 0,
                    has_else: false,
                    unreachable: false,
                    try_state: None,
                });
            }
            0x03 => {
                // loop
                let (params, results) = block_type_tys(module, &insn.immediate)?;
                let height = stack.len() - params.len();
                let header_bb = b.new_block(true)?;
                let carried = scan.take_locals(pc);
                let n_new = params.len().saturating_add(carried.len());
                if b.ir.val_tys.len().saturating_add(n_new) > MAX_IR_VALS_PER_FN {
                    return Err(format!(
                        "x64: function {func_index} SSA values exceed cap {MAX_IR_VALS_PER_FN}"
                    ));
                }
                let mut entry_args = Vec::with_capacity(n_new);
                let mut header_param_vals = Vec::with_capacity(params.len());
                for (k, &pt) in params.iter().enumerate() {
                    entry_args.push(stack[height + k]);
                    header_param_vals.push(b.add_block_param(header_bb, pt));
                }
                let mut header_local_vals = Vec::with_capacity(carried.len());
                for &lx in &carried {
                    entry_args.push(locals[lx as usize]);
                    header_local_vals.push(b.add_block_param(header_bb, local_tys[lx as usize]));
                }
                let args_r = b.ir.push_pool(&entry_args)?;
                b.terminate(Term::Jump {
                    target: header_bb,
                    args: args_r,
                });
                b.add_pred(header_bb);
                b.cur_bb = header_bb;
                for (k, v) in header_param_vals.into_iter().enumerate() {
                    stack[height + k] = v;
                }
                for (&lx, v) in carried.iter().zip(header_local_vals) {
                    set_local(
                        &mut locals,
                        &mut local_undo,
                        active_if_then_depth,
                        &mut total_if_snapshot_entries,
                        lx as usize,
                        v,
                    )?;
                }
                frames.push(BuildFrame {
                    kind: FrameKind::Loop,
                    height,
                    params_len: params.len(),
                    results,
                    target_bb: header_bb,
                    carried_locals: carried,
                    else_bb: None,
                    if_entry_params: Vec::new(),
                    if_undo_mark: 0,
                    has_else: false,
                    unreachable: false,
                    try_state: None,
                });
            }
            0x04 => {
                // if
                let (params, results) = block_type_tys(module, &insn.immediate)?;
                let cond = stack.pop().ok_or("if cond underflow")?;
                let height = stack.len() - params.len();
                let then_bb = b.new_block(false)?;
                let else_bb = b.new_block(false)?;
                let merge_bb = b.new_block(false)?;
                let carried = scan.take_locals(pc);
                let n_new = results.len().saturating_add(carried.len());
                if b.ir.val_tys.len().saturating_add(n_new) > MAX_IR_VALS_PER_FN {
                    return Err(format!(
                        "x64: function {func_index} SSA values exceed cap {MAX_IR_VALS_PER_FN}"
                    ));
                }
                for &rt in &results {
                    b.add_block_param(merge_bb, rt);
                }
                for &lx in &carried {
                    b.add_block_param(merge_bb, local_tys[lx as usize]);
                }
                b.terminate(Term::BranchIf {
                    cond,
                    then_bb,
                    then_args: ValRange::EMPTY,
                    else_bb,
                    else_args: ValRange::EMPTY,
                });
                b.add_pred(then_bb);
                b.add_pred(else_bb);
                b.cur_bb = then_bb;
                let param_entries = stack.len() - height;
                total_if_snapshot_entries = total_if_snapshot_entries.saturating_add(param_entries);
                if total_if_snapshot_entries > MAX_IF_SNAPSHOT_ENTRIES {
                    return Err(format!(
                        "x64: function {func_index} if-nesting snapshot budget ({total_if_snapshot_entries}) exceeds cap {MAX_IF_SNAPSHOT_ENTRIES}"
                    ));
                }
                let if_entry_params = stack[height..].to_vec();
                let if_undo_mark = local_undo.len();
                active_if_then_depth += 1;
                frames.push(BuildFrame {
                    kind: FrameKind::If,
                    height,
                    params_len: params.len(),
                    results,
                    target_bb: merge_bb,
                    carried_locals: carried,
                    else_bb: Some(else_bb),
                    if_entry_params,
                    if_undo_mark,
                    has_else: false,
                    unreachable: false,
                    try_state: None,
                });
            }
            0x1f => {
                // try_table
                let Immediate::TryTable(ref tt) = insn.immediate else {
                    return Err("try_table immediate".into());
                };
                let (params, results) = block_type_tys(module, &tt.block)?;
                let height = stack.len() - params.len();
                let merge_bb = b.new_block(false)?;
                let catch_bb = b.new_block(false)?;
                let carried = scan.take_locals(pc);
                let n_new = results
                    .len()
                    .saturating_add(carried.len().saturating_mul(2));
                if b.ir.val_tys.len().saturating_add(n_new) > MAX_IR_VALS_PER_FN {
                    return Err(format!(
                        "x64: function {func_index} SSA values exceed cap {MAX_IR_VALS_PER_FN}"
                    ));
                }
                for &rt in &results {
                    b.add_block_param(merge_bb, rt);
                }
                for &lx in &carried {
                    b.add_block_param(merge_bb, local_tys[lx as usize]);
                    b.add_block_param(catch_bb, local_tys[lx as usize]);
                }
                let catches = tt
                    .catches
                    .iter()
                    .map(|c| (c.kind, c.tag, c.label))
                    .collect();
                frames.push(BuildFrame {
                    kind: FrameKind::TryTable,
                    height,
                    params_len: params.len(),
                    results,
                    target_bb: merge_bb,
                    carried_locals: carried,
                    else_bb: None,
                    if_entry_params: Vec::new(),
                    if_undo_mark: 0,
                    has_else: false,
                    unreachable: false,
                    try_state: Some(Box::new(TryTableState {
                        catch_bb,
                        catches,
                        entry_locals: locals.clone(),
                    })),
                });
            }
            0x08 => {
                // throw tag
                let tag = imm_idx()?;
                let fti = *module
                    .tags
                    .get(tag as usize)
                    .ok_or("throw tag out of range")?;
                let tag_ty = module
                    .module_types
                    .get(fti as usize)
                    .ok_or("throw tag type")?;
                let n = tag_ty.params.len();
                let at = stack.len() - n;
                let args = b.ir.push_pool(&stack[at..])?;
                stack.truncate(at);
                let h = b.emit(Ty::I64, Op::ExnAlloc { tag, args });
                b.emit_void(Op::ExnSetActive(h));
                if let Some(tf) = frames.iter().rev().find(|f| f.kind == FrameKind::TryTable) {
                    let catch_bb = tf.try_state.as_ref().unwrap().catch_bb;
                    let mut cargs = Vec::with_capacity(tf.carried_locals.len());
                    for &lx in &tf.carried_locals {
                        cargs.push(locals[lx as usize]);
                    }
                    let vr = b.ir.push_pool(&cargs)?;
                    b.terminate(Term::Jump {
                        target: catch_bb,
                        args: vr,
                    });
                    b.add_pred(catch_bb);
                } else {
                    b.terminate(Term::ThrowActive);
                }
                frames.last_mut().unwrap().unreachable = true;
            }
            0x0a => {
                // throw_ref
                let exn_val = stack.pop().ok_or("throw_ref val")?;
                let trap_bb = b.new_block(false)?;
                let throw_bb = b.new_block(false)?;
                let is_null = b.emit(Ty::I32, Op::I64Eqz(exn_val));
                b.terminate(Term::BranchIf {
                    cond: is_null,
                    then_bb: trap_bb,
                    then_args: ValRange::EMPTY,
                    else_bb: throw_bb,
                    else_args: ValRange::EMPTY,
                });
                b.add_pred(trap_bb);
                b.add_pred(throw_bb);
                b.cur_bb = trap_bb;
                b.emit_void(Op::Trap(TRAP_INDIRECT_CALL));
                b.terminate(Term::Unreachable);
                b.cur_bb = throw_bb;
                b.emit_void(Op::ExnSetActive(exn_val));
                if let Some(tf) = frames.iter().rev().find(|f| f.kind == FrameKind::TryTable) {
                    let catch_bb = tf.try_state.as_ref().unwrap().catch_bb;
                    let mut cargs = Vec::with_capacity(tf.carried_locals.len());
                    for &lx in &tf.carried_locals {
                        cargs.push(locals[lx as usize]);
                    }
                    let vr = b.ir.push_pool(&cargs)?;
                    b.terminate(Term::Jump {
                        target: catch_bb,
                        args: vr,
                    });
                    b.add_pred(catch_bb);
                } else {
                    b.terminate(Term::ThrowActive);
                }
                frames.last_mut().unwrap().unreachable = true;
            }
            0x05 => {
                // else
                let f = frames.last().ok_or("else without if")?;
                if !f.unreachable {
                    let args_r = build_branch_args(&mut b, &stack, &locals, f)?;
                    let target = f.target_bb;
                    b.terminate(Term::Jump {
                        target,
                        args: args_r,
                    });
                    b.add_pred(target);
                }
                let f = frames.last_mut().unwrap();
                debug_assert!(!f.has_else && active_if_then_depth > 0);
                f.has_else = true;
                f.unreachable = false;
                dead = 0;
                b.cur_bb = f.else_bb.unwrap();
                stack.truncate(f.height);
                stack.extend_from_slice(&f.if_entry_params);
                active_if_then_depth -= 1;
                while local_undo.len() > f.if_undo_mark {
                    let (lx, old_val) = local_undo.pop().unwrap();
                    locals[lx as usize] = old_val;
                }
            }
            0x0b => {
                // end
                let f = frames.pop().ok_or("end without frame")?;
                match f.kind {
                    FrameKind::Function => {
                        if !f.unreachable {
                            let base = stack.len() - f.results.len();
                            let vals = b.ir.push_pool(&stack[base..])?;
                            b.terminate(Term::Return { vals });
                        }
                        break;
                    }
                    FrameKind::Loop => {
                        if f.unreachable {
                            frames.last_mut().unwrap().unreachable = true;
                        } else {
                            let res_vals = stack[stack.len() - f.results.len()..].to_vec();
                            stack.truncate(f.height);
                            stack.extend(res_vals);
                        }
                    }
                    FrameKind::Block | FrameKind::If | FrameKind::TryTable => {
                        if !f.unreachable {
                            let args_r = build_branch_args(&mut b, &stack, &locals, &f)?;
                            let target = f.target_bb;
                            b.terminate(Term::Jump {
                                target,
                                args: args_r,
                            });
                            b.add_pred(target);
                        }
                        if let Some(ts) = f.try_state
                            && b.ir.blocks[ts.catch_bb.0 as usize].preds > 0
                        {
                            // Build catch dispatcher using `catch_locals` without mutating `locals`/`local_undo`.
                            let mut catch_locals = ts.entry_locals;
                            let cparams = b.ir.blocks[ts.catch_bb.0 as usize].params.clone();
                            for (&lx, &lv) in f.carried_locals.iter().zip(&cparams) {
                                catch_locals[lx as usize] = lv;
                            }
                            b.cur_bb = ts.catch_bb;
                            let exn = b.emit(Ty::I64, Op::ExnGetActive);
                            let exn_tag = if ts.catches.iter().any(|(_, t, _)| t.is_some()) {
                                Some(b.emit(Ty::I32, Op::ExnGetTag(exn)))
                            } else {
                                None
                            };
                            let mut matched_all = false;
                            for &(kind, tag_opt, label) in &ts.catches {
                                let match_bb = b.new_block(false)?;
                                let next_bb = if let Some(tag_idx) = tag_opt {
                                    let nbb = b.new_block(false)?;
                                    let tc = b.const_i32(tag_idx as i32);
                                    let eq = b.emit_i32_cmp(CmpOp::Eq, exn_tag.unwrap(), tc);
                                    b.terminate(Term::BranchIf {
                                        cond: eq,
                                        then_bb: match_bb,
                                        then_args: ValRange::EMPTY,
                                        else_bb: nbb,
                                        else_args: ValRange::EMPTY,
                                    });
                                    b.add_pred(match_bb);
                                    b.add_pred(nbb);
                                    Some(nbb)
                                } else {
                                    b.terminate(Term::Jump {
                                        target: match_bb,
                                        args: ValRange::EMPTY,
                                    });
                                    b.add_pred(match_bb);
                                    None
                                };
                                b.cur_bb = match_bb;
                                b.emit_void(Op::ExnClearActive);
                                let mut vals = Vec::new();
                                if let Some(tag_idx) = tag_opt {
                                    let fti = module.tags[tag_idx as usize] as usize;
                                    for (field_i, &vt) in
                                        module.module_types[fti].params.iter().enumerate()
                                    {
                                        let ty = Ty::from_val(vt);
                                        vals.push(b.emit(
                                            ty,
                                            Op::ExnGetPayload {
                                                handle: exn,
                                                tag: tag_idx,
                                                field: field_i as u32,
                                                ty,
                                            },
                                        ));
                                    }
                                }
                                if kind == 1 || kind == 3 {
                                    vals.push(exn);
                                }
                                let tfi = frames.len() - 1 - label as usize;
                                let tf = &frames[tfi];
                                if tf.kind == FrameKind::Function {
                                    let vr = b.ir.push_pool(&vals)?;
                                    b.terminate(Term::Return { vals: vr });
                                } else {
                                    for &lx in &tf.carried_locals {
                                        vals.push(catch_locals[lx as usize]);
                                    }
                                    let vr = b.ir.push_pool(&vals)?;
                                    let tgt = tf.target_bb;
                                    b.terminate(Term::Jump {
                                        target: tgt,
                                        args: vr,
                                    });
                                    b.add_pred(tgt);
                                }
                                if let Some(nbb) = next_bb {
                                    b.cur_bb = nbb;
                                } else {
                                    matched_all = true;
                                    break;
                                }
                            }
                            if !matched_all {
                                if let Some(enc) = frames
                                    .iter()
                                    .rev()
                                    .find(|fr| fr.kind == FrameKind::TryTable)
                                {
                                    let enc_bb = enc.try_state.as_ref().unwrap().catch_bb;
                                    let mut enc_args = Vec::with_capacity(enc.carried_locals.len());
                                    for &lx in &enc.carried_locals {
                                        enc_args.push(catch_locals[lx as usize]);
                                    }
                                    let vr = b.ir.push_pool(&enc_args)?;
                                    b.terminate(Term::Jump {
                                        target: enc_bb,
                                        args: vr,
                                    });
                                    b.add_pred(enc_bb);
                                } else {
                                    b.terminate(Term::ThrowActive);
                                }
                            }
                        }
                        if f.kind == FrameKind::If && !f.has_else {
                            // Unwind `then`-branch local mutations back to exact `if`-entry state,
                            // then emit implicit else -> merge_bb edge carrying entry params & locals.
                            debug_assert!(active_if_then_depth > 0);
                            active_if_then_depth -= 1;
                            while local_undo.len() > f.if_undo_mark {
                                let (lx, old_val) = local_undo.pop().unwrap();
                                locals[lx as usize] = old_val;
                            }
                            b.cur_bb = f.else_bb.unwrap();
                            let mut args = Vec::with_capacity(
                                f.if_entry_params.len() + f.carried_locals.len(),
                            );
                            args.extend_from_slice(&f.if_entry_params);
                            for &lx in &f.carried_locals {
                                args.push(locals[lx as usize]);
                            }
                            let args_r = b.ir.push_pool(&args)?;
                            b.terminate(Term::Jump {
                                target: f.target_bb,
                                args: args_r,
                            });
                            b.add_pred(f.target_bb);
                        }
                        if b.ir.blocks[f.target_bb.0 as usize].preds == 0 {
                            frames.last_mut().unwrap().unreachable = true;
                        } else {
                            b.cur_bb = f.target_bb;
                            stack.truncate(f.height);
                            let mparams = b.ir.blocks[f.target_bb.0 as usize].params.clone();
                            let n_res = f.results.len();
                            for &rv in &mparams[..n_res] {
                                stack.push(rv);
                            }
                            for (&lx, &lv) in f.carried_locals.iter().zip(&mparams[n_res..]) {
                                set_local(
                                    &mut locals,
                                    &mut local_undo,
                                    active_if_then_depth,
                                    &mut total_if_snapshot_entries,
                                    lx as usize,
                                    lv,
                                )?;
                            }
                        }
                    }
                }
            }
            0x0c => {
                // br
                let depth = imm_idx()? as usize;
                let fi = frames.len() - 1 - depth;
                let f = &frames[fi];
                if f.kind == FrameKind::Function {
                    let base = stack.len() - f.results.len();
                    let vals = b.ir.push_pool(&stack[base..])?;
                    b.terminate(Term::Return { vals });
                } else {
                    let args_r = build_branch_args(&mut b, &stack, &locals, f)?;
                    let target = f.target_bb;
                    b.terminate(Term::Jump {
                        target,
                        args: args_r,
                    });
                    b.add_pred(target);
                }
                frames.last_mut().unwrap().unreachable = true;
            }
            0x0d => {
                // br_if
                let depth = imm_idx()? as usize;
                let cond = stack.pop().ok_or("br_if cond")?;
                let fi = frames.len() - 1 - depth;
                let fall_bb = b.new_block(false)?;
                if frames[fi].kind == FrameKind::Function {
                    let ret_bb = b.new_block(false)?;
                    let n_res = frames[fi].results.len();
                    let ret_vals = stack[stack.len() - n_res..].to_vec();
                    b.terminate(Term::BranchIf {
                        cond,
                        then_bb: ret_bb,
                        then_args: ValRange::EMPTY,
                        else_bb: fall_bb,
                        else_args: ValRange::EMPTY,
                    });
                    b.add_pred(ret_bb);
                    b.add_pred(fall_bb);
                    b.cur_bb = ret_bb;
                    let vr = b.ir.push_pool(&ret_vals)?;
                    b.terminate(Term::Return { vals: vr });
                    b.cur_bb = fall_bb;
                } else {
                    let args_r = build_branch_args(&mut b, &stack, &locals, &frames[fi])?;
                    let target = frames[fi].target_bb;
                    b.terminate(Term::BranchIf {
                        cond,
                        then_bb: target,
                        then_args: args_r,
                        else_bb: fall_bb,
                        else_args: ValRange::EMPTY,
                    });
                    b.add_pred(target);
                    b.add_pred(fall_bb);
                    b.cur_bb = fall_bb;
                }
            }
            0x0e => {
                // br_table
                let Immediate::BrTable(ref raw_targets, raw_default) = insn.immediate else {
                    return Err("br_table immediate".into());
                };
                let idx = stack.pop().ok_or("br_table idx")?;
                let mut ret_bb_cache: Option<BlockId> = None;
                let mut depth_cache: HashMap<u32, (BlockId, ValRange)> = HashMap::new();
                let mut unique_edges: Vec<(BlockId, ValRange)> = Vec::new();
                let mut resolve_depth = |b: &mut Builder, d: u32| -> Res<(BlockId, ValRange)> {
                    if let Some(&cached) = depth_cache.get(&d) {
                        return Ok(cached);
                    }
                    let fi = frames.len() - 1 - d as usize;
                    let f = &frames[fi];
                    let resolved = if f.kind == FrameKind::Function {
                        let rbb = match ret_bb_cache {
                            Some(bb) => bb,
                            None => {
                                let bb = b.new_block(false)?;
                                let saved = b.cur_bb;
                                b.cur_bb = bb;
                                let base = stack.len() - f.results.len();
                                let vr = b.ir.push_pool(&stack[base..])?;
                                b.terminate(Term::Return { vals: vr });
                                b.cur_bb = saved;
                                ret_bb_cache = Some(bb);
                                bb
                            }
                        };
                        b.add_pred(rbb);
                        (rbb, ValRange::EMPTY)
                    } else {
                        let args_r = build_branch_args(b, &stack, &locals, f)?;
                        b.add_pred(f.target_bb);
                        (f.target_bb, args_r)
                    };
                    depth_cache.insert(d, resolved);
                    unique_edges.push(resolved);
                    Ok(resolved)
                };
                let mut targets = Vec::with_capacity(raw_targets.len());
                for &d in raw_targets.iter() {
                    targets.push(resolve_depth(&mut b, d)?);
                }
                let default = resolve_depth(&mut b, raw_default)?;
                let tid = b.ir.br_tables.len() as u32;
                b.ir.br_tables.push(BrTableData {
                    targets,
                    default,
                    unique_edges,
                });
                b.terminate(Term::BrTable {
                    idx,
                    table_idx: tid,
                });
                frames.last_mut().unwrap().unreachable = true;
            }
            0x0f => {
                // return
                let n_res = frames[0].results.len();
                let base = stack.len() - n_res;
                let vals = b.ir.push_pool(&stack[base..])?;
                b.terminate(Term::Return { vals });
                frames.last_mut().unwrap().unreachable = true;
            }
            0x10 => {
                // call
                let f = imm_idx()? as usize;
                let fty = module.func_type(f).ok_or("call func index")?;
                let n = fty.params.len();
                let at = stack.len() - n;
                let args = b.ir.push_pool(&stack[at..])?;
                stack.truncate(at);
                if b.ir.val_tys.len().saturating_add(fty.results.len()) > MAX_IR_VALS_PER_FN {
                    return Err(format!(
                        "x64: function {func_index} SSA values exceed cap {MAX_IR_VALS_PER_FN}"
                    ));
                }
                if let Some(tf) = frames
                    .iter()
                    .rev()
                    .find(|fr| fr.kind == FrameKind::TryTable)
                {
                    let catch_bb = tf.try_state.as_ref().unwrap().catch_bb;
                    let mut cargs = Vec::with_capacity(tf.carried_locals.len());
                    for &lx in &tf.carried_locals {
                        cargs.push(locals[lx as usize]);
                    }
                    let catch_args = b.ir.push_pool(&cargs)?;
                    let ok_bb = b.new_block(false)?;
                    let mut rets_vec = Vec::with_capacity(fty.results.len());
                    for &rt in &fty.results {
                        rets_vec.push(b.add_block_param(ok_bb, Ty::from_val(rt)));
                    }
                    b.terminate(Term::CallCatch {
                        func: f as u32,
                        args,
                        ok_bb,
                        catch_bb,
                        catch_args,
                    });
                    b.add_pred(ok_bb);
                    b.add_pred(catch_bb);
                    b.cur_bb = ok_bb;
                    stack.extend(rets_vec);
                } else {
                    let mut rets_vec = Vec::with_capacity(fty.results.len());
                    for &rt in &fty.results {
                        let v = b.ir.push_val(Ty::from_val(rt));
                        b.consts.push(None);
                        rets_vec.push(v);
                    }
                    let rets = b.ir.push_pool(&rets_vec)?;
                    b.emit_void(Op::Call {
                        func: f as u32,
                        args,
                        rets,
                    });
                    stack.extend(rets_vec);
                }
            }
            0x11 => {
                // call_indirect
                let Immediate::Pair(ti, tbl) = insn.immediate else {
                    return Err("call_indirect immediate".into());
                };
                let &ci_id = module
                    .ci_index
                    .get(&(tbl, ti))
                    .ok_or("call_indirect not registered")?;
                let fty = module
                    .module_types
                    .get(ti as usize)
                    .ok_or("call_indirect type")?;
                let idx = stack.pop().ok_or("ci idx")?;
                let n = fty.params.len();
                let at = stack.len() - n;
                let args = b.ir.push_pool(&stack[at..])?;
                stack.truncate(at);
                if b.ir.val_tys.len().saturating_add(fty.results.len()) > MAX_IR_VALS_PER_FN {
                    return Err(format!(
                        "x64: function {func_index} SSA values exceed cap {MAX_IR_VALS_PER_FN}"
                    ));
                }
                if let Some(tf) = frames
                    .iter()
                    .rev()
                    .find(|fr| fr.kind == FrameKind::TryTable)
                {
                    let catch_bb = tf.try_state.as_ref().unwrap().catch_bb;
                    let mut cargs = Vec::with_capacity(tf.carried_locals.len());
                    for &lx in &tf.carried_locals {
                        cargs.push(locals[lx as usize]);
                    }
                    let catch_args = b.ir.push_pool(&cargs)?;
                    let ok_bb = b.new_block(false)?;
                    let mut rets_vec = Vec::with_capacity(fty.results.len());
                    for &rt in &fty.results {
                        rets_vec.push(b.add_block_param(ok_bb, Ty::from_val(rt)));
                    }
                    b.terminate(Term::CallIndirectCatch {
                        ci_id: ci_id as u32,
                        idx,
                        args,
                        ok_bb,
                        catch_bb,
                        catch_args,
                    });
                    b.add_pred(ok_bb);
                    b.add_pred(catch_bb);
                    b.cur_bb = ok_bb;
                    stack.extend(rets_vec);
                } else {
                    let mut rets_vec = Vec::with_capacity(fty.results.len());
                    for &rt in &fty.results {
                        let v = b.ir.push_val(Ty::from_val(rt));
                        b.consts.push(None);
                        rets_vec.push(v);
                    }
                    let rets = b.ir.push_pool(&rets_vec)?;
                    b.emit_void(Op::CallIndirect {
                        ci_id: ci_id as u32,
                        idx,
                        args,
                        rets,
                    });
                    stack.extend(rets_vec);
                }
            }
            0x12 => {
                // return_call
                let f = imm_idx()? as usize;
                let fty = module.func_type(f).ok_or("return_call func index")?;
                let n = fty.params.len();
                let at = stack.len() - n;
                let args = b.ir.push_pool(&stack[at..])?;
                stack.truncate(at);
                b.terminate(Term::ReturnCall {
                    func: f as u32,
                    args,
                });
                frames.last_mut().unwrap().unreachable = true;
            }
            0x13 => {
                // return_call_indirect
                let Immediate::Pair(ti, tbl) = insn.immediate else {
                    return Err("return_call_indirect immediate".into());
                };
                let &ci_id = module
                    .ci_index
                    .get(&(tbl, ti))
                    .ok_or("return_call_indirect not registered")?;
                let fty = module
                    .module_types
                    .get(ti as usize)
                    .ok_or("return_call_indirect type")?;
                let idx = stack.pop().ok_or("return_ci idx")?;
                let n = fty.params.len();
                let at = stack.len() - n;
                let args = b.ir.push_pool(&stack[at..])?;
                stack.truncate(at);
                b.terminate(Term::ReturnCallIndirect {
                    ci_id: ci_id as u32,
                    idx,
                    args,
                });
                frames.last_mut().unwrap().unreachable = true;
            }
            0x1a => {
                stack.pop().ok_or("drop")?;
            }
            0x1b | 0x1c => {
                let cond = stack.pop().ok_or("select c")?;
                let bv = stack.pop().ok_or("select b")?;
                let av = stack.pop().ok_or("select a")?;
                if let Some(c) = b.as_i32(cond) {
                    stack.push(if c != 0 { av } else { bv });
                } else if b.const_fold && av == bv {
                    stack.push(av);
                } else {
                    let t = b.ir.val_tys[av.0 as usize];
                    let r = b.emit(t, Op::Select { cond, a: av, b: bv });
                    stack.push(r);
                }
            }
            0x20 => {
                let x = imm_idx()? as usize;
                stack.push(locals[x]);
            }
            0x21 => {
                let x = imm_idx()? as usize;
                let v = stack.pop().ok_or("local.set")?;
                set_local(
                    &mut locals,
                    &mut local_undo,
                    active_if_then_depth,
                    &mut total_if_snapshot_entries,
                    x,
                    v,
                )?;
            }
            0x22 => {
                let x = imm_idx()? as usize;
                let v = *stack.last().ok_or("local.tee")?;
                set_local(
                    &mut locals,
                    &mut local_undo,
                    active_if_then_depth,
                    &mut total_if_snapshot_entries,
                    x,
                    v,
                )?;
            }
            0x23 => {
                let x = imm_idx()? as usize;
                let (vt, _) = module.globals[x];
                let v = if let Some(c) = module.global_consts[x] {
                    match c {
                        X64ConstVal::I32(k) => b.const_i32(k),
                        X64ConstVal::I64(k) => b.const_i64(k),
                        X64ConstVal::F32(bits) => b.emit(Ty::F32, Op::ConstF32(bits)),
                        X64ConstVal::F64(bits) => b.emit(Ty::F64, Op::ConstF64(bits)),
                        X64ConstVal::V128(bytes) => {
                            let idx = b.ir.v128_consts.len() as u32;
                            b.ir.v128_consts.push(bytes);
                            b.emit(Ty::V128, Op::ConstV128(idx))
                        }
                        X64ConstVal::RefNull => b.const_i64(0),
                        X64ConstVal::FuncRef(f) => b.const_i64(i64::from(f) + 1),
                    }
                } else {
                    b.emit(Ty::from_val(vt), Op::GlobalGet(x as u32))
                };
                stack.push(v);
            }
            0x24 => {
                let x = imm_idx()?;
                let v = stack.pop().ok_or("global.set")?;
                if module
                    .global_consts
                    .get(x as usize)
                    .and_then(|c| *c)
                    .is_none()
                {
                    b.emit_void(Op::GlobalSet(x, v));
                }
            }
            0x25 => {
                let t = imm_idx()?;
                if !module.mut_extern_tables.contains_key(&t) && !module.dyn_tables.contains_key(&t)
                {
                    return Err(format!("table.get on unsupported table {t}"));
                }
                let idx = stack.pop().ok_or("table.get idx")?;
                let r = b.emit(Ty::I64, Op::TableGet { table: t, idx });
                stack.push(r);
            }
            0x26 => {
                let t = imm_idx()?;
                if !module.mut_extern_tables.contains_key(&t) && !module.dyn_tables.contains_key(&t)
                {
                    return Err(format!("table.set on unsupported table {t}"));
                }
                let val = stack.pop().ok_or("table.set val")?;
                let idx = stack.pop().ok_or("table.set idx")?;
                b.emit_void(Op::TableSet { table: t, idx, val });
            }
            0xd0 => {
                let v = b.const_i64(0);
                stack.push(v);
            }
            0xd1 => {
                let v = stack.pop().ok_or("ref.is_null")?;
                let r = b.emit(Ty::I32, Op::I64Eqz(v));
                stack.push(r);
            }
            0xd2 => {
                let f = imm_idx()?;
                let v = b.const_i64(i64::from(f) + 1);
                stack.push(v);
            }
            0x28..=0x35 | 0xfd00..=0xfd0a | 0xfd5c | 0xfd5d => {
                let Immediate::MemArg { memory, offset, .. } = insn.immediate else {
                    return Err("load on unsupported memory".into());
                };
                let mem64 = module.mem_is_64(memory);
                let addr = stack.pop().ok_or("load addr")?;
                let kind = match op {
                    0x28 => LoadKind::I32,
                    0x29 => LoadKind::I64,
                    0x2a => LoadKind::F32,
                    0x2b => LoadKind::F64,
                    0x2c => LoadKind::I32Load8S,
                    0x2d => LoadKind::I32Load8U,
                    0x2e => LoadKind::I32Load16S,
                    0x2f => LoadKind::I32Load16U,
                    0x30 => LoadKind::I64Load8S,
                    0x31 => LoadKind::I64Load8U,
                    0x32 => LoadKind::I64Load16S,
                    0x33 => LoadKind::I64Load16U,
                    0x34 => LoadKind::I64Load32S,
                    0x35 => LoadKind::I64Load32U,
                    0xfd00 => LoadKind::V128,
                    0xfd01 => LoadKind::V128Load8x8S,
                    0xfd02 => LoadKind::V128Load8x8U,
                    0xfd03 => LoadKind::V128Load16x4S,
                    0xfd04 => LoadKind::V128Load16x4U,
                    0xfd05 => LoadKind::V128Load32x2S,
                    0xfd06 => LoadKind::V128Load32x2U,
                    0xfd07 => LoadKind::V128Load8Splat,
                    0xfd08 => LoadKind::V128Load16Splat,
                    0xfd09 => LoadKind::V128Load32Splat,
                    0xfd0a => LoadKind::V128Load64Splat,
                    0xfd5c => LoadKind::V128Load32Zero,
                    _ => LoadKind::V128Load64Zero,
                };
                if (!mem64 && offset > u32::MAX as u64)
                    || offset.checked_add(kind.bytes()).is_none()
                {
                    b.emit_void(Op::Trap(TRAP_OOB));
                    b.terminate(Term::Unreachable);
                    frames.last_mut().unwrap().unreachable = true;
                    continue;
                }
                let r = if memory == 0 {
                    b.emit(
                        kind.ty(),
                        Op::Load {
                            kind,
                            addr,
                            offset,
                            checked: false,
                        },
                    )
                } else {
                    b.emit(
                        kind.ty(),
                        Op::LoadMem {
                            mem: memory,
                            kind,
                            addr,
                            offset,
                        },
                    )
                };
                stack.push(r);
            }
            0x36..=0x3e | 0xfd0b => {
                let Immediate::MemArg { memory, offset, .. } = insn.immediate else {
                    return Err("store on unsupported memory".into());
                };
                let mem64 = module.mem_is_64(memory);
                let val = stack.pop().ok_or("store val")?;
                let addr = stack.pop().ok_or("store addr")?;
                let kind = match op {
                    0x36 | 0x3e => StoreKind::I32,
                    0x37 => StoreKind::I64,
                    0x38 => StoreKind::F32,
                    0x39 => StoreKind::F64,
                    0x3a | 0x3c => StoreKind::I8,
                    0x3b | 0x3d => StoreKind::I16,
                    _ => StoreKind::V128,
                };
                if (!mem64 && offset > u32::MAX as u64)
                    || offset.checked_add(kind.bytes()).is_none()
                {
                    b.emit_void(Op::Trap(TRAP_OOB));
                    b.terminate(Term::Unreachable);
                    frames.last_mut().unwrap().unreachable = true;
                    continue;
                }
                if memory == 0 {
                    b.emit_void(Op::Store {
                        kind,
                        addr,
                        val,
                        offset,
                        checked: false,
                    });
                } else {
                    b.emit_void(Op::StoreMem {
                        mem: memory,
                        kind,
                        addr,
                        val,
                        offset,
                    });
                }
            }
            0xfd54..=0xfd5b => {
                let (memory, offset, lane) = match &insn.immediate {
                    Immediate::MemLane {
                        memory,
                        offset,
                        lane,
                        ..
                    } => (*memory, *offset, *lane),
                    Immediate::Bytes(bs) if bs.len() == 17 => {
                        let off = u64::from_le_bytes(bs[4..12].try_into().unwrap());
                        let mem = u32::from_le_bytes(bs[12..16].try_into().unwrap());
                        (mem, off, bs[16])
                    }
                    _ => return Err("simd lane load/store on unsupported memory".into()),
                };
                let mem64 = module.mem_is_64(memory);
                let width: u8 = match (op - 0xfd54) % 4 {
                    0 => 1,
                    1 => 2,
                    2 => 4,
                    _ => 8,
                };
                let vec = stack.pop().ok_or("simd lane vec")?;
                let addr = stack.pop().ok_or("simd lane addr")?;
                if (!mem64 && offset > u32::MAX as u64)
                    || offset.checked_add(u64::from(width)).is_none()
                {
                    b.emit_void(Op::Trap(TRAP_OOB));
                    b.terminate(Term::Unreachable);
                    frames.last_mut().unwrap().unreachable = true;
                    continue;
                }
                if op <= 0xfd57 {
                    let r = if memory == 0 {
                        b.emit(
                            Ty::V128,
                            Op::V128LoadLane {
                                width,
                                addr,
                                vec,
                                offset,
                                lane,
                                checked: false,
                            },
                        )
                    } else {
                        b.emit(
                            Ty::V128,
                            Op::V128LoadLaneMem {
                                mem: memory,
                                width,
                                addr,
                                vec,
                                offset,
                                lane,
                            },
                        )
                    };
                    stack.push(r);
                } else if memory == 0 {
                    b.emit_void(Op::V128StoreLane {
                        width,
                        addr,
                        vec,
                        offset,
                        lane,
                        checked: false,
                    });
                } else {
                    b.emit_void(Op::V128StoreLaneMem {
                        mem: memory,
                        width,
                        addr,
                        vec,
                        offset,
                        lane,
                    });
                }
            }
            0x3f => {
                let mem = match insn.immediate {
                    Immediate::Index(m) => m,
                    Immediate::None => 0,
                    _ => return Err("memory.size immediate".into()),
                };
                let mem64 = module.mem_is_64(mem);
                let addr_ty = if mem64 { Ty::I64 } else { Ty::I32 };
                let v = if mem == 0 {
                    if b.const_fold
                        && let Some(bytes) = module.fixed32
                    {
                        if mem64 {
                            b.const_i64((bytes >> 16) as i64)
                        } else {
                            b.const_i32((bytes >> 16) as i32)
                        }
                    } else {
                        b.emit(addr_ty, Op::MemSize)
                    }
                } else {
                    b.emit(addr_ty, Op::MemSizeIdx(mem))
                };
                stack.push(v);
            }
            0x40 => {
                let mem = match insn.immediate {
                    Immediate::Index(m) => m,
                    Immediate::None => 0,
                    _ => return Err("memory.grow immediate".into()),
                };
                let addr_ty = if module.mem_is_64(mem) {
                    Ty::I64
                } else {
                    Ty::I32
                };
                let delta = stack.pop().ok_or("memory.grow")?;
                let v = if mem == 0 {
                    b.emit(addr_ty, Op::MemGrow(delta))
                } else {
                    b.emit(addr_ty, Op::MemGrowIdx { mem, delta })
                };
                stack.push(v);
            }
            0x41 => {
                let Immediate::I32(v) = insn.immediate else {
                    return Err("i32.const".into());
                };
                let r = b.const_i32(v);
                stack.push(r);
            }
            0x42 => {
                let Immediate::I64(v) = insn.immediate else {
                    return Err("i64.const".into());
                };
                let r = b.const_i64(v);
                stack.push(r);
            }
            0x43 => {
                let Immediate::F32(v) = insn.immediate else {
                    return Err("f32.const".into());
                };
                let r = b.emit(Ty::F32, Op::ConstF32(v));
                stack.push(r);
            }
            0x44 => {
                let Immediate::F64(v) = insn.immediate else {
                    return Err("f64.const".into());
                };
                let r = b.emit(Ty::F64, Op::ConstF64(v));
                stack.push(r);
            }
            0xfc08 => {
                let Immediate::Pair(data, mem) = insn.immediate else {
                    return Err("memory.init immediate".into());
                };
                let len = stack.pop().ok_or("minit len")?;
                let src = stack.pop().ok_or("minit src")?;
                let dst = stack.pop().ok_or("minit dst")?;
                b.emit_void(Op::MemInit {
                    data,
                    mem,
                    dst,
                    src,
                    len,
                });
            }
            0xfc09 => {
                let data = imm_idx()?;
                b.emit_void(Op::DataDrop(data));
            }
            0xfc0a => {
                let (dst_mem, src_mem) = match insn.immediate {
                    Immediate::Pair(d, s) => (d, s),
                    Immediate::None => (0, 0),
                    _ => return Err("memory.copy immediate".into()),
                };
                let len = stack.pop().ok_or("mcopy len")?;
                let src = stack.pop().ok_or("mcopy src")?;
                let dst = stack.pop().ok_or("mcopy dst")?;
                if dst_mem == 0 && src_mem == 0 {
                    let const_len = if module.memory64 {
                        b.as_i64(len).map(|c| c as u64)
                    } else {
                        b.as_i32(len).map(|c| u64::from(c as u32))
                    };
                    if let Some(c) = const_len
                        && c <= 64
                    {
                        b.emit_void(Op::MemCopyConst {
                            dst,
                            src,
                            len: c as u32,
                            dst_checked: false,
                            src_checked: false,
                        });
                    } else {
                        b.emit_void(Op::MemCopy { dst, src, len });
                    }
                } else {
                    b.emit_void(Op::MemCopyIdx {
                        dst_mem,
                        src_mem,
                        dst,
                        src,
                        len,
                    });
                }
            }
            0xfc0b => {
                let mem = match insn.immediate {
                    Immediate::Index(m) => m,
                    Immediate::None => 0,
                    _ => return Err("memory.fill immediate".into()),
                };
                let len = stack.pop().ok_or("mfill len")?;
                let val = stack.pop().ok_or("mfill val")?;
                let dst = stack.pop().ok_or("mfill dst")?;
                if mem == 0 {
                    let const_len = if module.memory64 {
                        b.as_i64(len).map(|c| c as u64)
                    } else {
                        b.as_i32(len).map(|c| u64::from(c as u32))
                    };
                    if let Some(c) = const_len
                        && c <= 64
                    {
                        b.emit_void(Op::MemFillConst {
                            dst,
                            val,
                            len: c as u32,
                            checked: false,
                        });
                    } else {
                        b.emit_void(Op::MemFill { dst, val, len });
                    }
                } else {
                    b.emit_void(Op::MemFillIdx { mem, dst, val, len });
                }
            }
            0xfc0c => {
                let Immediate::Pair(elem, table) = insn.immediate else {
                    return Err("table.init immediate".into());
                };
                let len = stack.pop().ok_or("tinit len")?;
                let src = stack.pop().ok_or("tinit src")?;
                let dst = stack.pop().ok_or("tinit dst")?;
                b.emit_void(Op::TableInit {
                    elem,
                    table,
                    dst,
                    src,
                    len,
                });
            }
            0xfc0d => {
                let elem = imm_idx()?;
                b.emit_void(Op::ElemDrop(elem));
            }
            0xfc0e => {
                let Immediate::Pair(dst_table, src_table) = insn.immediate else {
                    return Err("table.copy immediate".into());
                };
                let len = stack.pop().ok_or("tcopy len")?;
                let src = stack.pop().ok_or("tcopy src")?;
                let dst = stack.pop().ok_or("tcopy dst")?;
                b.emit_void(Op::TableCopy {
                    dst_table,
                    src_table,
                    dst,
                    src,
                    len,
                });
            }
            0xfc0f => {
                let table = imm_idx()?;
                let delta = stack.pop().ok_or("tgrow delta")?;
                let init = stack.pop().ok_or("tgrow init")?;
                let r = b.emit(Ty::I32, Op::TableGrow { table, init, delta });
                stack.push(r);
            }
            0xfc10 => {
                let table = imm_idx()?;
                let r = if b.const_fold && !module.dyn_tables.contains_key(&table) {
                    let len = *module
                        .table_lens
                        .get(table as usize)
                        .ok_or("table.size index")?;
                    b.const_i32(len as i32)
                } else {
                    b.emit(Ty::I32, Op::TableSize(table))
                };
                stack.push(r);
            }
            0xfc11 => {
                let table = imm_idx()?;
                let len = stack.pop().ok_or("tfill len")?;
                let val = stack.pop().ok_or("tfill val")?;
                let dst = stack.pop().ok_or("tfill dst")?;
                b.emit_void(Op::TableFill {
                    table,
                    dst,
                    val,
                    len,
                });
            }
            0xfc13 => {
                // i64.add128(a_lo, a_hi, b_lo, b_hi) -> (lo, hi)
                let b_hi = stack.pop().ok_or("i64.add128 b_hi")?;
                let b_lo = stack.pop().ok_or("i64.add128 b_lo")?;
                let a_hi = stack.pop().ok_or("i64.add128 a_hi")?;
                let a_lo = stack.pop().ok_or("i64.add128 a_lo")?;
                let lo = b.emit_i64_bin(IntBinOp::Add, a_lo, b_lo);
                let carry32 = b.emit_i64_cmp(CmpOp::LtU, lo, a_lo);
                let carry64 = if let Some(c) = b.as_i32(carry32) {
                    b.const_i64(i64::from(c as u32))
                } else {
                    b.emit(Ty::I64, Op::I64ExtendI32U(carry32))
                };
                let hi0 = b.emit_i64_bin(IntBinOp::Add, a_hi, b_hi);
                let hi = b.emit_i64_bin(IntBinOp::Add, hi0, carry64);
                stack.push(lo);
                stack.push(hi);
            }
            0xfc14 => {
                // i64.sub128(a_lo, a_hi, b_lo, b_hi) -> (lo, hi)
                let b_hi = stack.pop().ok_or("i64.sub128 b_hi")?;
                let b_lo = stack.pop().ok_or("i64.sub128 b_lo")?;
                let a_hi = stack.pop().ok_or("i64.sub128 a_hi")?;
                let a_lo = stack.pop().ok_or("i64.sub128 a_lo")?;
                let lo = b.emit_i64_bin(IntBinOp::Sub, a_lo, b_lo);
                let borrow32 = b.emit_i64_cmp(CmpOp::LtU, a_lo, b_lo);
                let borrow64 = if let Some(c) = b.as_i32(borrow32) {
                    b.const_i64(i64::from(c as u32))
                } else {
                    b.emit(Ty::I64, Op::I64ExtendI32U(borrow32))
                };
                let hi0 = b.emit_i64_bin(IntBinOp::Sub, a_hi, b_hi);
                let hi = b.emit_i64_bin(IntBinOp::Sub, hi0, borrow64);
                stack.push(lo);
                stack.push(hi);
            }
            0xfc15 | 0xfc16 => {
                // i64.mul_wide_s / i64.mul_wide_u(a, b) -> (lo, hi)
                let rhs = stack.pop().ok_or("i64.mul_wide rhs")?;
                let lhs = stack.pop().ok_or("i64.mul_wide lhs")?;
                let lo = b.emit_i64_bin(IntBinOp::Mul, lhs, rhs);
                let hi_op = if op == 0xfc15 {
                    IntBinOp::MulHiS
                } else {
                    IntBinOp::MulHiU
                };
                let hi = b.emit_i64_bin(hi_op, lhs, rhs);
                stack.push(lo);
                stack.push(hi);
            }
            0xfd0c => {
                let Immediate::Bytes(ref bytes) = insn.immediate else {
                    return Err("v128.const".into());
                };
                let arr: [u8; 16] = bytes.as_ref().try_into().map_err(|_| "v128.const len")?;
                let idx = b.ir.v128_consts.len() as u32;
                b.ir.v128_consts.push(arr);
                let r = b.emit(Ty::V128, Op::ConstV128(idx));
                stack.push(r);
            }
            0xfd0d => {
                let Immediate::Bytes(ref bytes) = insn.immediate else {
                    return Err("i8x16.shuffle lanes".into());
                };
                let arr: [u8; 16] = bytes.as_ref().try_into().map_err(|_| "shuffle lanes len")?;
                let bv = stack.pop().ok_or("shuffle b")?;
                let av = stack.pop().ok_or("shuffle a")?;
                let lanes_idx = b.ir.v128_consts.len() as u32;
                b.ir.v128_consts.push(arr);
                let r = b.emit(
                    Ty::V128,
                    Op::V128Shuffle {
                        a: av,
                        b: bv,
                        lanes_idx,
                    },
                );
                stack.push(r);
            }
            0xfd0f..=0xfd14 => {
                let val = stack.pop().ok_or("splat")?;
                let kind = match op {
                    0xfd0f => SimdSplatKind::I8x16,
                    0xfd10 => SimdSplatKind::I16x8,
                    0xfd11 => SimdSplatKind::I32x4,
                    0xfd12 => SimdSplatKind::I64x2,
                    0xfd13 => SimdSplatKind::F32x4,
                    _ => SimdSplatKind::F64x2,
                };
                let r = b.emit(Ty::V128, Op::V128Splat { kind, val });
                stack.push(r);
            }
            0xfd15 | 0xfd16 | 0xfd18 | 0xfd19 | 0xfd1b | 0xfd1d | 0xfd1f | 0xfd21 => {
                let lane = imm_idx()? as u8;
                let v = stack.pop().ok_or("extract_lane")?;
                let kind = match op {
                    0xfd15 => SimdLaneKind::I8x16S,
                    0xfd16 => SimdLaneKind::I8x16U,
                    0xfd18 => SimdLaneKind::I16x8S,
                    0xfd19 => SimdLaneKind::I16x8U,
                    0xfd1b => SimdLaneKind::I32x4,
                    0xfd1d => SimdLaneKind::I64x2,
                    0xfd1f => SimdLaneKind::F32x4,
                    _ => SimdLaneKind::F64x2,
                };
                let r = b.emit(kind.scalar_ty(), Op::V128ExtractLane { kind, v, lane });
                stack.push(r);
            }
            0xfd17 | 0xfd1a | 0xfd1c | 0xfd1e | 0xfd20 | 0xfd22 => {
                let lane = imm_idx()? as u8;
                let x = stack.pop().ok_or("replace_lane x")?;
                let v = stack.pop().ok_or("replace_lane v")?;
                let kind = match op {
                    0xfd17 => SimdLaneKind::I8x16U,
                    0xfd1a => SimdLaneKind::I16x8U,
                    0xfd1c => SimdLaneKind::I32x4,
                    0xfd1e => SimdLaneKind::I64x2,
                    0xfd20 => SimdLaneKind::F32x4,
                    _ => SimdLaneKind::F64x2,
                };
                let r = b.emit(Ty::V128, Op::V128ReplaceLane { kind, v, x, lane });
                stack.push(r);
            }
            0xfd52 => {
                let c = stack.pop().ok_or("bitselect c")?;
                let bv = stack.pop().ok_or("bitselect b")?;
                let av = stack.pop().ok_or("bitselect a")?;
                let r = b.emit(
                    Ty::V128,
                    Op::V128Ternary {
                        op,
                        a: av,
                        b: bv,
                        c,
                    },
                );
                stack.push(r);
            }
            0xfd53 | 0xfd63 | 0xfd64 | 0xfd83 | 0xfd84 | 0xfda3 | 0xfda4 | 0xfdc3 | 0xfdc4 => {
                let a = stack.pop().ok_or("simd to_i32")?;
                let r = b.emit(Ty::I32, Op::V128ToI32 { op, a });
                stack.push(r);
            }
            0xfd6b..=0xfd6d | 0xfd8b..=0xfd8d | 0xfdab..=0xfdad | 0xfdcb..=0xfdcd => {
                let shift = stack.pop().ok_or("simd shift cnt")?;
                let v = stack.pop().ok_or("simd shift v")?;
                let r = b.emit(Ty::V128, Op::V128Shift { op, v, shift });
                stack.push(r);
            }
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
            | 0xfdf8..=0xfdff => {
                let a = stack.pop().ok_or("simd unop")?;
                let r = b.emit(Ty::V128, Op::V128Un { op, a });
                stack.push(r);
            }
            0xfd0e | 0xfd23..=0xfd4c | 0xfd4e..=0xfd51 | 0xfd65..=0xfdff => {
                let bv = stack.pop().ok_or("simd binop b")?;
                let av = stack.pop().ok_or("simd binop a")?;
                let r = b.emit(Ty::V128, Op::V128Bin { op, a: av, b: bv });
                stack.push(r);
            }
            0xfa00..=0xfa13 => {
                // Relaxed SIMD (all 20 opcodes, 100% bit-exact with `wtime-runtime/src/simd.rs`).
                let r = match op {
                    0xfa00 => {
                        let bv = stack.pop().ok_or("relaxed_swizzle b")?;
                        let av = stack.pop().ok_or("relaxed_swizzle a")?;
                        b.emit(
                            Ty::V128,
                            Op::V128Bin {
                                op: 0xfd0e,
                                a: av,
                                b: bv,
                            },
                        )
                    }
                    0xfa01..=0xfa04 => {
                        let av = stack.pop().ok_or("relaxed_trunc a")?;
                        let std_op = match op {
                            0xfa01 => 0xfdf8,
                            0xfa02 => 0xfdf9,
                            0xfa03 => 0xfdfc,
                            _ => 0xfdfd,
                        };
                        b.emit(Ty::V128, Op::V128Un { op: std_op, a: av })
                    }
                    0xfa05..=0xfa08 => {
                        let cv = stack.pop().ok_or("relaxed_madd c")?;
                        let bv = stack.pop().ok_or("relaxed_madd b")?;
                        let av = stack.pop().ok_or("relaxed_madd a")?;
                        let (neg_op, mul_op, add_op) = if op <= 0xfa06 {
                            (0xfde1, 0xfde6, 0xfde4)
                        } else {
                            (0xfded, 0xfdf2, 0xfdf0)
                        };
                        let lhs = if (op & 1) == 0 {
                            b.emit(Ty::V128, Op::V128Un { op: neg_op, a: av })
                        } else {
                            av
                        };
                        let prod = b.emit(
                            Ty::V128,
                            Op::V128Bin {
                                op: mul_op,
                                a: lhs,
                                b: bv,
                            },
                        );
                        b.emit(
                            Ty::V128,
                            Op::V128Bin {
                                op: add_op,
                                a: prod,
                                b: cv,
                            },
                        )
                    }
                    0xfa09..=0xfa0c => {
                        let cv = stack.pop().ok_or("relaxed_laneselect c")?;
                        let bv = stack.pop().ok_or("relaxed_laneselect b")?;
                        let av = stack.pop().ok_or("relaxed_laneselect a")?;
                        b.emit(
                            Ty::V128,
                            Op::V128Ternary {
                                op: 0xfd52,
                                a: av,
                                b: bv,
                                c: cv,
                            },
                        )
                    }
                    0xfa0d..=0xfa11 => {
                        let bv = stack.pop().ok_or("relaxed binop b")?;
                        let av = stack.pop().ok_or("relaxed binop a")?;
                        let std_op = match op {
                            0xfa0d => 0xfde8,
                            0xfa0e => 0xfde9,
                            0xfa0f => 0xfdf4,
                            0xfa10 => 0xfdf5,
                            _ => 0xfd82,
                        };
                        b.emit(
                            Ty::V128,
                            Op::V128Bin {
                                op: std_op,
                                a: av,
                                b: bv,
                            },
                        )
                    }
                    0xfa12 => {
                        let bv = stack.pop().ok_or("relaxed_dot b")?;
                        let av = stack.pop().ok_or("relaxed_dot a")?;
                        let a0 = b.emit(
                            Ty::V128,
                            Op::V128Bin {
                                op: 0xfd9c,
                                a: av,
                                b: bv,
                            },
                        );
                        let a1 = b.emit(
                            Ty::V128,
                            Op::V128Bin {
                                op: 0xfd9d,
                                a: av,
                                b: bv,
                            },
                        );
                        let evens_idx = b.ir.v128_consts.len() as u32;
                        b.ir.v128_consts
                            .push([0, 1, 4, 5, 8, 9, 12, 13, 16, 17, 20, 21, 24, 25, 28, 29]);
                        let odds_idx = b.ir.v128_consts.len() as u32;
                        b.ir.v128_consts
                            .push([2, 3, 6, 7, 10, 11, 14, 15, 18, 19, 22, 23, 26, 27, 30, 31]);
                        let evens = b.emit(
                            Ty::V128,
                            Op::V128Shuffle {
                                a: a0,
                                b: a1,
                                lanes_idx: evens_idx,
                            },
                        );
                        let odds = b.emit(
                            Ty::V128,
                            Op::V128Shuffle {
                                a: a0,
                                b: a1,
                                lanes_idx: odds_idx,
                            },
                        );
                        b.emit(
                            Ty::V128,
                            Op::V128Bin {
                                op: 0xfd8e,
                                a: evens,
                                b: odds,
                            },
                        )
                    }
                    _ => {
                        // 0xfa13: i32x4.relaxed_dot_i8x16_i7x16_add_s
                        let cv = stack.pop().ok_or("relaxed_dot_add c")?;
                        let bv = stack.pop().ok_or("relaxed_dot_add b")?;
                        let av = stack.pop().ok_or("relaxed_dot_add a")?;
                        let a0 = b.emit(
                            Ty::V128,
                            Op::V128Bin {
                                op: 0xfd9c,
                                a: av,
                                b: bv,
                            },
                        );
                        let a1 = b.emit(
                            Ty::V128,
                            Op::V128Bin {
                                op: 0xfd9d,
                                a: av,
                                b: bv,
                            },
                        );
                        let s0 = b.emit(Ty::V128, Op::V128Un { op: 0xfd7e, a: a0 });
                        let s1 = b.emit(Ty::V128, Op::V128Un { op: 0xfd7e, a: a1 });
                        let evens_idx = b.ir.v128_consts.len() as u32;
                        b.ir.v128_consts
                            .push([0, 1, 2, 3, 8, 9, 10, 11, 16, 17, 18, 19, 24, 25, 26, 27]);
                        let odds_idx = b.ir.v128_consts.len() as u32;
                        b.ir.v128_consts
                            .push([4, 5, 6, 7, 12, 13, 14, 15, 20, 21, 22, 23, 28, 29, 30, 31]);
                        let evens = b.emit(
                            Ty::V128,
                            Op::V128Shuffle {
                                a: s0,
                                b: s1,
                                lanes_idx: evens_idx,
                            },
                        );
                        let odds = b.emit(
                            Ty::V128,
                            Op::V128Shuffle {
                                a: s0,
                                b: s1,
                                lanes_idx: odds_idx,
                            },
                        );
                        let dot = b.emit(
                            Ty::V128,
                            Op::V128Bin {
                                op: 0xfdae,
                                a: evens,
                                b: odds,
                            },
                        );
                        b.emit(
                            Ty::V128,
                            Op::V128Bin {
                                op: 0xfdae,
                                a: dot,
                                b: cv,
                            },
                        )
                    }
                };
                stack.push(r);
            }
            // Numeric ops:
            0x45 => {
                let a = stack.pop().ok_or("i32.eqz")?;
                let r = if let Some(c) = b.as_i32(a) {
                    b.const_i32(i32::from(c == 0))
                } else {
                    b.emit(Ty::I32, Op::I32Eqz(a))
                };
                stack.push(r);
            }
            0x46..=0x4f => {
                let bv = stack.pop().ok_or("i32 cmp b")?;
                let av = stack.pop().ok_or("i32 cmp a")?;
                let cmp = match op {
                    0x46 => CmpOp::Eq,
                    0x47 => CmpOp::Ne,
                    0x48 => CmpOp::LtS,
                    0x49 => CmpOp::LtU,
                    0x4a => CmpOp::GtS,
                    0x4b => CmpOp::GtU,
                    0x4c => CmpOp::LeS,
                    0x4d => CmpOp::LeU,
                    0x4e => CmpOp::GeS,
                    _ => CmpOp::GeU,
                };
                let r = b.emit_i32_cmp(cmp, av, bv);
                stack.push(r);
            }
            0x50 => {
                let a = stack.pop().ok_or("i64.eqz")?;
                let r = if let Some(c) = b.as_i64(a) {
                    b.const_i32(i32::from(c == 0))
                } else {
                    b.emit(Ty::I32, Op::I64Eqz(a))
                };
                stack.push(r);
            }
            0x51..=0x5a => {
                let bv = stack.pop().ok_or("i64 cmp b")?;
                let av = stack.pop().ok_or("i64 cmp a")?;
                let cmp = match op {
                    0x51 => CmpOp::Eq,
                    0x52 => CmpOp::Ne,
                    0x53 => CmpOp::LtS,
                    0x54 => CmpOp::LtU,
                    0x55 => CmpOp::GtS,
                    0x56 => CmpOp::GtU,
                    0x57 => CmpOp::LeS,
                    0x58 => CmpOp::LeU,
                    0x59 => CmpOp::GeS,
                    _ => CmpOp::GeU,
                };
                let r = b.emit_i64_cmp(cmp, av, bv);
                stack.push(r);
            }
            0x5b..=0x60 => {
                let bv = stack.pop().ok_or("f32 cmp b")?;
                let av = stack.pop().ok_or("f32 cmp a")?;
                let cmp = match op {
                    0x5b => CmpOp::Eq,
                    0x5c => CmpOp::Ne,
                    0x5d => CmpOp::LtS,
                    0x5e => CmpOp::GtS,
                    0x5f => CmpOp::LeS,
                    _ => CmpOp::GeS,
                };
                let r = b.emit(Ty::I32, Op::F32Cmp(cmp, av, bv));
                stack.push(r);
            }
            0x61..=0x66 => {
                let bv = stack.pop().ok_or("f64 cmp b")?;
                let av = stack.pop().ok_or("f64 cmp a")?;
                let cmp = match op {
                    0x61 => CmpOp::Eq,
                    0x62 => CmpOp::Ne,
                    0x63 => CmpOp::LtS,
                    0x64 => CmpOp::GtS,
                    0x65 => CmpOp::LeS,
                    _ => CmpOp::GeS,
                };
                let r = b.emit(Ty::I32, Op::F64Cmp(cmp, av, bv));
                stack.push(r);
            }
            0x67..=0x69 => {
                let a = stack.pop().ok_or("i32 bit unop")?;
                let r = match op {
                    0x67 => b.emit(Ty::I32, Op::I32Clz(a)),
                    0x68 => b.emit(Ty::I32, Op::I32Ctz(a)),
                    _ => b.emit(Ty::I32, Op::I32Popcnt(a)),
                };
                stack.push(r);
            }
            0x6a..=0x78 => {
                let bv = stack.pop().ok_or("i32 bin b")?;
                let av = stack.pop().ok_or("i32 bin a")?;
                let bop = match op {
                    0x6a => IntBinOp::Add,
                    0x6b => IntBinOp::Sub,
                    0x6c => IntBinOp::Mul,
                    0x6d => IntBinOp::DivS,
                    0x6e => IntBinOp::DivU,
                    0x6f => IntBinOp::RemS,
                    0x70 => IntBinOp::RemU,
                    0x71 => IntBinOp::And,
                    0x72 => IntBinOp::Or,
                    0x73 => IntBinOp::Xor,
                    0x74 => IntBinOp::Shl,
                    0x75 => IntBinOp::ShrS,
                    0x76 => IntBinOp::ShrU,
                    0x77 => IntBinOp::Rotl,
                    _ => IntBinOp::Rotr,
                };
                let r = b.emit_i32_bin(bop, av, bv);
                stack.push(r);
            }
            0x79..=0x7b => {
                let a = stack.pop().ok_or("i64 bit unop")?;
                let r = match op {
                    0x79 => b.emit(Ty::I64, Op::I64Clz(a)),
                    0x7a => b.emit(Ty::I64, Op::I64Ctz(a)),
                    _ => b.emit(Ty::I64, Op::I64Popcnt(a)),
                };
                stack.push(r);
            }
            0x7c..=0x8a => {
                let bv = stack.pop().ok_or("i64 bin b")?;
                let av = stack.pop().ok_or("i64 bin a")?;
                let bop = match op {
                    0x7c => IntBinOp::Add,
                    0x7d => IntBinOp::Sub,
                    0x7e => IntBinOp::Mul,
                    0x7f => IntBinOp::DivS,
                    0x80 => IntBinOp::DivU,
                    0x81 => IntBinOp::RemS,
                    0x82 => IntBinOp::RemU,
                    0x83 => IntBinOp::And,
                    0x84 => IntBinOp::Or,
                    0x85 => IntBinOp::Xor,
                    0x86 => IntBinOp::Shl,
                    0x87 => IntBinOp::ShrS,
                    0x88 => IntBinOp::ShrU,
                    0x89 => IntBinOp::Rotl,
                    _ => IntBinOp::Rotr,
                };
                let r = b.emit_i64_bin(bop, av, bv);
                stack.push(r);
            }
            0x8b..=0x91 => {
                let a = stack.pop().ok_or("f32 unop")?;
                let uop = match op {
                    0x8b => FloatUnOp::Abs,
                    0x8c => FloatUnOp::Neg,
                    0x8d => FloatUnOp::Ceil,
                    0x8e => FloatUnOp::Floor,
                    0x8f => FloatUnOp::Trunc,
                    0x90 => FloatUnOp::Nearest,
                    _ => FloatUnOp::Sqrt,
                };
                let r = b.emit(Ty::F32, Op::F32Un(uop, a));
                stack.push(r);
            }
            0x92..=0x98 => {
                let bv = stack.pop().ok_or("f32 bin b")?;
                let av = stack.pop().ok_or("f32 bin a")?;
                let bop = match op {
                    0x92 => FloatBinOp::Add,
                    0x93 => FloatBinOp::Sub,
                    0x94 => FloatBinOp::Mul,
                    0x95 => FloatBinOp::Div,
                    0x96 => FloatBinOp::Min,
                    0x97 => FloatBinOp::Max,
                    _ => FloatBinOp::Copysign,
                };
                let r = b.emit(Ty::F32, Op::F32Bin(bop, av, bv));
                stack.push(r);
            }
            0x99..=0x9f => {
                let a = stack.pop().ok_or("f64 unop")?;
                let uop = match op {
                    0x99 => FloatUnOp::Abs,
                    0x9a => FloatUnOp::Neg,
                    0x9b => FloatUnOp::Ceil,
                    0x9c => FloatUnOp::Floor,
                    0x9d => FloatUnOp::Trunc,
                    0x9e => FloatUnOp::Nearest,
                    _ => FloatUnOp::Sqrt,
                };
                let r = b.emit(Ty::F64, Op::F64Un(uop, a));
                stack.push(r);
            }
            0xa0..=0xa6 => {
                let bv = stack.pop().ok_or("f64 bin b")?;
                let av = stack.pop().ok_or("f64 bin a")?;
                let bop = match op {
                    0xa0 => FloatBinOp::Add,
                    0xa1 => FloatBinOp::Sub,
                    0xa2 => FloatBinOp::Mul,
                    0xa3 => FloatBinOp::Div,
                    0xa4 => FloatBinOp::Min,
                    0xa5 => FloatBinOp::Max,
                    _ => FloatBinOp::Copysign,
                };
                let r = b.emit(Ty::F64, Op::F64Bin(bop, av, bv));
                stack.push(r);
            }
            0xa7 => {
                let a = stack.pop().ok_or("i32.wrap")?;
                let r = if let Some(c) = b.as_i64(a) {
                    b.const_i32(c as i32)
                } else {
                    b.emit(Ty::I32, Op::I32WrapI64(a))
                };
                stack.push(r);
            }
            0xa8..=0xab | 0xae..=0xb1 | 0xfc00..=0xfc07 => {
                let a = stack.pop().ok_or("trunc")?;
                let top = match op {
                    0xa8 => TruncOp::I32F32S,
                    0xa9 => TruncOp::I32F32U,
                    0xaa => TruncOp::I32F64S,
                    0xab => TruncOp::I32F64U,
                    0xae => TruncOp::I64F32S,
                    0xaf => TruncOp::I64F32U,
                    0xb0 => TruncOp::I64F64S,
                    0xb1 => TruncOp::I64F64U,
                    0xfc00 => TruncOp::I32F32SatS,
                    0xfc01 => TruncOp::I32F32SatU,
                    0xfc02 => TruncOp::I32F64SatS,
                    0xfc03 => TruncOp::I32F64SatU,
                    0xfc04 => TruncOp::I64F32SatS,
                    0xfc05 => TruncOp::I64F32SatU,
                    0xfc06 => TruncOp::I64F64SatS,
                    _ => TruncOp::I64F64SatU,
                };
                let r = b.emit(top.result_ty(), Op::Trunc(top, a));
                stack.push(r);
            }
            0xac => {
                let a = stack.pop().ok_or("i64.extend_i32_s")?;
                let r = if let Some(c) = b.as_i32(a) {
                    b.const_i64(c as i64)
                } else {
                    b.emit(Ty::I64, Op::I64ExtendI32S(a))
                };
                stack.push(r);
            }
            0xad => {
                let a = stack.pop().ok_or("i64.extend_i32_u")?;
                let r = if let Some(c) = b.as_i32(a) {
                    b.const_i64(c as u32 as i64)
                } else {
                    b.emit(Ty::I64, Op::I64ExtendI32U(a))
                };
                stack.push(r);
            }
            0xb2 => {
                let a = stack.pop().ok_or("f32.convert_i32_s")?;
                let r = b.emit(Ty::F32, Op::F32ConvertI32S(a));
                stack.push(r);
            }
            0xb3 => {
                let a = stack.pop().ok_or("f32.convert_i32_u")?;
                let r = b.emit(Ty::F32, Op::F32ConvertI32U(a));
                stack.push(r);
            }
            0xb4 => {
                let a = stack.pop().ok_or("f32.convert_i64_s")?;
                let r = b.emit(Ty::F32, Op::F32ConvertI64S(a));
                stack.push(r);
            }
            0xb5 => {
                let a = stack.pop().ok_or("f32.convert_i64_u")?;
                let r = b.emit(Ty::F32, Op::F32ConvertI64U(a));
                stack.push(r);
            }
            0xb6 => {
                let a = stack.pop().ok_or("f32.demote_f64")?;
                let r = b.emit(Ty::F32, Op::F32DemoteF64(a));
                stack.push(r);
            }
            0xb7 => {
                let a = stack.pop().ok_or("f64.convert_i32_s")?;
                let r = b.emit(Ty::F64, Op::F64ConvertI32S(a));
                stack.push(r);
            }
            0xb8 => {
                let a = stack.pop().ok_or("f64.convert_i32_u")?;
                let r = b.emit(Ty::F64, Op::F64ConvertI32U(a));
                stack.push(r);
            }
            0xb9 => {
                let a = stack.pop().ok_or("f64.convert_i64_s")?;
                let r = b.emit(Ty::F64, Op::F64ConvertI64S(a));
                stack.push(r);
            }
            0xba => {
                let a = stack.pop().ok_or("f64.convert_i64_u")?;
                let r = b.emit(Ty::F64, Op::F64ConvertI64U(a));
                stack.push(r);
            }
            0xbb => {
                let a = stack.pop().ok_or("f64.promote_f32")?;
                let r = b.emit(Ty::F64, Op::F64PromoteF32(a));
                stack.push(r);
            }
            0xbc => {
                let a = stack.pop().ok_or("i32.reinterpret_f32")?;
                let r = b.emit(Ty::I32, Op::I32ReinterpretF32(a));
                stack.push(r);
            }
            0xbd => {
                let a = stack.pop().ok_or("i64.reinterpret_f64")?;
                let r = b.emit(Ty::I64, Op::I64ReinterpretF64(a));
                stack.push(r);
            }
            0xbe => {
                let a = stack.pop().ok_or("f32.reinterpret_i32")?;
                let r = b.emit(Ty::F32, Op::F32ReinterpretI32(a));
                stack.push(r);
            }
            0xbf => {
                let a = stack.pop().ok_or("f64.reinterpret_i64")?;
                let r = b.emit(Ty::F64, Op::F64ReinterpretI64(a));
                stack.push(r);
            }
            0xc0 => {
                let a = stack.pop().ok_or("i32.extend8_s")?;
                let r = b.emit(Ty::I32, Op::I32Extend8S(a));
                stack.push(r);
            }
            0xc1 => {
                let a = stack.pop().ok_or("i32.extend16_s")?;
                let r = b.emit(Ty::I32, Op::I32Extend16S(a));
                stack.push(r);
            }
            0xc2 => {
                let a = stack.pop().ok_or("i64.extend8_s")?;
                let r = b.emit(Ty::I64, Op::I64Extend8S(a));
                stack.push(r);
            }
            0xc3 => {
                let a = stack.pop().ok_or("i64.extend16_s")?;
                let r = b.emit(Ty::I64, Op::I64Extend16S(a));
                stack.push(r);
            }
            0xc4 => {
                let a = stack.pop().ok_or("i64.extend32_s")?;
                let r = b.emit(Ty::I64, Op::I64Extend32S(a));
                stack.push(r);
            }
            _ => {
                return Err(format!(
                    "fn {func_index} pc {pc}: unsupported opcode {op:#x}"
                ));
            }
        }
    }

    let mut ir = b.ir;
    if opt {
        optimize_ir_with_budget_and_scratch(&mut ir, budget, opt_scratch)?;
    }
    Ok(ir)
}

/// Reusable worker-local scratch buffers for `optimize_ir_with_budget_and_scratch`.
#[derive(Default)]
pub struct OptIrScratch {
    def_stamp: Vec<(u32, u32)>,
    checked_stamp: Vec<(u32, u64)>,
    out_buf: Vec<Inst>,
    uses: Vec<u32>,
    keep: Vec<bool>,
}

/// Fast O(N) SSA optimizations:
/// 1. Compare + `BranchIf` fusion (`CmpBranch`) + `i32.eqz` inversion + constant branch folding.
/// 2. Within-block redundant bounds-check elimination + pure load-run check hoisting.
/// 3. Dead-instruction elimination (reverse use-count pass).
pub fn optimize_ir(ir: &mut IrFunc) {
    let _ = optimize_ir_with_budget(ir, None);
}

/// Runs `optimize_ir` while cooperatively polling `budget`'s deadline across blocks.
pub fn optimize_ir_with_budget(ir: &mut IrFunc, budget: Option<&CompileBudget>) -> Res<()> {
    let mut scratch = OptIrScratch::default();
    optimize_ir_with_budget_and_scratch(ir, budget, &mut scratch)
}

/// Same as `optimize_ir_with_budget`, reusing worker-local `OptIrScratch` buffers.
pub fn optimize_ir_with_budget_and_scratch(
    ir: &mut IrFunc,
    budget: Option<&CompileBudget>,
    scratch: &mut OptIrScratch,
) -> Res<()> {
    // Pass 1: Fuse `Select` -> `CmpSelect` and `BranchIf` -> `CmpBranch` (without crossing Call/CallIndirect!).
    // Uses an O(1) epoch-stamped `(epoch, inst_idx)` lookup table (`def_stamp`) where `epoch`
    // bumps at every block start and every `Call`/`CallIndirect` barrier.
    let n_vals = ir.val_tys.len();
    scratch.def_stamp.clear();
    if scratch.def_stamp.capacity() > 16_384 && n_vals <= 4096 {
        scratch.def_stamp.shrink_to(4096);
    }
    scratch.def_stamp.resize(n_vals, (0, 0));
    let def_stamp = &mut scratch.def_stamp;
    let mut cur_epoch: u32 = 0;

    let mut dropped_preds: Vec<BlockId> = Vec::new();
    let mut any_mem_op = false;
    for (bi, bb) in ir.blocks.iter_mut().enumerate() {
        if let Some(bg) = budget
            && (bi & 255) == 0
        {
            bg.check_deadline()?;
        }
        cur_epoch = cur_epoch.wrapping_add(1);
        if cur_epoch == 0 {
            def_stamp.fill((0, 0));
            cur_epoch = 1;
        }
        // Pass 1a: Fuse `Cmp` + optional `I32Eqz` + `Select` -> `Op::CmpSelect`.
        for idx in 0..bb.insts.len() {
            let op = bb.insts[idx].op;
            match op {
                Op::Call { .. } | Op::CallIndirect { .. } => {
                    cur_epoch = cur_epoch.wrapping_add(1);
                    if cur_epoch == 0 {
                        def_stamp.fill((0, 0));
                        cur_epoch = 1;
                    }
                }
                Op::BoundsCheck { .. } | Op::Load { .. } | Op::Store { .. } => {
                    any_mem_op = true;
                }
                Op::Select {
                    mut cond,
                    a: mut tv,
                    b: mut fv,
                } => {
                    let (ep0, idx0) = def_stamp[cond.0 as usize];
                    if ep0 == cur_epoch
                        && let Op::I32Eqz(inner) = bb.insts[idx0 as usize].op
                    {
                        cond = inner;
                        std::mem::swap(&mut tv, &mut fv);
                    }
                    let (ep1, idx1) = def_stamp[cond.0 as usize];
                    let fused_sel = if ep1 == cur_epoch {
                        match bb.insts[idx1 as usize].op {
                            Op::I32Cmp(op, a, b) => Some(Op::CmpSelect {
                                kind: CmpBranchKind::I32(op),
                                a,
                                b,
                                tv,
                                fv,
                            }),
                            Op::I32CmpImm(op, a, imm) => Some(Op::CmpSelect {
                                kind: CmpBranchKind::I32Imm(op, imm),
                                a,
                                b: a,
                                tv,
                                fv,
                            }),
                            Op::I64Cmp(op, a, b) => Some(Op::CmpSelect {
                                kind: CmpBranchKind::I64(op),
                                a,
                                b,
                                tv,
                                fv,
                            }),
                            Op::I64CmpImm(op, a, imm) => Some(Op::CmpSelect {
                                kind: CmpBranchKind::I64Imm(op, imm),
                                a,
                                b: a,
                                tv,
                                fv,
                            }),
                            Op::F32Cmp(op, a, b) => Some(Op::CmpSelect {
                                kind: CmpBranchKind::F32(op),
                                a,
                                b,
                                tv,
                                fv,
                            }),
                            Op::F64Cmp(op, a, b) => Some(Op::CmpSelect {
                                kind: CmpBranchKind::F64(op),
                                a,
                                b,
                                tv,
                                fv,
                            }),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    if let Some(new_op) = fused_sel {
                        bb.insts[idx].op = new_op;
                    }
                }
                _ => {}
            }
            if let Some(res) = bb.insts[idx].res {
                def_stamp[res.0 as usize] = (cur_epoch, idx as u32);
            }
        }
        // Pass 1b: Fuse `BranchIf` with defining `Cmp`/`Eqz`/`ConstI32` in the same block (not crossing Call/CallIndirect).
        if let Term::BranchIf {
            mut cond,
            mut then_bb,
            mut then_args,
            mut else_bb,
            mut else_args,
        } = bb.term
        {
            let mut fused: Option<Term> = None;
            let (ep0, idx0) = def_stamp[cond.0 as usize];
            if ep0 == cur_epoch
                && let Op::I32Eqz(inner) = bb.insts[idx0 as usize].op
            {
                cond = inner;
                std::mem::swap(&mut then_bb, &mut else_bb);
                std::mem::swap(&mut then_args, &mut else_args);
            }
            let (ep1, idx1) = def_stamp[cond.0 as usize];
            match (ep1 == cur_epoch).then(|| bb.insts[idx1 as usize].op) {
                Some(Op::ConstI32(c)) => {
                    fused = Some(if c != 0 {
                        dropped_preds.push(else_bb);
                        Term::Jump {
                            target: then_bb,
                            args: then_args,
                        }
                    } else {
                        dropped_preds.push(then_bb);
                        Term::Jump {
                            target: else_bb,
                            args: else_args,
                        }
                    });
                }
                Some(Op::I32Cmp(op, a, b)) => {
                    fused = Some(Term::CmpBranch {
                        kind: CmpBranchKind::I32(op),
                        a,
                        b,
                        then_bb,
                        then_args,
                        else_bb,
                        else_args,
                    });
                }
                Some(Op::I32CmpImm(op, a, imm)) => {
                    fused = Some(Term::CmpBranch {
                        kind: CmpBranchKind::I32Imm(op, imm),
                        a,
                        b: a,
                        then_bb,
                        then_args,
                        else_bb,
                        else_args,
                    });
                }
                Some(Op::I64Cmp(op, a, b)) => {
                    fused = Some(Term::CmpBranch {
                        kind: CmpBranchKind::I64(op),
                        a,
                        b,
                        then_bb,
                        then_args,
                        else_bb,
                        else_args,
                    });
                }
                Some(Op::I64CmpImm(op, a, imm)) => {
                    fused = Some(Term::CmpBranch {
                        kind: CmpBranchKind::I64Imm(op, imm),
                        a,
                        b: a,
                        then_bb,
                        then_args,
                        else_bb,
                        else_args,
                    });
                }
                Some(Op::F32Cmp(op, a, b)) => {
                    fused = Some(Term::CmpBranch {
                        kind: CmpBranchKind::F32(op),
                        a,
                        b,
                        then_bb,
                        then_args,
                        else_bb,
                        else_args,
                    });
                }
                Some(Op::F64Cmp(op, a, b)) => {
                    fused = Some(Term::CmpBranch {
                        kind: CmpBranchKind::F64(op),
                        a,
                        b,
                        then_bb,
                        then_args,
                        else_bb,
                        else_args,
                    });
                }
                _ => {
                    bb.term = Term::BranchIf {
                        cond,
                        then_bb,
                        then_args,
                        else_bb,
                        else_args,
                    };
                }
            }
            if let Some(t) = fused {
                bb.term = t;
            }
        }
        // Pass 1c: Fold constant `Term::CmpBranch` (e.g. rotated loop entry guard `cmp $-1, 2`) -> `Jump`.
        if let Term::CmpBranch {
            kind,
            a,
            b,
            then_bb,
            then_args,
            else_bb,
            else_args,
        } = bb.term
        {
            let (ep_a, idx_a) = def_stamp[a.0 as usize];
            let ca = (ep_a == cur_epoch).then(|| bb.insts[idx_a as usize].op);
            let eval_cmp_i32 = |op: CmpOp, x: i32, y: i32| match op {
                CmpOp::Eq => x == y,
                CmpOp::Ne => x != y,
                CmpOp::LtS => x < y,
                CmpOp::LtU => (x as u32) < (y as u32),
                CmpOp::GtS => x > y,
                CmpOp::GtU => (x as u32) > (y as u32),
                CmpOp::LeS => x <= y,
                CmpOp::LeU => (x as u32) <= (y as u32),
                CmpOp::GeS => x >= y,
                CmpOp::GeU => (x as u32) >= (y as u32),
            };
            let eval_cmp_i64 = |op: CmpOp, x: i64, y: i64| match op {
                CmpOp::Eq => x == y,
                CmpOp::Ne => x != y,
                CmpOp::LtS => x < y,
                CmpOp::LtU => (x as u64) < (y as u64),
                CmpOp::GtS => x > y,
                CmpOp::GtU => (x as u64) > (y as u64),
                CmpOp::LeS => x <= y,
                CmpOp::LeU => (x as u64) <= (y as u64),
                CmpOp::GeS => x >= y,
                CmpOp::GeU => (x as u64) >= (y as u64),
            };
            let known = match (kind, ca) {
                (CmpBranchKind::I32Imm(op, imm), Some(Op::ConstI32(x))) => {
                    Some(eval_cmp_i32(op, x, imm))
                }
                (CmpBranchKind::I64Imm(op, imm), Some(Op::ConstI64(x))) => {
                    Some(eval_cmp_i64(op, x, i64::from(imm)))
                }
                (CmpBranchKind::I32(op), Some(Op::ConstI32(x))) => {
                    let (ep_b, idx_b) = def_stamp[b.0 as usize];
                    if ep_b == cur_epoch
                        && let Op::ConstI32(y) = bb.insts[idx_b as usize].op
                    {
                        Some(eval_cmp_i32(op, x, y))
                    } else {
                        None
                    }
                }
                (CmpBranchKind::I64(op), Some(Op::ConstI64(x))) => {
                    let (ep_b, idx_b) = def_stamp[b.0 as usize];
                    if ep_b == cur_epoch
                        && let Op::ConstI64(y) = bb.insts[idx_b as usize].op
                    {
                        Some(eval_cmp_i64(op, x, y))
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(take_then) = known {
                bb.term = if take_then {
                    dropped_preds.push(else_bb);
                    Term::Jump {
                        target: then_bb,
                        args: then_args,
                    }
                } else {
                    dropped_preds.push(then_bb);
                    Term::Jump {
                        target: else_bb,
                        args: else_args,
                    }
                };
            }
        }
    }
    for dropped in dropped_preds {
        if let Some(bb) = ir.blocks.get_mut(dropped.0 as usize) {
            bb.preds = bb.preds.saturating_sub(1);
        }
    }

    // Pass 2: Within-block redundant bounds-check elimination + pure load-run check hoisting.
    // Uses a dense block-stamped array (O(1) indexing, zero SipHash) and in-place mutation when no
    // hoisted `BoundsCheck` is inserted into the block.
    let n_vals = ir.val_tys.len();
    if any_mem_op {
        scratch.checked_stamp.clear();
        if scratch.checked_stamp.capacity() > 16_384 && n_vals <= 4096 {
            scratch.checked_stamp.shrink_to(4096);
        }
        scratch.checked_stamp.resize(n_vals, (0, 0));
        let checked_stamp = &mut scratch.checked_stamp;
        let out_buf = &mut scratch.out_buf;
        for (bi, bb) in ir.blocks.iter_mut().enumerate() {
            if let Some(bg) = budget
                && (bi & 255) == 0
            {
                bg.check_deadline()?;
            }
            let stamp = (bi as u32) + 1;
            let mut need_rewrite = false;
            let mut i = 0;
            while i < bb.insts.len() {
                if let Some(bg) = budget
                    && (i & 4095) == 4095
                {
                    bg.check_deadline()?;
                }
                match bb.insts[i].op {
                    Op::BoundsCheck { addr, end_off, .. } => {
                        let slot = &mut checked_stamp[addr.0 as usize];
                        if slot.0 != stamp || end_off > slot.1 {
                            *slot = (stamp, end_off);
                        }
                        if need_rewrite {
                            out_buf.push(bb.insts[i]);
                        }
                    }
                    Op::Load {
                        kind,
                        addr,
                        offset,
                        checked,
                    } => {
                        let end = offset.saturating_add(kind.bytes());
                        let slot = &mut checked_stamp[addr.0 as usize];
                        let prev_max = if slot.0 == stamp { slot.1 } else { 0 };
                        if checked || (slot.0 == stamp && prev_max >= end) {
                            if !checked
                                && let Op::Load {
                                    checked: ref mut c, ..
                                } = bb.insts[i].op
                            {
                                *c = true;
                            }
                            if slot.0 != stamp || end > prev_max {
                                *slot = (stamp, end);
                            }
                            if need_rewrite {
                                out_buf.push(bb.insts[i]);
                            }
                            i += 1;
                            continue;
                        }
                        let mut max_end = end;
                        let mut run_ops = 1usize;
                        let mut j = i + 1;
                        while j < bb.insts.len() {
                            if let Some(bg) = budget
                                && (j & 4095) == 4095
                            {
                                bg.check_deadline()?;
                            }
                            match bb.insts[j].op {
                                Op::Load {
                                    kind: lk,
                                    addr: la,
                                    offset: lo,
                                    ..
                                } if la == addr => {
                                    let Some(e) = lo.checked_add(lk.bytes()) else {
                                        break;
                                    };
                                    if e > max_end {
                                        max_end = e;
                                    }
                                    run_ops += 1;
                                }
                                Op::Store {
                                    kind: sk,
                                    addr: sa,
                                    offset: so,
                                    ..
                                } if sa == addr => {
                                    let Some(e) = so.checked_add(sk.bytes()) else {
                                        break;
                                    };
                                    if e > max_end {
                                        max_end = e;
                                    }
                                    run_ops += 1;
                                    break;
                                }
                                ref other if !other.has_side_effect() => {}
                                _ => break,
                            }
                            j += 1;
                        }
                        if run_ops >= 2 && max_end > end {
                            if !need_rewrite {
                                out_buf.clear();
                                out_buf.reserve(bb.insts.len() + 4);
                                out_buf.extend_from_slice(&bb.insts[..i]);
                                need_rewrite = true;
                            }
                            out_buf.push(Inst {
                                res: None,
                                op: Op::BoundsCheck {
                                    addr,
                                    end_off: max_end,
                                    for_bulk: false,
                                },
                            });
                            checked_stamp[addr.0 as usize] = (stamp, max_end);
                            let mut inst = bb.insts[i];
                            if let Op::Load {
                                checked: ref mut c, ..
                            } = inst.op
                            {
                                *c = true;
                            }
                            out_buf.push(inst);
                        } else {
                            checked_stamp[addr.0 as usize] = (stamp, end);
                            if need_rewrite {
                                out_buf.push(bb.insts[i]);
                            }
                        }
                    }
                    Op::Store {
                        kind,
                        addr,
                        offset,
                        checked,
                        ..
                    } => {
                        let end = offset.saturating_add(kind.bytes());
                        let slot = &mut checked_stamp[addr.0 as usize];
                        let prev_max = if slot.0 == stamp { slot.1 } else { 0 };
                        if !checked
                            && slot.0 == stamp
                            && prev_max >= end
                            && let Op::Store {
                                checked: ref mut c, ..
                            } = bb.insts[i].op
                        {
                            *c = true;
                        }
                        if slot.0 != stamp || end > prev_max {
                            *slot = (stamp, end);
                        }
                        if need_rewrite {
                            out_buf.push(bb.insts[i]);
                        }
                    }
                    _ => {
                        if need_rewrite {
                            out_buf.push(bb.insts[i]);
                        }
                    }
                }
                i += 1;
            }
            if need_rewrite {
                bb.insts.clear();
                bb.insts.extend_from_slice(out_buf);
            }
        }
    }

    // Pass 3: Dead-code elimination (single reverse use-count pass with saturating u32 counts).
    scratch.uses.clear();
    if scratch.uses.capacity() > 16_384 && n_vals <= 4096 {
        scratch.uses.shrink_to(4096);
    }
    scratch.uses.resize(n_vals, 0u32);
    let uses = &mut scratch.uses;
    for (bi, bb) in ir.blocks.iter().enumerate() {
        if let Some(bg) = budget
            && (bi & 255) == 0
        {
            bg.check_deadline()?;
        }
        for (ii, inst) in bb.insts.iter().enumerate() {
            if let Some(bg) = budget
                && (ii & 4095) == 4095
            {
                bg.check_deadline()?;
            }
            inst.op.for_each_use(&ir.val_pool, |v| {
                uses[v.0 as usize] = uses[v.0 as usize].saturating_add(1);
            });
        }
        bb.term.for_each_use(&ir.val_pool, &ir.br_tables, |v| {
            uses[v.0 as usize] = uses[v.0 as usize].saturating_add(1);
        });
    }
    let keep = &mut scratch.keep;
    for (bi, bb) in ir.blocks.iter_mut().rev().enumerate() {
        if let Some(bg) = budget
            && (bi & 255) == 0
        {
            bg.check_deadline()?;
        }
        keep.clear();
        keep.resize(bb.insts.len(), true);
        let mut any_dropped = false;
        for (step, (idx, inst)) in bb.insts.iter().enumerate().rev().enumerate() {
            if let Some(bg) = budget
                && (step & 4095) == 4095
            {
                bg.check_deadline()?;
            }
            if let Some(res) = inst.res
                && uses[res.0 as usize] == 0
                && !inst.op.has_side_effect()
            {
                keep[idx] = false;
                any_dropped = true;
                inst.op.for_each_use(&ir.val_pool, |v| {
                    uses[v.0 as usize] = uses[v.0 as usize].saturating_sub(1);
                });
            }
        }
        if any_dropped {
            let mut k = 0;
            bb.insts.retain(|_| {
                let r = keep[k];
                k += 1;
                r
            });
        }
    }
    Ok(())
}
