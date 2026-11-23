//! Bounded observations, available only in instrumented Register builds.

/// Cumulative lifecycle event counts for a lazy Register preparation cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegisterCacheLifecycle {
    /// Leases admitted, including attempts exhausted before invoking a builder.
    pub preparation_starts: u64,
    /// Actual builder invocations, counted before entering the unlocked builder.
    pub builder_starts: u64,
    /// Preparation builds that ran to completion.
    pub completed: u64,
    /// Completed builds published into the cache as ready entries.
    pub published: u64,
    /// Completed builds that could not be published (for example due to budget limits).
    pub publication_failures: u64,
    /// Bodies explicitly declined by the Register builder.
    pub declined: u64,
    /// Attempts stopped because preparation work budgets were exhausted.
    pub exhausted: u64,
    /// Attempts rejected due to invalid or incompatible preparation state.
    pub invalid: u64,
    /// Preparation leases aborted before completion.
    pub aborted: u64,
    /// Cache entries evicted to stay within entry or byte limits.
    pub evictions: u64,
}

/// Current preparation state of a single cached Register function entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterCacheEntryState {
    /// Body has not yet reached the preparation threshold.
    Cold,
    /// Body is currently being compiled by a builder lease.
    Compiling,
    /// Compiled Register artifact is resident and ready for execution.
    Ready {
        /// Bytes retained by this ready artifact.
        retained_bytes: usize,
    },
    /// Preparation is backing off before the next retry attempt.
    Cooldown {
        /// Remaining entries before preparation may be retried.
        remaining: u64,
    },
    /// Body is permanently marked ineligible for Register preparation.
    Negative,
}

/// Snapshot of a single resident entry in a lazy Register preparation cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegisterCacheEntrySnapshot {
    /// Index in the module's defined bodies, excluding imported functions.
    pub body: usize,
    /// Observed function entry count for this body.
    pub entries: u64,
    /// Attempts for this resident entry; eviction may discard the entry.
    pub attempts: u8,
    /// Current preparation state of the entry.
    pub state: RegisterCacheEntryState,
}

/// Bounded diagnostic snapshot of a module's lazy Register preparation cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisterCacheSnapshot {
    /// Owning engine identifier.
    pub engine: u64,
    /// Configured function-entry threshold before triggering lazy preparation.
    pub threshold: u64,
    /// Maximum number of resident entries permitted in the cache.
    pub entry_limit: usize,
    /// Body index currently holding the active preparation lease, if any.
    pub active_body: Option<usize>,
    /// In insertion order, bounded by entry_limit (at most 64).
    pub entries: Vec<RegisterCacheEntrySnapshot>,
    /// Settled work only: a running builder settles its debit when it finishes.
    pub remaining_attempted_work: usize,
    /// Remaining work budget available for retained Register artifacts.
    pub remaining_retained_work: usize,
    /// Includes fixed bookkeeping and evicted artifacts still pinned by frames.
    pub retained_bytes: usize,
    /// Configured byte budget for retained cache artifacts and bookkeeping.
    pub byte_limit: usize,
    /// Cumulative lifecycle counters for this cache.
    pub lifecycle: RegisterCacheLifecycle,
}
