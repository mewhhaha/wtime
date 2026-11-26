use crate::Reader;
use wtime_types::*;
/// A decoded instruction: 32 bytes, so that two share a cache line. Its
/// source position is kept beside it, in `FunctionBody::offsets`.
#[derive(Clone, Debug, PartialEq)]
pub struct Instruction {
    /// Decoded opcode key: single-byte opcodes occupy `0x00..=0xff`, while prefixed
    /// opcodes pack their prefix and sub-opcode (`0xfb..` GC, `0xfc..` misc/bulk,
    /// `0xfd..`/`0xfa..` SIMD, `0xfe..` atomics).
    pub opcode: u16,
    /// Decoded instruction immediate operand, if any.
    pub immediate: Immediate,
}
/// The immediate of `try_table`, boxed: it is rare and large.
#[derive(Clone, Debug, PartialEq)]
pub struct TryTableImmediate {
    /// Block signature immediate (`Immediate::Block` or `Immediate::BlockType`).
    pub block: Immediate,
    /// Catch clauses attached to the `try_table` block in source order.
    pub catches: Vec<Catch>,
}
/// The immediate of `resume`, `resume_throw` and `switch`, boxed: it is rare
/// and large.
#[derive(Clone, Debug, PartialEq)]
pub struct ResumeImmediate {
    /// Continuation type index.
    pub ty: u32,
    /// Exception tag index for `resume_throw`, or `None` for `resume` / `switch`.
    pub tag: Option<u32>,
    /// On-clauses as `(tag_index, Some(label))` for handler branches or `(tag_index, None)` for `switch`.
    pub handlers: Vec<(u32, Option<u32>)>,
}
/// A single catch clause inside a [`TryTableImmediate`].
#[derive(Clone, Debug, PartialEq)]
pub struct Catch {
    /// Wire catch kind: `0` (`catch`), `1` (`catch_ref`), `2` (`catch_all`), or `3` (`catch_all_ref`).
    pub kind: u8,
    /// Matched tag index for `catch`/`catch_ref`, or `None` for `catch_all`/`catch_all_ref`.
    pub tag: Option<u32>,
    /// Target relative branch label depth.
    pub label: u32,
}
/// Decoded immediate operand carried by an [`Instruction`].
#[derive(Clone, Debug, PartialEq)]
pub enum Immediate {
    /// Boxed `try_table` block type and catch clauses.
    TryTable(Box<TryTableImmediate>),
    /// Boxed continuation `resume` / `resume_throw` / `switch` immediate.
    Resume(Box<ResumeImmediate>),
    /// Branch-on-cast immediate (`br_on_cast` / `br_on_cast_fail`).
    Cast {
        /// Nullability bitflags (bit 0: source nullable, bit 1: target nullable).
        flags: u8,
        /// Target relative branch label depth.
        label: u32,
        /// Source heap type.
        from: HeapType,
        /// Target heap type.
        to: HeapType,
    },
    /// Instruction takes no immediate operand.
    None,
    /// Single `u32` index or lane immediate.
    Index(u32),
    /// Pair of `u32` indices (such as type/table, struct/field, or segment/memory).
    Pair(u32, u32),
    /// Signed 33-bit block type (`-64` for empty, negative value-type code, or type index).
    Block(i64),
    /// Explicit reference-type block result (`0x63` / `0x64` prefix).
    BlockType(ValType),
    /// `br_table` target label slice and default label depth.
    BrTable(Box<[u32]>, u32),
    /// Memory argument paired with a SIMD lane index (`v128.load*_lane` / `v128.store*_lane`).
    MemLane {
        /// Base-2 logarithm of the declared alignment hint.
        align: u32,
        /// Constant byte offset added to the dynamic address.
        offset: u64,
        /// Target linear memory index.
        memory: u32,
        /// SIMD lane index.
        lane: u8,
    },
    /// Memory load/store or atomic memory immediate.
    MemArg {
        /// Base-2 logarithm of the declared alignment hint.
        align: u32,
        /// Constant byte offset added to the dynamic address.
        offset: u64,
        /// Target linear memory index.
        memory: u32,
    },
    /// 32-bit integer constant (`i32.const`).
    I32(i32),
    /// 64-bit integer constant (`i64.const`).
    I64(i64),
    /// Raw IEEE 754 bit pattern of an `f32.const`.
    F32(u32),
    /// Raw IEEE 754 bit pattern of an `f64.const`.
    F64(u64),
    /// Explicit value-type list (`select` typed form).
    Types(Box<[ValType]>),
    /// Heap type immediate (`ref.null`, `ref.test`, `ref.cast`, etc.).
    Heap(HeapType),
    /// Raw 16-byte immediate (`v128.const` or `i8x16.shuffle` lane indices).
    Bytes(Box<[u8]>),
}
pub(crate) fn val(r: &mut Reader) -> Result<ValType> {
    if matches!(r.peek()?,0x68..=0x6e|0x71..=0x75) {
        return Ok(ValType::Ref(RefType::new(true, heap(r)?)));
    }
    Ok(match r.byte()? {
        0x7f => ValType::I32,
        0x7e => ValType::I64,
        0x7d => ValType::F32,
        0x7c => ValType::F64,
        0x7b => ValType::V128,
        0x70 => ValType::FUNCREF,
        0x6f => ValType::EXTERNREF,
        0x63 => ValType::Ref(RefType::new(true, heap(r)?)),
        0x64 => ValType::Ref(RefType::new(false, heap(r)?)),
        _ => return Err(r.error("invalid value type")),
    })
}
pub(crate) fn heap(r: &mut Reader) -> Result<HeapType> {
    if r.peek()? == 0x62 {
        r.feature(r.features().custom_descriptors, "exact heap types")?;
        r.byte()?;
        return Ok(HeapType::ExactDefined(TypeIdx(r.u32()?)));
    }
    Ok(match r.signed(33)? {
        n if n >= 0 => HeapType::Defined(TypeIdx(n as u32)),
        -16 => HeapType::Func,
        -17 => HeapType::Extern,
        -18 => HeapType::Any,
        -19 => HeapType::Eq,
        -20 => HeapType::I31,
        -21 => HeapType::Struct,
        -22 => HeapType::Array,
        -15 => HeapType::None,
        -13 => HeapType::NoFunc,
        -14 => HeapType::NoExtern,
        -23 => HeapType::Exn,
        -12 => HeapType::NoExn,
        -24 => HeapType::Cont,
        -11 => HeapType::NoCont,
        _ => return Err(r.error("invalid heap type")),
    })
}
pub(crate) fn count(r: &mut Reader) -> Result<usize> {
    let n = r.u32()? as usize;
    if n > r.remaining() {
        return Err(r.error("vector count exceeds remaining bytes"));
    }
    Ok(n)
}
fn memarg(r: &mut Reader) -> Result<Immediate> {
    let flags = r.u32()?;
    if flags & !127 != 0 {
        return Err(r.error("invalid memory alignment flags"));
    }
    r.feature(
        flags & 64 == 0 || r.features().multi_memory,
        "explicit memory index",
    )?;
    let memory = if flags & 64 != 0 { r.u32()? } else { 0 };
    Ok(Immediate::MemArg {
        align: flags & 63,
        offset: r.memory_offset()?,
        memory,
    })
}
#[cfg(test)]
pub(crate) fn expression(r: &mut Reader) -> Result<Vec<Instruction>> {
    expression_with_capacity(r, 0)
}
pub(crate) struct BoundedBody {
    pub instructions: Vec<Instruction>,
    pub offsets: Vec<u32>,
    pub count: usize,
    pub imm_elems: usize,
    pub saw_data_count_op: bool,
    pub dropped: bool,
    pub imm_exceeded: bool,
}
/// Decodes a constant/initializer expression while charging every decoded
/// instruction against `expr_insns <= max_insns` and every variable-length
/// immediate element against `expr_imm_elems <= max_insns`. When `keep` is false
/// or either budget is exceeded, syntax validation continues in full with zero
/// heap allocations for instructions or variable-length immediates.
pub(crate) fn expression_bounded(
    r: &mut Reader,
    expr_insns: &mut usize,
    expr_imm_elems: &mut usize,
    max_insns: usize,
    mut keep: bool,
) -> Result<(Vec<Instruction>, bool)> {
    let mut out = Vec::new();
    let mut exceeded = *expr_insns > max_insns || *expr_imm_elems > max_insns;
    if exceeded {
        keep = false;
    }
    let mut depth = 1usize;
    while depth > 0 {
        if keep && (*expr_insns >= max_insns || *expr_imm_elems > max_insns) {
            keep = false;
            out = Vec::new();
        }
        let mut imm_budget = ImmBudget::Local {
            used: expr_imm_elems,
            max: max_insns,
        };
        let mut imm_over = false;
        let mut step_imm = 0usize;
        let insn = read_instruction(
            r,
            &mut depth,
            keep,
            Some(&mut imm_budget),
            &mut step_imm,
            &mut imm_over,
        )?;
        *expr_insns = expr_insns.saturating_add(1);
        if imm_over || *expr_insns > max_insns {
            exceeded = true;
            if keep {
                keep = false;
                out = Vec::new();
            }
        } else if keep {
            out.push(insn);
        }
    }
    Ok((out, exceeded))
}
#[repr(align(64))]
pub(crate) struct AlignedAtomicUsize(pub(crate) std::sync::atomic::AtomicUsize);

