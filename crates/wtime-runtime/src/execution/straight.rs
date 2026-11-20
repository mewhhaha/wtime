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
        let Some(mut op) = ops.get(*pc_ref) else {
            return Ok(Straight::Declined);
        };
        loop {
            let opcode = op.opcode;
            // Counted before it runs, as the general loop counts it, so that
            // an instruction that traps is counted too. One that is declined
            // below is not counted again by the general loop.
            #[cfg(feature = "performance-counters")]
            {
                self.perf.record(perf::Event::Dispatch, 1);
                self.perf.record(perf::Event::InstructionClone, 1);
                self.perf.record(
                    perf::Event::InstructionBytes,
                    perf::immediate_copy_bytes(&op.immediate),
                );
            }
            #[cfg(feature = "opcode-coverage")]
            crate::coverage::record(opcode);
            let pc = *pc_ref;
            *pc_ref = pc + 1;
            // Leaves the instruction at the program counter for the general
            // loop: charged, unless it is the instruction the loop charged.
            macro_rules! decline {
                () => {{
                    *pc_ref = pc;
                    return Ok(if first { Straight::Declined } else { Straight::Charged });
                }};
            }
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
            first = false;
            // Charge the next instruction before it runs, with the checks of
            // `next_source` in its order.
            op = match ops.get(*pc_ref) {
                Some(op) => op,
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
