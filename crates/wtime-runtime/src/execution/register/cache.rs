//! Bounded, source-owned preparation, deliberately separate from guest execution.
use super::{
    execute::{Preparation, PreparedFunction, prepare_function},
    ir::Budget,
};
use crate::Module;
#[cfg(feature = "execution-reference")]
use crate::{
    RegisterCacheEntrySnapshot, RegisterCacheEntryState, RegisterCacheLifecycle,
    RegisterCacheSnapshot,
};
use std::{
    mem::size_of,
    sync::{Arc, Mutex},
};

const MAX_ENTRIES: usize = 64;
const MAX_WORK: usize = 100_000;
const MAX_BYTES: usize = 1024 * 1024;
const HOT_THRESHOLD: u64 = 16;
const COOLDOWN: u64 = 128;
const MAX_ATTEMPTS: u8 = 2;
const ARC_HEADER: usize = 2 * size_of::<usize>();

struct Owner {
    engine: u64,
    source: Arc<[u8]>,
}
impl Owner {
    fn new(module: &Module) -> Self {
        Self {
            engine: module.data.engine,
            source: module.data.inner.decoded().bytes.clone(),
        }
    }
    fn matches(&self, module: &Module) -> bool {
        self.engine == module.data.engine
            && Arc::ptr_eq(&self.source, &module.data.inner.decoded().bytes)
    }
}