impl AlignedAtomicUsize {
    pub(crate) fn new(v: usize) -> Self {
        Self(std::sync::atomic::AtomicUsize::new(v))
    }
}

/// Per-worker reservation bank for parallel function-body decoding.
/// Amortizes cross-core atomic updates over `shared_insns` and `shared_locals`
/// into batched refills while strictly bounding aggregate retained memory and
/// releasing any unspent reservation on `Drop`.
pub(crate) struct DecodeWorkerBudget<'a> {
    shared_insns: &'a AlignedAtomicUsize,
    shared_imm_elems: &'a AlignedAtomicUsize,
    shared_locals: &'a AlignedAtomicUsize,
    max_insns: usize,
    max_locals: usize,
    insn_chunk: usize,
    local_chunk: usize,
    rem_insns: usize,
    rem_locals: usize,
}

impl<'a> DecodeWorkerBudget<'a> {
    pub(crate) fn new(
        shared_insns: &'a AlignedAtomicUsize,
        shared_imm_elems: &'a AlignedAtomicUsize,
        shared_locals: &'a AlignedAtomicUsize,
        max_insns: usize,
        max_locals: usize,
        threads: usize,
    ) -> Self {
        let denom = threads.max(1).saturating_mul(4);
        Self {
            shared_insns,
            shared_imm_elems,
            shared_locals,
            max_insns,
            max_locals,
            insn_chunk: (max_insns / denom).clamp(1, 8192),
            local_chunk: (max_locals / denom).clamp(1, 2048),
            rem_insns: 0,
            rem_locals: 0,
        }
    }

