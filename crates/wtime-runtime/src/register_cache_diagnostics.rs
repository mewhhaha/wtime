//! Bounded observations, available only in instrumented Register builds.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegisterCacheLifecycle {
    /// Leases admitted, including attempts exhausted before invoking a builder.
    pub preparation_starts: u64,
    /// Actual builder invocations, counted before entering the unlocked builder.
    pub builder_starts: u64,
    pub completed: u64,
    pub published: u64,
    pub publication_failures: u64,
    pub declined: u64,
    pub exhausted: u64,
    pub invalid: u64,
    pub aborted: u64,
    pub evictions: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterCacheEntryState {
    Cold,
    Compiling,
    Ready { retained_bytes: usize },
    Cooldown { remaining: u64 },
    Negative,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegisterCacheEntrySnapshot {
    /// Index in the module's defined bodies, excluding imported functions.
    pub body: usize,
    pub entries: u64,
    /// Attempts for this resident entry; eviction may discard the entry.
    pub attempts: u8,
    pub state: RegisterCacheEntryState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisterCacheSnapshot {
    pub engine: u64,
    pub threshold: u64,
    pub entry_limit: usize,
    pub active_body: Option<usize>,
    /// In insertion order, bounded by entry_limit (at most 64).
    pub entries: Vec<RegisterCacheEntrySnapshot>,
    /// Settled work only: a running builder settles its debit when it finishes.
    pub remaining_attempted_work: usize,
    pub remaining_retained_work: usize,
    /// Includes fixed bookkeeping and evicted artifacts still pinned by frames.
    pub retained_bytes: usize,
    pub byte_limit: usize,
    pub lifecycle: RegisterCacheLifecycle,
}