pub(super) struct CachedFunction {
    pub(super) function: PreparedFunction,
    _retention: Retention,
}
struct Retention {
    ledger: Arc<Mutex<Ledger>>,
    bytes: usize,
}
impl Drop for Retention {
    fn drop(&mut self) {
        // The ledger contains only numbers, so a token cannot form a cache cycle.
        // Poison preserves the charge and prevents subsequent compilation.
        if let Ok(mut ledger) = self.ledger.lock() {
            if let Some(bytes) = ledger.bytes.checked_sub(self.bytes) {
                ledger.bytes = bytes;
            }
        }
    }
}
struct Ledger {
    attempted: usize,
    work: usize,
    bytes: usize,
    byte_limit: usize,
}
enum EntryState {
    Cold,
    Compiling,
    Ready(Arc<CachedFunction>),
    Cooldown { remaining: u64 },
    Negative,
}
struct Entry {
    body: usize,
    entries: u64,
    attempts: u8,
    state: EntryState,
}
struct State {
    // In insertion order. Fixed capacity makes both lookup and eviction bounded.
    entries: Vec<Entry>,
    active: Option<usize>,
    #[cfg(feature = "execution-reference")]
    lifecycle: RegisterCacheLifecycle,
}
pub(crate) struct Cache {
    owner: Owner,
    state: Mutex<State>,
    ledger: Arc<Mutex<Ledger>>,
    entry_limit: usize,
    threshold: u64,
}
impl Cache {
    #[cfg(feature = "execution-reference")]
    pub(crate) fn snapshot(&self, module: &Module) -> Option<RegisterCacheSnapshot> {
        if !self.owner.matches(module) {
            return None;
        }
        // Keep the existing state -> ledger lock order. Numeric copies never
        // clone/drop an artifact Arc while its retention ledger is locked.
        let state = self.state.lock().ok()?;
        let ledger = self.ledger.lock().ok()?;
        let mut entries = Vec::new();
        entries.try_reserve_exact(state.entries.len()).ok()?;
        for entry in &state.entries {
            let observed = match &entry.state {
                EntryState::Cold => RegisterCacheEntryState::Cold,
                EntryState::Compiling => RegisterCacheEntryState::Compiling,
                EntryState::Ready(function) => RegisterCacheEntryState::Ready {
                    retained_bytes: function._retention.bytes,
                },
                EntryState::Cooldown { remaining } => RegisterCacheEntryState::Cooldown {
                    remaining: *remaining,
                },
                EntryState::Negative => RegisterCacheEntryState::Negative,
            };
            entries.push(RegisterCacheEntrySnapshot {
                body: entry.body,
                entries: entry.entries,
                attempts: entry.attempts,
                state: observed,
            });
        }
        Some(RegisterCacheSnapshot {
            engine: self.owner.engine,
            threshold: self.threshold,
            entry_limit: self.entry_limit,
            active_body: state.active,
            entries,
            remaining_attempted_work: ledger.attempted,
            remaining_retained_work: ledger.work,
            retained_bytes: ledger.bytes,
            byte_limit: ledger.byte_limit,
            lifecycle: state.lifecycle,
        })
    }
    #[cfg(test)]
    pub(super) fn test_budget(&self) -> (usize, usize, usize) {
        let ledger = self.ledger.lock().unwrap();
        (ledger.attempted, ledger.work, ledger.bytes)
    }
    #[cfg(test)]
    pub(super) fn test_entry(&self, body: usize) -> Option<(u64, u8, bool)> {
        let state = self.state.lock().unwrap();
        let entry = state.entries.iter().find(|entry| entry.body == body)?;
        Some((
            entry.entries,
            entry.attempts,
            matches!(entry.state, EntryState::Ready(_)),
        ))
    }
    pub(crate) fn with_threshold(module: &Module, threshold: u64) -> Option<Arc<Self>> {
        Self::with_limits(module, MAX_ENTRIES, threshold, MAX_WORK, MAX_BYTES)
    }
    pub(super) fn new(module: &Module) -> Option<Arc<Self>> {
        Self::with_limits(module, MAX_ENTRIES, HOT_THRESHOLD, MAX_WORK, MAX_BYTES)
    }
    fn with_limits(
        module: &Module,
        entries: usize,
        threshold: u64,
        work: usize,
        bytes: usize,
    ) -> Option<Arc<Self>> {
        module.data.plans.as_ref()?;
        if entries == 0
            || entries > MAX_ENTRIES
            || threshold == 0
            || work > MAX_WORK
            || bytes > MAX_BYTES
        {
            return None;
        }
        let mut index = Vec::new();
        index.try_reserve_exact(entries).ok()?;
        let overhead = index
            .capacity()
            .checked_mul(size_of::<Entry>())?
            .checked_add(size_of::<Self>())?
            .checked_add(ARC_HEADER)?
            .checked_add(size_of::<Mutex<Ledger>>())?
            .checked_add(ARC_HEADER)?;
        let remaining = work.checked_sub(entries)?;
        if overhead > bytes {
            return None;
        }
        Some(Arc::new(Self {
            owner: Owner::new(module),
            state: Mutex::new(State {
                entries: index,
                active: None,
                #[cfg(feature = "execution-reference")]
                lifecycle: RegisterCacheLifecycle::default(),
            }),
            ledger: Arc::new(Mutex::new(Ledger {
                attempted: remaining,
                work: remaining,
                bytes: overhead,
                byte_limit: bytes,
            })),
            entry_limit: entries,
            threshold,
        }))
    }
    pub(super) fn enter(
        self: &Arc<Self>,
        module: &Module,
        body: usize,
    ) -> Option<Arc<CachedFunction>> {
        self.enter_with(module, body, prepare_function)
    }
    fn enter_with(
        self: &Arc<Self>,
        module: &Module,
        body: usize,
        builder: impl FnOnce(&Module, usize, &mut Budget, &mut usize) -> Preparation,
    ) -> Option<Arc<CachedFunction>> {
        if !self.owner.matches(module)
            || self.ledger.is_poisoned()
            || module.data.inner.decoded().code.get(body).is_none()
            || module.data.plans.as_ref()?.functions.get(body).is_none()
        {
            return None;
        }
        // No waiting behind preparation or contended bookkeeping.
        let mut state = self.state.try_lock().ok()?;
        let index = if let Some(index) = state.entries.iter().position(|entry| entry.body == body) {
            index
        } else {
            if state.active.is_some() {
                return None;
            }
            if state.entries.len() == self.entry_limit {
                // Failed attempts remain indexed: eviction must not erase their
                // cooldown or permanent negative result and restart their retry cap.
                let oldest = state.entries.iter().position(|entry| {
                    matches!(entry.state, EntryState::Cold | EntryState::Ready(_))
                })?;
                state.entries.remove(oldest);
                #[cfg(feature = "execution-reference")]
                {
                    state.lifecycle.evictions = state.lifecycle.evictions.saturating_add(1);
                }
            }
            state.entries.push(Entry {
                body,
                entries: 0,
                attempts: 0,
                state: EntryState::Cold,
            });
            state.entries.len() - 1
        };
        let entry = &mut state.entries[index];
        entry.entries = entry.entries.saturating_add(1);
        match &mut entry.state {
            EntryState::Ready(function) => {
                return function
                    .function
                    .matches(module, body)
                    .then(|| function.clone());
            }
            EntryState::Negative | EntryState::Compiling => return None,
            EntryState::Cold if entry.entries < self.threshold => return None,
            EntryState::Cooldown { remaining } => {
                *remaining = remaining.saturating_sub(1);
                if *remaining != 0 {
                    return None;
                }
            }
            _ => {}
        }
        if state.active.is_some() {
            return None;
        }
        let ledger = self.ledger.try_lock().ok()?;
        let available_bytes = ledger.byte_limit.checked_sub(ledger.bytes)?;
        let budget = Budget::new(ledger.work, available_bytes);
        let attempted = ledger.attempted;
        drop(ledger);
        state.entries[index].attempts = state.entries[index].attempts.saturating_add(1);
        state.entries[index].state = EntryState::Compiling;
        state.active = Some(body);
        #[cfg(feature = "execution-reference")]
        {
            state.lifecycle.preparation_starts =
                state.lifecycle.preparation_starts.saturating_add(1);
        }
        drop(state);
        let mut lease = Lease {
            cache: self.clone(),
            body,
            initial: budget.remaining(),
            budget,
            initial_attempted: attempted,
            attempted,
            finished: false,
        };
        // P132 charges PreparedFunction itself. Charge the wrapper/token and Arc
        // control words before allocating or invoking the bounded builder.
        let wrapper = size_of::<CachedFunction>()
            .checked_sub(size_of::<PreparedFunction>())?
            .checked_add(ARC_HEADER)?;
        let result = if lease.budget.reserve(0, wrapper).is_ok() {
            #[cfg(feature = "execution-reference")]
            {
                // Instrumented observations may briefly synchronize here; the
                // builder itself still runs after both cache locks are dropped.
                if let Ok(mut state) = self.state.lock() {
                    state.lifecycle.builder_starts =
                        state.lifecycle.builder_starts.saturating_add(1);
                }
            }
            builder(module, body, &mut lease.budget, &mut lease.attempted)
        } else {
            Preparation::Exhausted
        };
        lease.finish(module, result)
    }
}
#[cfg(test)]
impl CachedFunction {
    pub(super) fn retained_bytes(&self) -> usize {
        self._retention.bytes
    }
}
impl std::fmt::Debug for Cache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegisterEntryCache")
            .field("threshold", &self.threshold)
            .finish_non_exhaustive()
    }
}