    #[inline]
    pub(crate) fn try_reserve_locals(&mut self, want: usize) -> bool {
        if want == 0 {
            return true;
        }
        if self.rem_locals >= want {
            self.rem_locals -= want;
            return true;
        }
        let need = want - self.rem_locals;
        let refill = need.max(self.local_chunk);
        let got = reserve_budget(&self.shared_locals.0, self.max_locals, refill);
        if got >= need {
            self.rem_locals = self.rem_locals + got - want;
            true
        } else {
            if got > 0 {
                self.shared_locals
                    .0
                    .fetch_sub(got, std::sync::atomic::Ordering::Relaxed);
            }
            false
        }
    }

    #[inline]
    pub(crate) fn release_locals(&mut self, count: usize) {
        self.rem_locals = self.rem_locals.saturating_add(count);
    }

    #[inline]
    pub(crate) fn consume_one_insn(&mut self, hint: usize) -> bool {
        if self.rem_insns > 0 {
            self.rem_insns -= 1;
            return true;
        }
        let refill = hint.clamp(1, 1024).max(self.insn_chunk);
        let got = reserve_budget(&self.shared_insns.0, self.max_insns, refill);
        if got == 0 {
            return false;
        }
        self.rem_insns = got - 1;
        true
    }

    #[inline]
    fn insns_exhausted(&self) -> bool {
        self.rem_insns == 0
            && self
                .shared_insns
                .0
                .load(std::sync::atomic::Ordering::Relaxed)
                >= self.max_insns
    }

