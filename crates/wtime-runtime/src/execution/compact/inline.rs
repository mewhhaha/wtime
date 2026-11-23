//! A fixed six-event i32 leaf; no activation allocation on the success path.
use super::*;
use crate::continuation::Prompt;
use crate::execution::{Frame, accounting};

const MAX_FUNCTIONS: usize = 128;
const MAX_LEAVES: usize = 64;
const MAX_PLAN_BYTES: usize = 16384;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Leaf {
    body: usize,
    multiplier: i32,
    addend: i32,
}
impl Leaf {
    fn op(self, pc: usize) -> CompactOp {
        match pc {
            0 => CompactOp {
                opcode: 0x20,
                form: INDEX,
                aux: 0,
                payload: 0,
            },
            1 => CompactOp {
                opcode: 0x41,
                form: I32_BITS,
                aux: 0,
                payload: self.multiplier as u32 as u64,
            },
            2 => CompactOp {
                opcode: 0x6c,
                form: NONE,
                aux: 0,
                payload: 0,
            },
            3 => CompactOp {
                opcode: 0x41,
                form: I32_BITS,
                aux: 0,
                payload: self.addend as u32 as u64,
            },
            4 => CompactOp {
                opcode: 0x6a,
                form: NONE,
                aux: 0,
                payload: 0,
            },
            5 => CompactOp {
                opcode: 0x0b,
                form: NONE,
                aux: 0,
                payload: 0,
            },
            _ => unreachable!("sealed numeric leaf source"),
        }
    }
}
#[derive(Debug)]
pub(super) struct Leaves {
    engine: u64,
    source: Arc<[u8]>,
    entries: Vec<Option<Leaf>>,
    pub(super) bytes: usize,
}
fn recognize(module: &Module, body: usize) -> Option<Leaf> {
    let decoded = module.data.inner.decoded();
    let code = decoded.code.get(body)?;
    if !code.locals.is_empty() || code.instructions.len() != 6 {
        return None;
    }
    let ty = decoded.types.get(decoded.functions.get(body)?.as_usize())?;
    if ty.params.as_slice() != [ValType::I32] || ty.results.as_slice() != [ValType::I32] {
        return None;
    }
    let ops = &code.instructions;
    if !matches!(ops[0].immediate, Immediate::Index(0))
        || ops[0].opcode != 0x20
        || ops[1].opcode != 0x41
        || ops[2].opcode != 0x6c
        || ops[3].opcode != 0x41
        || ops[4].opcode != 0x6a
        || ops[5].opcode != 0x0b
        || [2, 4, 5]
            .iter()
            .any(|&i| !matches!(ops[i].immediate, Immediate::None))
    {
        return None;
    }
    let (Immediate::I32(multiplier), Immediate::I32(addend)) =
        (ops[1].immediate.clone(), ops[3].immediate.clone())
    else {
        return None;
    };
    Some(Leaf {
        body,
        multiplier,
        addend,
    })
}
impl Leaves {
    // Charge both recognition passes and bounded verification. A declined
    // table still consumes the attempted charge, but never declines the core.
    pub(super) fn charge(module: &Module) -> Option<usize> {
        let count = module.data.inner.decoded().code.len();
        if count > MAX_FUNCTIONS {
            Some(0)
        } else {
            count.checked_mul(32)
        }
    }
    pub(super) fn prepare(module: &Module, max_work: usize, max_bytes: usize) -> Option<Self> {
        let count = module.data.inner.decoded().code.len();
        let work = Self::charge(module)?;
        if count > MAX_FUNCTIONS || work > max_work {
            return None;
        }
        let cap = max_bytes.min(MAX_PLAN_BYTES);
        let planned =
            std::mem::size_of::<Self>().checked_add(vector_bytes::<Option<Leaf>>(count)?)?;
        if planned > cap {
            return None;
        }
        let found = (0..count)
            .filter(|&body| recognize(module, body).is_some())
            .count();
        if found == 0 || found > MAX_LEAVES {
            return None;
        }
        let mut entries = Vec::new();
        entries.try_reserve_exact(count).ok()?;
        let bytes = std::mem::size_of::<Self>()
            .checked_add(vector_bytes::<Option<Leaf>>(entries.capacity())?)?;
        if bytes > cap {
            return None;
        }
        entries.extend((0..count).map(|body| recognize(module, body)));
        Some(Self {
            engine: module.data.engine,
            source: module.data.inner.decoded().bytes.clone(),
            entries,
            bytes,
        })
    }
    pub(super) fn get(&self, body: usize) -> Option<Leaf> {
        self.entries.get(body).copied().flatten()
    }
    pub(super) fn verify(&self, module: &Module, max_work: usize, max_bytes: usize) -> bool {
        if self.engine != module.data.engine
            || !Arc::ptr_eq(&self.source, &module.data.inner.decoded().bytes)
            || self.bytes > max_bytes
            || self.bytes > MAX_PLAN_BYTES
            || std::mem::size_of::<Self>().checked_add(
                vector_bytes::<Option<Leaf>>(self.entries.capacity()).unwrap_or(usize::MAX),
            ) != Some(self.bytes)
        {
            return false;
        }
        let Some(scratch) = max_bytes.checked_sub(self.bytes) else {
            return false;
        };
        let Some(expected) = Self::prepare(module, max_work, scratch) else {
            return false;
        };
        self.entries == expected.entries
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn try_leaf(
    store: &mut Store,
    fun: Func,
    caller_instance: usize,
    stack: &mut Vec<Value>,
    fuel: &mut Option<u64>,
    depth: usize,
    prompts: &mut Vec<Prompt>,
    stopped: &mut Option<Frame>,
) -> Result<bool> {
    let checked = store.check(fun.0, &store.funcs)?;
    let target = &store.funcs[checked];
    let leaf = (target.instance == caller_instance && target.host.is_none())
        .then(|| {
            store.instances[caller_instance]
                .module
                .data
                .compact
                .as_ref()?
                .inline_leaf(target.body)
        })
        .flatten();
    let Some(leaf) = leaf else {
        return decline(store);
    };
    if target.ty.params.as_slice() != [ValType::I32]
        || target.ty.results.as_slice() != [ValType::I32]
    {
        return decline(store);
    }
    let Some(start) = stack.len().checked_sub(1) else {
        return Err(trap("call stack underflow"));
    };
    if depth >= store.engine.config.limits.max_call_depth {
        return Err(err(ErrorKind::ResourceLimit, "call depth exhausted"));
    }
    if fuel.is_some_and(|n| n < 6)
        || start
            .checked_add(2)
            .is_none_or(|n| n > store.engine.config.limits.max_stack_height)
    {
        return decline(store);
    }
    #[cfg(feature = "execution-reference")]
    if store
        .materialization_request
        .is_some_and(|(instance, body, _, _)| instance == caller_instance && body == leaf.body)
    {
        return decline(store);
    }
    let Value::I32(argument) = stack[start] else {
        return Err(err(ErrorKind::Host, "argument type mismatch"));
    };
    stack.truncate(start);
    #[cfg(feature = "execution-reference")]
    {
        store.compact_inline_leaf_calls = store.compact_inline_leaf_calls.saturating_add(1);
    }
    let mut pending = [Value::I32(0), Value::I32(0)];
    let mut height = 0usize;
    for pc in 0..6 {
        #[cfg(test)]
        STOP_AT.with(|slot| {
            if let Some((at, fuel_stop)) = slot.get()
                && at == pc
            {
                slot.set(None);
                if fuel_stop {
                    *fuel = Some(0);
                } else {
                    store.cancellation.cancel();
                }
            }
        });
        let logical_height = stack
            .len()
            .checked_add(height)
            .ok_or_else(|| trap("inline operand height overflow"))?;
        if let Err(error) = accounting::next_source(store, fuel, depth + 1, logical_height, prompts)
        {
            let mut frame =
                super::super::calls::guest_frame(store, fun, &[Value::I32(argument)], start)?;
            frame.pc = pc;
            stack.extend_from_slice(&pending[..height]);
            *stopped = Some(frame);
            return Err(error);
        }
        super::execute::record(leaf.op(pc), store);
        #[cfg(feature = "execution-reference")]
        {
            store.compact_dispatches = store.compact_dispatches.saturating_add(1);
        }
        match pc {
            0 => {
                pending[0] = Value::I32(argument);
                height = 1;
            }
            1 => {
                pending[1] = Value::I32(leaf.multiplier);
                height = 2;
            }
            2 => {
                pending[0] = Value::I32(argument.wrapping_mul(leaf.multiplier));
                height = 1;
            }
            3 => {
                pending[1] = Value::I32(leaf.addend);
                height = 2;
            }
            4 => {
                let Value::I32(n) = pending[0] else {
                    unreachable!()
                };
                pending[0] = Value::I32(n.wrapping_add(leaf.addend));
                height = 1;
            }
            5 => {}
            _ => unreachable!(),
        }
    }
    stack.push(pending[0]);
    Ok(true)
}
fn decline(store: &mut Store) -> Result<bool> {
    #[cfg(feature = "execution-reference")]
    {
        store.compact_inline_declines = store.compact_inline_declines.saturating_add(1);
    }
    let _ = store;
    Ok(false)
}
#[cfg(test)]
thread_local! {static STOP_AT:std::cell::Cell<Option<(usize,bool)>>=const{std::cell::Cell::new(None)};}

#[cfg(all(test, feature = "execution-reference"))]
mod tests {
    use super::*;
    const BYTES: &[u8] =
        include_bytes!("../../../tests/fixtures/performance/p050/stack_calls.wasm");
    fn admitted() -> (Engine, Module) {
        let e = Engine::with_execution_policy(Config::default(), ExecutionPolicy::Compact).unwrap();
        let m = e.compile(BYTES).unwrap();
        (e, m)
    }
    #[test]
    fn p050_leaf_plan_corruption_and_budget_bounds_decline() {
        let (_, m) = admitted();
        for case in 0..7 {
            let mut p = Leaves::prepare(&m, MAX_WORK, MAX_BYTES).unwrap();
            assert!(p.verify(&m, MAX_WORK, MAX_BYTES));
            match case {
                0 => p.engine += 1,
                1 => p.source = Arc::from(vec![0; 8]),
                2 => p.entries[0] = None,
                3 => p.entries[0].as_mut().unwrap().multiplier += 1,
                4 => p.entries[0].as_mut().unwrap().body = usize::MAX,
                5 => p.bytes += 1,
                6 => p.entries.clear(),
                _ => unreachable!(),
            }
            assert!(!p.verify(&m, MAX_WORK, MAX_BYTES));
        }
        assert!(Leaves::prepare(&m, 0, MAX_BYTES).is_none());
        assert!(Leaves::prepare(&m, MAX_WORK, 1).is_none());
        let p = Leaves::prepare(&m, MAX_WORK, MAX_BYTES).unwrap();
        assert!(!p.verify(&m, MAX_WORK, p.bytes));
        for body in 1..6 {
            assert!(recognize(&m, body).is_none());
        }
        assert!(recognize(&m, 0).is_some());
    }
    #[test]
    fn p050_all_leaf_boundaries_cancel_and_fuel_restore_logical_activation() {
        let (e, m) = admitted();
        for fuel_stop in [false, true] {
            for pc in 0..6 {
                let mut s = Store::new(&e);
                let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
                let External::Func(target) = i.export(&s, "step").unwrap() else {
                    panic!()
                };
                let External::Func(caller) = i.export(&s, "run").unwrap() else {
                    panic!()
                };
                let mut caller_frame = s.frame(caller, vec![Value::I32(5)], 0).unwrap();
                caller_frame.pc = 3;
                let mut stack = vec![Value::I32(91), Value::I32(5)];
                let mut fuel = Some(6);
                let mut prompts = vec![];
                let mut stopped = None;
                STOP_AT.with(|slot| slot.set(Some((pc, fuel_stop))));
                let error = try_leaf(
                    &mut s,
                    target,
                    caller_frame.function.instance,
                    &mut stack,
                    &mut fuel,
                    1,
                    &mut prompts,
                    &mut stopped,
                )
                .unwrap_err();
                assert_eq!(
                    (error.kind, error.message.as_str()),
                    (
                        ErrorKind::ResourceLimit,
                        if fuel_stop {
                            "fuel exhausted"
                        } else {
                            "execution cancelled"
                        }
                    )
                );
                let frame = stopped.unwrap();
                assert_eq!(frame.function.body, 0);
                assert_eq!(frame.pc, pc);
                assert_eq!(frame.base, 1);
                assert_eq!(frame.locals, [Value::I32(5)]);
                assert!(frame.labels.is_empty());
                assert_eq!(caller_frame.pc, 3);
                assert_eq!(fuel, Some(if fuel_stop { 0 } else { 6 - pc as u64 }));
                let mut expected = vec![Value::I32(91)];
                match pc {
                    0 => {}
                    1 => expected.push(Value::I32(5)),
                    2 => expected.extend([Value::I32(5), Value::I32(3)]),
                    3 => expected.push(Value::I32(15)),
                    4 => expected.extend([Value::I32(15), Value::I32(1)]),
                    5 => expected.push(Value::I32(16)),
                    _ => unreachable!(),
                }
                assert_eq!(stack, expected);
                assert_eq!(s.compact_inline_leaf_calls(), 1);
                assert_eq!(s.compact_dispatches(), pc as u64);
            }
        }
    }
}