struct Lease {
    cache: Arc<Cache>,
    body: usize,
    initial: (usize, usize),
    budget: Budget,
    initial_attempted: usize,
    attempted: usize,
    finished: bool,
}
impl Lease {
    fn settle_work(&self, ledger: &mut Ledger) -> bool {
        let Some(attempted) = self.initial_attempted.checked_sub(self.attempted) else {
            return false;
        };
        let Some(work) = self.initial.0.checked_sub(self.budget.remaining().0) else {
            return false;
        };
        let Some(next_attempted) = ledger.attempted.checked_sub(attempted) else {
            return false;
        };
        let Some(next_work) = ledger.work.checked_sub(work) else {
            return false;
        };
        ledger.attempted = next_attempted;
        ledger.work = next_work;
        true
    }
    fn finish(mut self, module: &Module, result: Preparation) -> Option<Arc<CachedFunction>> {
        let mut locked = self.cache.state.lock().ok()?;
        let state = &mut *locked;
        let index = state.entries.iter().position(|entry| {
            entry.body == self.body && matches!(entry.state, EntryState::Compiling)
        })?;
        if state.active != Some(self.body) {
            return None;
        }
        let mut ledger = self.cache.ledger.lock().ok()?;
        if !self.settle_work(&mut ledger) {
            return None;
        }
        // No remaining failure path may settle this attempt a second time.
        self.finished = true;
        state.active = None;
        #[cfg(feature = "execution-reference")]
        match &result {
            Preparation::Complete(_) => {
                state.lifecycle.completed = state.lifecycle.completed.saturating_add(1)
            }
            Preparation::Declined => {
                state.lifecycle.declined = state.lifecycle.declined.saturating_add(1)
            }
            Preparation::Exhausted => {
                state.lifecycle.exhausted = state.lifecycle.exhausted.saturating_add(1)
            }
            Preparation::Invalid => {
                state.lifecycle.invalid = state.lifecycle.invalid.saturating_add(1)
            }
        }
        #[cfg(feature = "execution-reference")]
        let complete = matches!(&result, Preparation::Complete(_));
        let entry = &mut state.entries[index];
        entry.state = match result {
            Preparation::Complete(function)
                if self.cache.owner.matches(module) && function.matches(module, self.body) =>
            {
                let retained = self
                    .initial
                    .1
                    .checked_sub(self.budget.remaining().1)
                    .and_then(|bytes| ledger.bytes.checked_add(bytes).map(|total| (bytes, total)))
                    .filter(|(_, total)| *total <= ledger.byte_limit);
                if let Some((bytes, total)) = retained {
                    ledger.bytes = total;
                    // Drop ledger before any Arc could be destroyed (its token
                    // takes this same mutex). The new artifact is immutable.
                    drop(ledger);
                    let cached = Arc::new(CachedFunction {
                        function,
                        _retention: Retention {
                            ledger: self.cache.ledger.clone(),
                            bytes,
                        },
                    });
                    #[cfg(feature = "execution-reference")]
                    {
                        state.lifecycle.published = state.lifecycle.published.saturating_add(1);
                    }
                    entry.state = EntryState::Ready(cached.clone());
                    return Some(cached);
                } else {
                    EntryState::Negative
                }
            }
            Preparation::Declined if entry.attempts < MAX_ATTEMPTS => EntryState::Cooldown {
                remaining: COOLDOWN,
            },
            _ => EntryState::Negative,
        };
        #[cfg(feature = "execution-reference")]
        if complete {
            state.lifecycle.publication_failures =
                state.lifecycle.publication_failures.saturating_add(1);
        }
        None
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        // A panic/aborted builder owns the only compile lease. Preserve consumed
        // work, clear the lease, and refuse future attempts for this body.
        if let Ok(mut state) = self.cache.state.lock() {
            if state.active == Some(self.body) {
                if let Ok(mut ledger) = self.cache.ledger.lock() {
                    if !self.settle_work(&mut ledger) {
                        ledger.attempted = 0;
                        ledger.work = 0;
                    }
                }
                if let Some(entry) = state
                    .entries
                    .iter_mut()
                    .find(|entry| entry.body == self.body)
                {
                    entry.state = EntryState::Negative;
                }
                state.active = None;
                #[cfg(feature = "execution-reference")]
                {
                    state.lifecycle.aborted = state.lifecycle.aborted.saturating_add(1);
                }
            }
        } else if let Ok(mut ledger) = self.cache.ledger.lock() {
            // Poisoned cache state is unusable; keep all possible work charged.
            ledger.attempted = 0;
            ledger.work = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Engine, FeatureSet};
    include!("../../../tests/performance/p133_private.rs");
    #[cfg(feature = "execution-reference")]
    mod diagnostics {
        use super::*;
        use crate as wtime_runtime;
        include!("../../../tests/performance/p138_trace.rs");
        include!("../../../tests/performance/p138_private.rs");
    }
}