    #[inline]
    fn avail_insns(&self) -> usize {
        self.rem_insns.saturating_add(
            self.max_insns.saturating_sub(
                self.shared_insns
                    .0
                    .load(std::sync::atomic::Ordering::Relaxed),
            ),
        )
    }

    #[inline]
    fn shared_imm(&self) -> &'a std::sync::atomic::AtomicUsize {
        &self.shared_imm_elems.0
    }
}

impl Drop for DecodeWorkerBudget<'_> {
    fn drop(&mut self) {
        if self.rem_insns > 0 {
            self.shared_insns
                .0
                .fetch_sub(self.rem_insns, std::sync::atomic::Ordering::Relaxed);
            self.rem_insns = 0;
        }
        if self.rem_locals > 0 {
            self.shared_locals
                .0
                .fetch_sub(self.rem_locals, std::sync::atomic::Ordering::Relaxed);
            self.rem_locals = 0;
        }
    }
}

pub(crate) fn reserve_budget(
    shared: &std::sync::atomic::AtomicUsize,
    max_total: usize,
    want: usize,
) -> usize {
    use std::sync::atomic::Ordering;
    let mut cur = shared.load(Ordering::Relaxed);
    loop {
        if cur >= max_total {
            return 0;
        }
        let grant = want.min(max_total - cur);
        match shared.compare_exchange_weak(cur, cur + grant, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return grant,
            Err(actual) => cur = actual,
        }
    }
}
pub(crate) fn try_reserve_exact_budget(
    shared: &std::sync::atomic::AtomicUsize,
    max_total: usize,
    want: usize,
) -> bool {
    use std::sync::atomic::Ordering;
    if want == 0 {
        return true;
    }
    let mut cur = shared.load(Ordering::Relaxed);
    loop {
        let Some(next) = cur.checked_add(want) else {
            return false;
        };
        if next > max_total {
            return false;
        }
        match shared.compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return true,
            Err(actual) => cur = actual,
        }
    }
}
enum ImmBudget<'a> {
    Local {
        used: &'a mut usize,
        max: usize,
    },
    Shared {
        used: &'a std::sync::atomic::AtomicUsize,
        max: usize,
    },
}
impl ImmBudget<'_> {
    fn try_reserve(&mut self, n: usize) -> bool {
        match self {
            ImmBudget::Local { used, max } => {
                **used = used.saturating_add(n);
                **used <= *max
            }
            ImmBudget::Shared { used, max } => try_reserve_exact_budget(used, *max, n),
        }
    }
}
/// A function body's instructions and the absolute offset of each in the
/// module. Pre-reserves quota from `worker_budget` BEFORE retaining
/// instructions or variable-length immediates so aggregate retained memory
/// across all parallel workers never exceeds `max_insns`. When `keep` is false
/// or quota is exhausted, syntax decoding continues in full with zero heap
/// allocation so malformed bytecode still reports `ErrorKind::Decode`.
pub(crate) fn body_expression_bounded(
    r: &mut Reader,
    mut worker_budget: Option<&mut DecodeWorkerBudget<'_>>,
    max_insns: usize,
    mut keep: bool,
) -> Result<BoundedBody> {
    let mut instructions = Vec::new();
    let mut offsets = Vec::new();
    let mut dropped = !keep;
    let mut imm_exceeded = false;
    let mut local_imm_elems: usize = 0;
    let shared_imm_elems = worker_budget.as_ref().map(|wb| wb.shared_imm());
    if keep
        && let Some(wb) = worker_budget.as_ref()
        && wb.insns_exhausted()
    {
        keep = false;
        dropped = true;
    }
    if keep {
        let avail = match worker_budget.as_ref() {
            Some(wb) => wb.avail_insns(),
            None => max_insns,
        };
        let estimate = (r.remaining() / 2).clamp(1, 1 << 16).min(avail);
        let _ = instructions.try_reserve_exact(estimate);
        let _ = offsets.try_reserve_exact(estimate);
    }
    // Check once that every byte offset within this body fits in `u32` (when the whole body
    // end offset fits in `u32`, no per-instruction `try_from` check is needed).
    let offsets_fit_u32 = r
        .offset()
        .checked_add(r.remaining())
        .is_some_and(|end| end <= u32::MAX as usize);
    let mut depth = 1usize;
    let mut count = 0usize;
    let mut body_imm_elems = 0usize;
    let mut saw_data_count_op = false;
    while depth > 0 {
        let offset = if offsets_fit_u32 {
            r.offset() as u32
        } else {
            u32::try_from(r.offset()).map_err(|_| {
                Error::new(
                    ErrorKind::ResourceLimit,
                    "module too large for u32 instruction offsets",
                )
            })?
        };
        let mut reserved_this_insn = false;
        if keep {
            if count >= max_insns {
                instructions = Vec::new();
                offsets = Vec::new();
                keep = false;
                dropped = true;
            } else if let Some(wb) = worker_budget.as_deref_mut() {
                if wb.consume_one_insn(r.remaining()) {
                    reserved_this_insn = true;
                } else {
                    instructions = Vec::new();
                    offsets = Vec::new();
                    keep = false;
                    dropped = true;
                }
            }
        }
        let mut imm_over = false;
        let mut step_imm = 0usize;
        let insn = match shared_imm_elems {
            Some(shared_imm) => {
                let mut budget = ImmBudget::Shared {
                    used: shared_imm,
                    max: max_insns,
                };
                read_instruction(
                    r,
                    &mut depth,
                    keep,
                    Some(&mut budget),
                    &mut step_imm,
                    &mut imm_over,
                )?
            }
            None => {
                let mut budget = ImmBudget::Local {
                    used: &mut local_imm_elems,
                    max: max_insns,
                };
                read_instruction(
                    r,
                    &mut depth,
                    keep,
                    Some(&mut budget),
                    &mut step_imm,
                    &mut imm_over,
                )?
            }
        };
        if insn.opcode >= 0xfc00 && matches!(insn.opcode, 0xfc08 | 0xfc09) {
            saw_data_count_op = true;
        }
        count = count.saturating_add(1);
        body_imm_elems = body_imm_elems.saturating_add(step_imm);
        if imm_over {
            imm_exceeded = true;
            if keep {
                if reserved_this_insn && let Some(wb) = worker_budget.as_deref_mut() {
                    wb.rem_insns = wb.rem_insns.saturating_add(1);
                }
                instructions = Vec::new();
                offsets = Vec::new();
                keep = false;
                dropped = true;
            }
        } else if keep {
            instructions.push(insn);
            offsets.push(offset);
        }
    }
    if instructions.capacity() > instructions.len().saturating_mul(2).max(16) {
        instructions.shrink_to_fit();
        offsets.shrink_to_fit();
    }
    Ok(BoundedBody {
        instructions,
        offsets,
        count,
        imm_elems: body_imm_elems,
        saw_data_count_op,
        dropped,
        imm_exceeded,
    })
}
#[cfg(test)]
fn expression_with_capacity(r: &mut Reader, capacity: usize) -> Result<Vec<Instruction>> {
    let mut out = Vec::new();
    // A hint only: a failed reservation leaves growth to push.
    let _ = out.try_reserve_exact(capacity);
    let mut depth = 1usize;
    let mut imm_over = false;
    let mut step_imm = 0usize;
    while depth > 0 {
        out.push(read_instruction(
            r,
            &mut depth,
            true,
            None,
            &mut step_imm,
            &mut imm_over,
        )?);
    }
    Ok(out)
}
/// Decode one instruction and its immediates, without validating control nesting.
pub fn decode_instruction(r: &mut Reader) -> Result<Instruction> {
    let mut imm_over = false;
    let mut step_imm = 0usize;
    read_instruction(r, &mut 1, true, None, &mut step_imm, &mut imm_over)
}
#[inline(always)]
fn read_instruction(
    r: &mut Reader,
    depth: &mut usize,
    keep: bool,
    imm_budget: Option<&mut ImmBudget<'_>>,
    step_imm: &mut usize,
    imm_over: &mut bool,
) -> Result<Instruction> {
    let first = r.byte()?;
    let immediate = match first {
        0x00 | 0x01 | 0x05 | 0x0a | 0x0f | 0x1a | 0x1b | 0x45..=0xc4 | 0xd1 | 0xd3 | 0xd4 => {
            Immediate::None
        }
        0x0b => {
            *depth -= 1;
            Immediate::None
        }
        0x0c
        | 0x0d
        | 0x10
        | 0x12
        | 0x14
        | 0x15
        | 0x20..=0x26
        | 0x3f
        | 0x40
        | 0xd2
        | 0xd5
        | 0xd6
        | 0x08
        | 0xe0
        | 0xe2 => Immediate::Index(r.u32()?),
        0x41 => Immediate::I32(r.signed(32)? as i32),
        0x42 => Immediate::I64(r.signed(64)?),
        0x28..=0x3e => memarg(r)?,
        0x02..=0x04 => {
            *depth += 1;
            if matches!(r.peek()?, 0x63 | 0x64) {
                Immediate::BlockType(val(r)?)
            } else {
                Immediate::Block(r.signed(33)?)
            }
        }
        0x11 | 0x13 | 0xe1 | 0xe6 => Immediate::Pair(r.u32()?, r.u32()?),
        0x43 => Immediate::F32(u32::from_le_bytes(r.take(4)?.try_into().unwrap())),
        0x44 => Immediate::F64(u64::from_le_bytes(r.take(8)?.try_into().unwrap())),
        0xd0 => Immediate::Heap(heap(r)?),
        _ => return read_instruction_cold(first, r, depth, keep, imm_budget, step_imm, imm_over),
    };
    Ok(Instruction {
        opcode: first as u16,
        immediate,
    })
}

