//! Straight-line runs of the Decoded tier.
//!
//! After the dispatch loop has charged the instruction at a frame's program
//! counter, consecutive instructions that cannot change frames
//! or instances run here without returning to the loop head, as do structured
//! control instructions that stay within the frame. Every
//! instruction is still charged exactly once, before it executes, with the
//! same checks as `accounting::next_source`, so fuel, cancellation and the
//! operand-stack limit behave as in the general loop. Each opcode computes
//! what the general loop computes for it, with the same errors.
use super::*;

/// How a straight-line run ended.
pub(super) enum Straight {
    /// The charged instruction is not a straight-line one; nothing ran.
    /// It has been counted by the diagnostic counters.
    Declined,
    /// Instructions ran; the next one is not a straight-line one and has
    /// already been charged and counted by the diagnostic counters.
    Charged,
    /// Instructions ran up to the end of the body; nothing is charged.
    Ran,
}

impl Store {
    /// Runs instructions of `frame` from its program counter while they are
    /// straight-line instructions. The instruction at the program counter has
    /// already been charged.
    pub(super) fn run_straight(
        &mut self,
        inst_id: usize,
        frame: &mut Frame,
        module: &Module,
        ops: &[Instruction],
        stack: &mut Vec<Value>,
        fuel: &mut Option<u64>,
    ) -> Result<Straight> {
        let mut pc = frame.pc;
        let mut local_fuel = *fuel;
        // A token owned for the run is measurably faster than a shared
        // reference to one (9-13% wall time on sieve and sudoku), although
        // the clone costs an atomic increment and decrement per run.
        let cancel = self.cancellation.clone();
        let result = self.run_straight_inner(inst_id, frame, module, ops, stack, &mut local_fuel, &mut pc, &cancel.0);
        frame.pc = pc;
        *fuel = local_fuel;
        self.current_fuel = local_fuel;
        result
    }
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn run_straight_inner(
        &mut self,
        inst_id: usize,
        frame: &mut Frame,
        module: &Module,
        ops: &[Instruction],
        stack: &mut Vec<Value>,
        fuel: &mut Option<u64>,
        pc_ref: &mut usize,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Straight> {
        // `next_source`, less what a run cannot change: fuel resumption and
        // prompt retirement only follow continuation instructions, so the
        // charge of the run's first instruction settled them for the run.
        let max_height = self.engine.config.limits.max_stack_height;
        // Store index of the instance's memory 0, resolved at its first access:
        // nothing in a run can rebind it.
        let mut memory0: Option<usize> = None;
        let mut first = true;
        // Errors in the order numeric::exec reports them.
        macro_rules! unary {
            ($v:ident, |$a:ident| $e:expr) => {{
                let slot = stack.last_mut().ok_or_else(|| trap("operand stack underflow"))?;
                let Value::$v($a) = *slot else {
                    return Err(trap("numeric type mismatch"));
                };
                *slot = $e;
            }};
        }
        macro_rules! binary {
            ($v:ident, |$a:ident, $b:ident| $e:expr) => {{
                let $b = match stack.pop() {
                    Some(Value::$v(b)) => b,
                    Some(_) => return Err(trap("numeric type mismatch")),
                    None => return Err(trap("operand stack underflow")),
                };
                let slot = stack.last_mut().ok_or_else(|| trap("operand stack underflow"))?;
                let Value::$v($a) = *slot else {
                    return Err(trap("numeric type mismatch"));
                };
                *slot = $e;
            }};
        }
        let body = frame.function.body;
        let lowered: &[lowered::Op] = module.data.lowered[body].get_or_init(|| lowered::lower(module, body));
        let Some(mut lop) = lowered.get(*pc_ref).copied() else {
            return Ok(Straight::Declined);
        };
        loop {
            let pc = *pc_ref;
            // Counted before it runs, as the general loop counts it, so that
            // an instruction that traps is counted too. One that is declined
            // below is not counted again by the general loop.
            #[cfg(feature = "performance-counters")]
            {
                self.perf.record(perf::Event::Dispatch, 1);
                self.perf.record(perf::Event::InstructionClone, 1);
                self.perf.record(
                    perf::Event::InstructionBytes,
                    perf::immediate_copy_bytes(&ops[pc].immediate),
                );
            }
            #[cfg(feature = "opcode-coverage")]
            crate::coverage::record(ops[pc].opcode);
            *pc_ref = pc + 1;
            // Leaves the instruction at the program counter for the general
            // loop: charged, unless it is the instruction the loop charged.
            macro_rules! decline {
                () => {{
                    *pc_ref = pc;
                    return Ok(if first { Straight::Declined } else { Straight::Charged });
                }};
            }
            // A branch to label `depth`, as the general loop takes it; one
            // that leaves the function stays there.
            macro_rules! branch {
                ($depth:expr) => {{
                    let depth = $depth as usize;
                    let pos = frame.labels.len().checked_sub(depth + 1).ok_or_else(|| trap("branch depth"))?;
                    let l = &frame.labels[pos];
                    keep(stack, l.height, if l.loop_ { l.params } else { l.results })?;
                    *pc_ref = if l.loop_ { l.start } else { l.end + 1 };
                    let retain = pos + usize::from(l.loop_);
                    frame.labels.truncate(retain)
                }};
            }
            macro_rules! label {
                ($end:expr, $params:expr, $results:expr, $loop_:expr) => {{
                    let a = $params as usize;
                    let height = stack.len().checked_sub(a).ok_or_else(|| trap("block stack underflow"))?;
                    frame.labels.push(Label {
                        catches: vec![],
                        start: pc + 1,
                        end: $end as usize,
                        height,
                        params: a,
                        results: $results as usize,
                        loop_: $loop_,
                    });
                }};
            }
            // Single instructions, shared by their own ops and fused pairs.
            macro_rules! local_get {
                ($x:expr) => {{
                    let value = *frame.locals.get($x as usize).ok_or_else(|| trap("local index"))?;
                    stack.push(value)
                }};
            }
            macro_rules! local_set {
                ($x:expr) => {{
                    let value = pop(stack)?;
                    *frame.locals.get_mut($x as usize).ok_or_else(|| trap("local index"))? = value
                }};
            }
            macro_rules! global_get {
                ($x:expr) => {{
                    stack.push(self.get_global(self.instances[inst_id].globals[$x as usize])?)
                }};
            }
            macro_rules! global_set {
                ($x:expr) => {{
                    let v = pop(stack)?;
                    let global = self.instances[inst_id].globals[$x as usize];
                    // A number into a mutable global of its type, as set_global
                    // accepts it; anything else takes set_global's checks.
                    let i = self.check(global.0, &self.globals)?;
                    let slot = &mut self.globals[i];
                    if slot.ty.mutable && !matches!(v, Value::Ref(_)) && v.ty() == slot.ty.value {
                        slot.value = v
                    } else {
                        self.set_global(global, v)?
                    }
                }};
            }
            // Charges instruction `pc + k` of a fused group, as the bottom of
            // the loop charges the next instruction, and counts it; `extra`
            // operands of the group are held outside the operand stack.
            macro_rules! charge_at {
                ($k:literal, $extra:literal) => {{
                    *pc_ref = pc + $k;
                    if cancel.load(std::sync::atomic::Ordering::Acquire) {
                        return Err(err(ErrorKind::ResourceLimit, "execution cancelled"));
                    }
                    if let Some(n) = fuel {
                        if *n == 0 {
                            return Err(err(ErrorKind::ResourceLimit, "fuel exhausted"));
                        }
                        *n -= 1;
                    }
                    if stack.len() + $extra > max_height {
                        return Err(err(ErrorKind::ResourceLimit, "operand stack limit"));
                    }
                    #[cfg(feature = "performance-counters")]
                    {
                        self.perf.record(perf::Event::Dispatch, 1);
                        self.perf.record(perf::Event::InstructionClone, 1);
                        self.perf.record(
                            perf::Event::InstructionBytes,
                            perf::immediate_copy_bytes(&ops[pc + $k].immediate),
                        );
                    }
                    #[cfg(feature = "opcode-coverage")]
                    crate::coverage::record(ops[pc + $k].opcode);
                    *pc_ref = pc + $k + 1;
                }};
            }
            macro_rules! charge_second {
                () => {
                    charge_at!(1, 0)
                };
            }
            macro_rules! local {
                ($x:expr) => {
                    *frame.locals.get($x as usize).ok_or_else(|| trap("local index"))?
                };
            }
            macro_rules! set_local {
                ($x:expr, $v:expr) => {{
                    let value = $v;
                    *frame.locals.get_mut($x as usize).ok_or_else(|| trap("local index"))? = value
                }};
            }
            // A load or store on memory 0 with its operands on the stack.
            macro_rules! memory0 {
                ($opcode:expr, $offset:expr) => {{
                    let index = *memory0.get_or_insert_with(|| self.instances[inst_id].mems[0].0.index);
                    self.memory_access_raw(index, $opcode, $offset, stack)?
                }};
            }
            // The value of a load on memory 0 from address operand `addr`,
            // held outside the operand stack.
            macro_rules! load0 {
                ($opcode:expr, $offset:expr, $addr:expr) => {{
                    let index = *memory0.get_or_insert_with(|| self.instances[inst_id].mems[0].0.index);
                    let addr = $addr;
                    match self.load_fast(index, $opcode, addr, $offset) {
                        Some(v) => v,
                        None => {
                            stack.push(addr);
                            self.memory_slow(index, $opcode, $offset, stack)?;
                            pop(stack)?
                        }
                    }
                }};
            }
            match lop {
                lowered::Op::LocalSetGet { set, get } => {
                    local_set!(set);
                    charge_second!();
                    local_get!(get)
                }
                lowered::Op::LocalGetGet { first, second } => {
                    local_get!(first);
                    charge_second!();
                    local_get!(second)
                }
                lowered::Op::LocalGetI32Const { local, value } => {
                    local_get!(local);
                    charge_second!();
                    stack.push(Value::I32(value))
                }
                lowered::Op::I32ConstAdd(value) => {
                    stack.push(Value::I32(value));
                    charge_second!();
                    binary!(I32, |a, b| Value::I32(a.wrapping_add(b)))
                }
                lowered::Op::GlobalGetLocalSet { global, local } => {
                    global_get!(global);
                    charge_second!();
                    local_set!(local)
                }
                lowered::Op::I32AddLocalSet(local) => {
                    binary!(I32, |a, b| Value::I32(a.wrapping_add(b)));
                    charge_second!();
                    local_set!(local)
                }
                lowered::Op::LocalGetLocalSet { from, to } => {
                    local_get!(from);
                    charge_second!();
                    local_set!(to)
                }
                lowered::Op::LocalGetGlobalSet { local, global } => {
                    local_get!(local);
                    charge_second!();
                    global_set!(global)
                }
                lowered::Op::LocalConstBin { local, value, opcode } => {
                    let a = local!(local);
                    charge_at!(1, 1);
                    charge_at!(2, 2);
                    stack.push(const_binop(opcode, a, value)?)
                }
                lowered::Op::LocalConstBinSet { local, value, to, opcode } => {
                    let a = local!(local);
                    charge_at!(1, 1);
                    charge_at!(2, 2);
                    let result = const_binop(opcode, a, value)?;
                    charge_at!(3, 1);
                    set_local!(to, result)
                }
                lowered::Op::LocalLoad { local, offset, opcode } => {
                    let a = local!(local);
                    charge_at!(1, 1);
                    let v = load0!(opcode, u64::from(offset), a);
                    stack.push(v)
                }
                lowered::Op::LocalLoadSet { local, offset, to, opcode } => {
                    let a = local!(local);
                    charge_at!(1, 1);
                    let v = load0!(opcode, u64::from(offset), a);
                    charge_at!(2, 1);
                    set_local!(to, v)
                }
                lowered::Op::LocalLocalStore { addr, value, offset, opcode } => {
                    let a = local!(addr);
                    charge_at!(1, 1);
                    let v = local!(value);
                    charge_at!(2, 2);
                    let index = *memory0.get_or_insert_with(|| self.instances[inst_id].mems[0].0.index);
                    if !self.store_fast(index, opcode, a, v, u64::from(offset)) {
                        stack.push(a);
                        stack.push(v);
                        self.memory_slow(index, opcode, u64::from(offset), stack)?
                    }
                }
                lowered::Op::Leave => decline!(),
                lowered::Op::Nop => {}
                lowered::Op::Block { end, params, results } => label!(end, params, results, false),
                lowered::Op::Loop { end, params, results } => label!(end, params, results, true),
                lowered::Op::If { end, other, params, results } => {
                    let cond = address(pop(stack)?)? != 0;
                    label!(end, params, results, false);
                    if !cond {
                        *pc_ref = if other == lowered::NO_ELSE { end as usize } else { other as usize + 1 }
                    }
                }
                lowered::Op::Else => *pc_ref = frame.labels.last().ok_or_else(|| trap("else without label"))?.end,
                lowered::Op::End => {
                    let Some(l) = frame.labels.pop() else { decline!() };
                    #[cfg(feature = "performance-counters")]
                    self.perf.record(perf::Event::Branch, u64::from(l.results != 0));
                    keep(stack, l.height, l.results)?;
                }
                lowered::Op::Br(depth) => {
                    if depth as usize == frame.labels.len() {
                        decline!()
                    }
                    branch!(depth)
                }
                lowered::Op::BrIf(depth) => {
                    // The condition is read without consuming it, so that a
                    // branch out of the function can be left untouched.
                    let taken = address(*stack.last().ok_or_else(|| trap("operand stack underflow"))?)? != 0;
                    if taken && depth as usize == frame.labels.len() {
                        decline!()
                    }
                    stack.pop();
                    if taken {
                        branch!(depth)
                    }
                }
                lowered::Op::Drop => {
                    pop(stack)?;
                }
                lowered::Op::Select => {
                    let c = address(pop(stack)?)?;
                    let b = pop(stack)?;
                    let a = pop(stack)?;
                    stack.push(if c != 0 { a } else { b })
                }
                lowered::Op::LocalGet(x) => local_get!(x),
                lowered::Op::LocalSet(x) => local_set!(x),
                lowered::Op::LocalTee(x) => {
                    let value = *stack.last().ok_or_else(|| trap("empty stack"))?;
                    *frame.locals.get_mut(x as usize).ok_or_else(|| trap("local index"))? = value
                }
                lowered::Op::GlobalGet(x) => global_get!(x),
                lowered::Op::GlobalSet(x) => global_set!(x),
                lowered::Op::Memory0 { opcode, offset } => memory0!(opcode, offset),
                lowered::Op::I32Const(x) => stack.push(Value::I32(x)),
                lowered::Op::I64Const(x) => stack.push(Value::I64(x)),
                lowered::Op::F32Const(x) => stack.push(Value::F32(x)),
                lowered::Op::F64Const(x) => stack.push(Value::F64(x)),
                lowered::Op::I32Eqz => {
                    let slot = stack.last_mut().ok_or_else(|| trap("operand stack underflow"))?;
                    let Value::I32(a) = *slot else {
                        return Err(trap("numeric type mismatch"));
                    };
                    *slot = Value::I32((a == 0) as i32)
                }
                lowered::Op::Int(opcode) => match opcode {
                    0x46 => binary!(I32, |a, b| Value::I32((a == b) as i32)),
                    0x47 => binary!(I32, |a, b| Value::I32((a != b) as i32)),
                    0x48 => binary!(I32, |a, b| Value::I32((a < b) as i32)),
                    0x49 => binary!(I32, |a, b| Value::I32(((a as u32) < (b as u32)) as i32)),
                    0x4a => binary!(I32, |a, b| Value::I32((a > b) as i32)),
                    0x4b => binary!(I32, |a, b| Value::I32(((a as u32) > (b as u32)) as i32)),
                    0x4c => binary!(I32, |a, b| Value::I32((a <= b) as i32)),
                    0x4d => binary!(I32, |a, b| Value::I32(((a as u32) <= (b as u32)) as i32)),
                    0x4e => binary!(I32, |a, b| Value::I32((a >= b) as i32)),
                    0x4f => binary!(I32, |a, b| Value::I32(((a as u32) >= (b as u32)) as i32)),
                    0x6a => binary!(I32, |a, b| Value::I32(a.wrapping_add(b))),
                    0x6b => binary!(I32, |a, b| Value::I32(a.wrapping_sub(b))),
                    0x6c => binary!(I32, |a, b| Value::I32(a.wrapping_mul(b))),
                    0x71 => binary!(I32, |a, b| Value::I32(a & b)),
                    0x72 => binary!(I32, |a, b| Value::I32(a | b)),
                    0x73 => binary!(I32, |a, b| Value::I32(a ^ b)),
                    0x74 => binary!(I32, |a, b| Value::I32(a.wrapping_shl(b as u32))),
                    0x75 => binary!(I32, |a, b| Value::I32(a.wrapping_shr(b as u32))),
                    0x76 => binary!(I32, |a, b| Value::I32((a as u32).wrapping_shr(b as u32) as i32)),
                    0x51 => binary!(I64, |a, b| Value::I32((a == b) as i32)),
                    0x52 => binary!(I64, |a, b| Value::I32((a != b) as i32)),
                    0x53 => binary!(I64, |a, b| Value::I32((a < b) as i32)),
                    0x54 => binary!(I64, |a, b| Value::I32(((a as u64) < (b as u64)) as i32)),
                    0x55 => binary!(I64, |a, b| Value::I32((a > b) as i32)),
                    0x56 => binary!(I64, |a, b| Value::I32(((a as u64) > (b as u64)) as i32)),
                    0x57 => binary!(I64, |a, b| Value::I32((a <= b) as i32)),
                    0x58 => binary!(I64, |a, b| Value::I32(((a as u64) <= (b as u64)) as i32)),
                    0x59 => binary!(I64, |a, b| Value::I32((a >= b) as i32)),
                    0x5a => binary!(I64, |a, b| Value::I32(((a as u64) >= (b as u64)) as i32)),
                    0xa7 => unary!(I64, |a| Value::I32(a as i32)),
                    0xac => unary!(I32, |a| Value::I64(a as i64)),
                    0xad => unary!(I32, |a| Value::I64(a as u32 as i64)),
                    0x7c => binary!(I64, |a, b| Value::I64(a.wrapping_add(b))),
                    0x7d => binary!(I64, |a, b| Value::I64(a.wrapping_sub(b))),
                    0x7e => binary!(I64, |a, b| Value::I64(a.wrapping_mul(b))),
                    0x83 => binary!(I64, |a, b| Value::I64(a & b)),
                    0x84 => binary!(I64, |a, b| Value::I64(a | b)),
                    0x86 => binary!(I64, |a, b| Value::I64(a.wrapping_shl(b as u32))),
                    0x88 => binary!(I64, |a, b| Value::I64((a as u64).wrapping_shr(b as u32) as i64)),
                    _ => numeric::exec(opcode, stack)?,
                },
                lowered::Op::Numeric(opcode) => numeric::exec(opcode, stack)?,
                // Run from the source instruction, exactly as before lowering.
                lowered::Op::Slow => {
                            let op = &ops[pc];
                            let opcode = op.opcode;
                            match opcode {
                        0x01 => {}
                        // Structured control within the frame, as the general loop
                        // runs it; whatever would leave the function stays there.
                        0x02..=0x04 => {
                            let (a, r) = arity(&op.immediate, module);
                            let cond = if opcode == 4 { address(pop(stack)?)? != 0 } else { true };
                            let (end, other) = module.data.control.get(frame.function.body, pc)
                                .ok_or_else(|| trap("missing control boundary"))?;
                            let height = stack.len().checked_sub(a).ok_or_else(|| trap("block stack underflow"))?;
                            frame.labels.push(Label {
                                catches: vec![],
                                start: pc + 1,
                                end,
                                height,
                                params: a,
                                results: r,
                                loop_: opcode == 3,
                            });
                            if !cond {
                                *pc_ref = other.map_or(end, |x| x + 1)
                            }
                        }
                        0x05 => *pc_ref = frame.labels.last().ok_or_else(|| trap("else without label"))?.end,
                        0x0b => {
                            let Some(l) = frame.labels.pop() else { decline!() };
                            #[cfg(feature = "performance-counters")]
                            self.perf.record(perf::Event::Branch, u64::from(l.results != 0));
                            keep(stack, l.height, l.results)?;
                        }
                        0x0c..=0x0e => {
                            // The depth, read without consuming the operand, so that a
                            // branch out of the function can be left untouched.
                            let operand = || -> Result<u64> {
                                address(*stack.last().ok_or_else(|| trap("operand stack underflow"))?)
                            };
                            let depth = match &op.immediate {
                                Immediate::Index(x) if opcode == 0x0d => {
                                    if operand()? == 0 {
                                        stack.pop();
                                        None
                                    } else {
                                        Some(*x as usize)
                                    }
                                }
                                Immediate::Index(x) => Some(*x as usize),
                                Immediate::BrTable(v, d) => {
                                    let n = operand()?;
                                    Some(v.get(n as usize).copied().unwrap_or(*d) as usize)
                                }
                                _ => return Err(trap("branch immediate")),
                            };
                            if let Some(depth) = depth {
                                if depth == frame.labels.len() {
                                    decline!()
                                }
                                if opcode != 0x0c {
                                    stack.pop();
                                }
                                let pos = frame.labels.len().checked_sub(depth + 1).ok_or_else(|| trap("branch depth"))?;
                                let l = &frame.labels[pos];
                                keep(stack, l.height, if l.loop_ { l.params } else { l.results })?;
                                *pc_ref = if l.loop_ { l.start } else { l.end + 1 };
                                let retain = pos + usize::from(l.loop_);
                                frame.labels.truncate(retain)
                            }
                        }
                        0x1a => {
                            pop(stack)?;
                        }
                        0x1b | 0x1c => {
                            let c = address(pop(stack)?)?;
                            let b = pop(stack)?;
                            let a = pop(stack)?;
                            stack.push(if c != 0 { a } else { b })
                        }
                        0x20 => {
                            let Immediate::Index(x) = op.immediate else {
                                return Err(trap("index immediate"));
                            };
                            let value = *frame.locals.get(x as usize).ok_or_else(|| trap("local index"))?;
                            stack.push(value)
                        }
                        0x21 => {
                            let Immediate::Index(x) = op.immediate else {
                                return Err(trap("index immediate"));
                            };
                            let value = pop(stack)?;
                            *frame.locals.get_mut(x as usize).ok_or_else(|| trap("local index"))? = value
                        }
                        0x22 => {
                            let Immediate::Index(x) = op.immediate else {
                                return Err(trap("index immediate"));
                            };
                            let value = *stack.last().ok_or_else(|| trap("empty stack"))?;
                            *frame.locals.get_mut(x as usize).ok_or_else(|| trap("local index"))? = value
                        }
                        0x23 => {
                            let Immediate::Index(x) = op.immediate else {
                                return Err(trap("index immediate"));
                            };
                            stack.push(self.get_global(self.instances[inst_id].globals[x as usize])?)
                        }
                        0x24 => {
                            let Immediate::Index(x) = op.immediate else {
                                return Err(trap("index immediate"));
                            };
                            let v = pop(stack)?;
                            let global = self.instances[inst_id].globals[x as usize];
                            // A number into a mutable global of its type, as set_global
                            // accepts it; anything else takes set_global's checks.
                            let i = self.check(global.0, &self.globals)?;
                            let slot = &mut self.globals[i];
                            if slot.ty.mutable && !matches!(v, Value::Ref(_)) && v.ty() == slot.ty.value {
                                slot.value = v
                            } else {
                                self.set_global(global, v)?
                            }
                        }
                        0x28..=0x3e => match op.immediate {
                            Immediate::MemArg { memory: 0, .. } => {
                                let index = *memory0.get_or_insert_with(|| self.instances[inst_id].mems[0].0.index);
                                self.memory_access(index, op, stack)?
                            }
                            _ => self.load_store(inst_id, op, stack)?,
                        },
                        0x41 => {
                            if let Immediate::I32(x) = op.immediate {
                                stack.push(Value::I32(x))
                            }
                        }
                        0x42 => {
                            if let Immediate::I64(x) = op.immediate {
                                stack.push(Value::I64(x))
                            }
                        }
                        0x43 => {
                            if let Immediate::F32(x) = op.immediate {
                                stack.push(Value::F32(x))
                            }
                        }
                        0x44 => {
                            if let Immediate::F64(x) = op.immediate {
                                stack.push(Value::F64(x))
                            }
                        }
                        // The most frequent integer operators, as numeric::exec
                        // computes them; the rest go through it.
                        0x45 => {
                            let slot = stack.last_mut().ok_or_else(|| trap("operand stack underflow"))?;
                            let Value::I32(a) = *slot else {
                                return Err(trap("numeric type mismatch"));
                            };
                            *slot = Value::I32((a == 0) as i32)
                        }
                        0x46 => binary!(I32, |a, b| Value::I32((a == b) as i32)),
                        0x47 => binary!(I32, |a, b| Value::I32((a != b) as i32)),
                        0x48 => binary!(I32, |a, b| Value::I32((a < b) as i32)),
                        0x49 => binary!(I32, |a, b| Value::I32(((a as u32) < (b as u32)) as i32)),
                        0x4a => binary!(I32, |a, b| Value::I32((a > b) as i32)),
                        0x4b => binary!(I32, |a, b| Value::I32(((a as u32) > (b as u32)) as i32)),
                        0x4c => binary!(I32, |a, b| Value::I32((a <= b) as i32)),
                        0x4d => binary!(I32, |a, b| Value::I32(((a as u32) <= (b as u32)) as i32)),
                        0x4e => binary!(I32, |a, b| Value::I32((a >= b) as i32)),
                        0x4f => binary!(I32, |a, b| Value::I32(((a as u32) >= (b as u32)) as i32)),
                        0x6a => binary!(I32, |a, b| Value::I32(a.wrapping_add(b))),
                        0x6b => binary!(I32, |a, b| Value::I32(a.wrapping_sub(b))),
                        0x6c => binary!(I32, |a, b| Value::I32(a.wrapping_mul(b))),
                        0x71 => binary!(I32, |a, b| Value::I32(a & b)),
                        0x72 => binary!(I32, |a, b| Value::I32(a | b)),
                        0x73 => binary!(I32, |a, b| Value::I32(a ^ b)),
                        0x74 => binary!(I32, |a, b| Value::I32(a.wrapping_shl(b as u32))),
                        0x75 => binary!(I32, |a, b| Value::I32(a.wrapping_shr(b as u32))),
                        0x76 => binary!(I32, |a, b| Value::I32((a as u32).wrapping_shr(b as u32) as i32)),
                        0x51 => binary!(I64, |a, b| Value::I32((a == b) as i32)),
                        0x52 => binary!(I64, |a, b| Value::I32((a != b) as i32)),
                        0x53 => binary!(I64, |a, b| Value::I32((a < b) as i32)),
                        0x54 => binary!(I64, |a, b| Value::I32(((a as u64) < (b as u64)) as i32)),
                        0x7c => binary!(I64, |a, b| Value::I64(a.wrapping_add(b))),
                        0x7d => binary!(I64, |a, b| Value::I64(a.wrapping_sub(b))),
                        0x7e => binary!(I64, |a, b| Value::I64(a.wrapping_mul(b))),
                        0x83 => binary!(I64, |a, b| Value::I64(a & b)),
                        0x84 => binary!(I64, |a, b| Value::I64(a | b)),
                        0x86 => binary!(I64, |a, b| Value::I64(a.wrapping_shl(b as u32))),
                        0x88 => binary!(I64, |a, b| Value::I64((a as u64).wrapping_shr(b as u32) as i64)),
                        _ if (0x45..=0xc4).contains(&opcode) => numeric::exec(opcode, stack)?,
                        // Not a straight-line instruction: it stays at the program
                        // counter for the general loop, charged unless it is the
                        // instruction the loop charged.
                        _ => decline!(),
                    }
                }
            }
            first = false;
            // Charge the next instruction before it runs, with the checks of
            // `next_source` in its order.
            lop = match lowered.get(*pc_ref) {
                Some(&op) => op,
                // Validated code ends with `end`; let the general loop report it.
                None => return Ok(Straight::Ran),
            };
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                return Err(err(ErrorKind::ResourceLimit, "execution cancelled"));
            }
            if let Some(n) = fuel {
                if *n == 0 {
                    return Err(err(ErrorKind::ResourceLimit, "fuel exhausted"));
                }
                *n -= 1;
            }
            if stack.len() > max_height {
                return Err(err(ErrorKind::ResourceLimit, "operand stack limit"));
            }
        }
    }
}