#[cold]
#[inline(never)]
fn read_instruction_cold(
    first: u8,
    r: &mut Reader,
    depth: &mut usize,
    keep: bool,
    imm_budget: Option<&mut ImmBudget<'_>>,
    step_imm: &mut usize,
    imm_over: &mut bool,
) -> Result<Instruction> {
    let mut opcode = first as u16;
    let immediate = match first {
        0x0e => {
            let n = count(r)?;
            *step_imm = n;
            let allowed = if keep {
                match imm_budget {
                    Some(b) => {
                        let ok = b.try_reserve(n);
                        if !ok {
                            *imm_over = true;
                        }
                        ok
                    }
                    None => true,
                }
            } else {
                false
            };
            if allowed {
                let mut v = Vec::new();
                let _ = v.try_reserve_exact(n);
                for _ in 0..n {
                    v.push(r.u32()?);
                }
                Immediate::BrTable(v.into_boxed_slice(), r.u32()?)
            } else {
                for _ in 0..n {
                    let _ = r.u32()?;
                }
                let _ = r.u32()?;
                Immediate::None
            }
        }
        0x1c => {
            let n = count(r)?;
            *step_imm = n;
            let allowed = if keep {
                match imm_budget {
                    Some(b) => {
                        let ok = b.try_reserve(n);
                        if !ok {
                            *imm_over = true;
                        }
                        ok
                    }
                    None => true,
                }
            } else {
                false
            };
            if allowed {
                let mut v = Vec::new();
                let _ = v.try_reserve_exact(n);
                for _ in 0..n {
                    v.push(val(r)?);
                }
                Immediate::Types(v.into_boxed_slice())
            } else {
                for _ in 0..n {
                    let _ = val(r)?;
                }
                Immediate::None
            }
        }
        0xfc => {
            let sub = r.u32()?;
            if sub > 22 || sub == 18 {
                return Err(r.error("unknown miscellaneous opcode"));
            }
            opcode = 0xfc00 | sub as u16;
            match sub {
                0..=7 | 19..=22 => Immediate::None,
                8 | 10 | 12 | 14 => Immediate::Pair(r.u32()?, r.u32()?),
                _ => Immediate::Index(r.u32()?),
            }
        }
        0xfd => {
            let sub = r.u32()?;
            if sub > 0x113
                || matches!(
                    sub,
                    154 | 162
                        | 165
                        | 166
                        | 175
                        | 176
                        | 178
                        | 179
                        | 180
                        | 187
                        | 194
                        | 197
                        | 198
                        | 207
                        | 208
                        | 210
                        | 211
                        | 212
                        | 226
                        | 238
                )
            {
                return Err(r.error("unsupported SIMD opcode"));
            }
            opcode = if sub >= 256 {
                0xfa00 | (sub as u16 - 256)
            } else {
                0xfd00 | sub as u16
            };
            match sub {
                0..=11 | 92 | 93 => memarg(r)?,
                12 | 13 => {
                    let bytes = r.take(16)?;
                    if keep {
                        Immediate::Bytes(bytes.into())
                    } else {
                        Immediate::None
                    }
                }
                21..=34 => Immediate::Index(r.byte()? as u32),
                84..=91 => {
                    let m = memarg(r)?;
                    let lane = r.byte()?;
                    if let Immediate::MemArg {
                        align,
                        offset,
                        memory,
                    } = m
                    {
                        Immediate::MemLane {
                            align,
                            offset,
                            memory,
                            lane,
                        }
                    } else {
                        return Err(r.error("invalid memory lane"));
                    }
                }
                14..=20 | 35..=83 | 94..=255 | 256..=275 => Immediate::None,
                _ => return Err(r.error("unknown SIMD opcode")),
            }
        }
        0xfb => {
            let sub = r.u32()?;
            if sub > 38 || sub == 31 {
                return Err(r.error("unknown GC opcode"));
            }
            opcode = 0xfb00 | sub as u16;
            match sub {
                0 | 1 | 6 | 7 | 11..=14 | 16 | 32..=34 => Immediate::Index(r.u32()?),
                2..=5 | 8..=10 | 17..=19 => Immediate::Pair(r.u32()?, r.u32()?),
                20..=23 | 35 | 36 => Immediate::Heap(heap(r)?),
                24 | 25 | 37 | 38 => {
                    let flags = r.byte()?;
                    if flags > 3 {
                        return Err(r.error("invalid cast flags"));
                    }
                    Immediate::Cast {
                        flags,
                        label: r.u32()?,
                        from: heap(r)?,
                        to: heap(r)?,
                    }
                }
                15 | 26..=30 => Immediate::None,
                _ => return Err(r.error("unknown GC opcode")),
            }
        }
        0xe3..=0xe5 => {
            let ty = r.u32()?;
            let tag = if first == 0xe4 { Some(r.u32()?) } else { None };
            let n = count(r)?;
            *step_imm = n;
            let allowed = if keep {
                match imm_budget {
                    Some(b) => {
                        let ok = b.try_reserve(n);
                        if !ok {
                            *imm_over = true;
                        }
                        ok
                    }
                    None => true,
                }
            } else {
                false
            };
            if allowed {
                let mut handlers = Vec::new();
                let _ = handlers.try_reserve_exact(n);
                for _ in 0..n {
                    let switch = r.byte()?;
                    if switch > 1 {
                        return Err(r.error("invalid continuation handler"));
                    }
                    let t = r.u32()?;
                    handlers.push((t, if switch == 0 { Some(r.u32()?) } else { None }));
                }
                Immediate::Resume(Box::new(ResumeImmediate { ty, tag, handlers }))
            } else {
                for _ in 0..n {
                    let switch = r.byte()?;
                    if switch > 1 {
                        return Err(r.error("invalid continuation handler"));
                    }
                    let _ = r.u32()?;
                    if switch == 0 {
                        let _ = r.u32()?;
                    }
                }
                Immediate::None
            }
        }
        0x1f => {
            *depth += 1;
            let block = if matches!(r.peek()?, 0x63 | 0x64) {
                Immediate::BlockType(val(r)?)
            } else {
                Immediate::Block(r.signed(33)?)
            };
            let n = count(r)?;
            *step_imm = n;
            let allowed = if keep {
                match imm_budget {
                    Some(b) => {
                        let ok = b.try_reserve(n);
                        if !ok {
                            *imm_over = true;
                        }
                        ok
                    }
                    None => true,
                }
            } else {
                false
            };
            if allowed {
                let mut catches = Vec::new();
                let _ = catches.try_reserve_exact(n);
                for _ in 0..n {
                    let kind = r.byte()?;
                    if kind > 3 {
                        return Err(r.error("invalid catch kind"));
                    }
                    catches.push(Catch {
                        kind,
                        tag: if kind < 2 { Some(r.u32()?) } else { None },
                        label: r.u32()?,
                    });
                }
                Immediate::TryTable(Box::new(TryTableImmediate { block, catches }))
            } else {
                for _ in 0..n {
                    let kind = r.byte()?;
                    if kind > 3 {
                        return Err(r.error("invalid catch kind"));
                    }
                    if kind < 2 {
                        let _ = r.u32()?;
                    }
                    let _ = r.u32()?;
                }
                Immediate::None
            }
        }
        0xfe => {
            let sub = r.u32()?;
            opcode = 0xfe00 | sub as u16;
            match sub {
                0..=2 | 16..=78 => memarg(r)?,
                3 => {
                    if r.byte()? != 0 {
                        return Err(r.error("invalid atomic fence immediate"));
                    }
                    Immediate::None
                }
                _ => return Err(r.error("unknown atomic opcode")),
            }
        }
        _ => return Err(r.error("unknown opcode")),
    };
    Ok(Instruction { opcode, immediate })
}

#[cfg(test)]
mod ledger_tests {
    use super::*;
    fn encoding(key: &str) -> Vec<u8> {
        let mut pieces = key.split(':');
        let mut bytes = vec![u8::from_str_radix(pieces.next().unwrap(), 16).unwrap()];
        if let Some(sub) = pieces.next() {
            let mut n: u32 = sub.parse().unwrap();
            loop {
                let b = (n & 127) as u8;
                n >>= 7;
                bytes.push(b | if n > 0 { 128 } else { 0 });
                if n == 0 {
                    break;
                }
            }
        }
        bytes.extend([0; 64]);
        bytes
    }
    #[test]
    #[ignore = "requires opcode ledger environment from opcode_coverage.py"]
    fn t008_actual_dispatch_coverage() {
        let Ok(assigned) = std::env::var("WTIME_ASSIGNED_OPCODES") else {
            return;
        };
        let reserved = std::env::var("WTIME_RESERVED_OPCODES").unwrap();
        for key in assigned.split(',').filter(|s| !s.is_empty()) {
            decode_instruction(&mut Reader::new(&encoding(key)))
                .unwrap_or_else(|e| panic!("missing decoder handler for {key}: {e}"));
        }
        for key in reserved.split(',').filter(|s| !s.is_empty()) {
            assert!(
                decode_instruction(&mut Reader::new(&encoding(key))).is_err(),
                "reserved opcode {key} acquired a decoder handler"
            );
        }
    }
}