/// `a op b` for an inline integer binop whose second operand is the
/// constant `b` (sign-extended for i64 operators), with the error the
/// operator reports when `a` is not of its type.
#[inline(always)]
fn const_binop(opcode: u16, a: Value, b: i32) -> Result<Value> {
    if lowered::i32_binop(opcode) {
        let Value::I32(a) = a else {
            return Err(trap("numeric type mismatch"));
        };
        Ok(match opcode {
            0x46 => Value::I32((a == b) as i32),
            0x47 => Value::I32((a != b) as i32),
            0x48 => Value::I32((a < b) as i32),
            0x49 => Value::I32(((a as u32) < (b as u32)) as i32),
            0x4a => Value::I32((a > b) as i32),
            0x4b => Value::I32(((a as u32) > (b as u32)) as i32),
            0x4c => Value::I32((a <= b) as i32),
            0x4d => Value::I32(((a as u32) <= (b as u32)) as i32),
            0x4e => Value::I32((a >= b) as i32),
            0x4f => Value::I32(((a as u32) >= (b as u32)) as i32),
            0x6a => Value::I32(a.wrapping_add(b)),
            0x6b => Value::I32(a.wrapping_sub(b)),
            0x6c => Value::I32(a.wrapping_mul(b)),
            0x71 => Value::I32(a & b),
            0x72 => Value::I32(a | b),
            0x73 => Value::I32(a ^ b),
            0x74 => Value::I32(a.wrapping_shl(b as u32)),
            0x75 => Value::I32(a.wrapping_shr(b as u32)),
            0x76 => Value::I32((a as u32).wrapping_shr(b as u32) as i32),
            _ => return Err(trap("fused operator")),
        })
    } else {
        let Value::I64(a) = a else {
            return Err(trap("numeric type mismatch"));
        };
        let b = i64::from(b);
        Ok(match opcode {
            0x51 => Value::I32((a == b) as i32),
            0x52 => Value::I32((a != b) as i32),
            0x53 => Value::I32((a < b) as i32),
            0x54 => Value::I32(((a as u64) < (b as u64)) as i32),
            0x55 => Value::I32((a > b) as i32),
            0x56 => Value::I32(((a as u64) > (b as u64)) as i32),
            0x57 => Value::I32((a <= b) as i32),
            0x58 => Value::I32(((a as u64) <= (b as u64)) as i32),
            0x59 => Value::I32((a >= b) as i32),
            0x5a => Value::I32(((a as u64) >= (b as u64)) as i32),
            0x7c => Value::I64(a.wrapping_add(b)),
            0x7d => Value::I64(a.wrapping_sub(b)),
            0x7e => Value::I64(a.wrapping_mul(b)),
            0x83 => Value::I64(a & b),
            0x84 => Value::I64(a | b),
            0x86 => Value::I64(a.wrapping_shl(b as u32)),
            0x88 => Value::I64((a as u64).wrapping_shr(b as u32) as i64),
            _ => return Err(trap("fused operator")),
        })
    }
}
