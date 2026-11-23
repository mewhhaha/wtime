//! Optional observations of live Register working-vector capacity.
use crate::execution::Frame;
use crate::{Error, ErrorKind, Result};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};

/// Snapshot of live and peak Register-tier working-vector capacity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegisterActivationSnapshot {
    /// Current working-vector capacity in bytes across active Register frames.
    pub current_bytes: usize,
    /// Peak working-vector capacity in bytes observed since creation or reset.
    pub peak_bytes: usize,
    /// Number of currently active Register frames.
    pub current_frames: usize,
    /// Peak concurrent Register frame count observed since creation or reset.
    pub peak_frames: usize,
    /// Number of currently open decoded invocation observation scopes.
    pub active_scopes: usize,
    /// Total number of capacity samples recorded.
    pub capacity_samples: u64,
    /// Whether any counter or byte accumulation overflowed during observation.
    pub overflowed: bool,
}

#[derive(Default)]
struct Shared {
    current_bytes: AtomicUsize,
    peak_bytes: AtomicUsize,
    current_frames: AtomicUsize,
    peak_frames: AtomicUsize,
    active_scopes: AtomicUsize,
    capacity_samples: AtomicU64,
    overflowed: AtomicBool,
}

#[derive(Clone, Default)]
pub(crate) struct Diagnostics(Arc<Shared>);

pub(crate) struct Scope {
    state: Diagnostics,
    base_bytes: usize,
    base_frames: usize,
    counted: bool,
}

impl Diagnostics {
    pub(crate) fn snapshot(&self) -> RegisterActivationSnapshot {
        RegisterActivationSnapshot {
            current_bytes: self.0.current_bytes.load(Relaxed),
            peak_bytes: self.0.peak_bytes.load(Relaxed),
            current_frames: self.0.current_frames.load(Relaxed),
            peak_frames: self.0.peak_frames.load(Relaxed),
            active_scopes: self.0.active_scopes.load(Relaxed),
            capacity_samples: self.0.capacity_samples.load(Relaxed),
            overflowed: self.0.overflowed.load(Relaxed),
        }
    }

    pub(crate) fn reset(&self) -> Result<()> {
        if self.0.active_scopes.load(Relaxed) != 0 {
            return Err(Error::new(
                ErrorKind::Host,
                "cannot reset active Register capacity observation",
            ));
        }
        self.0.current_bytes.store(0, Relaxed);
        self.0.peak_bytes.store(0, Relaxed);
        self.0.current_frames.store(0, Relaxed);
        self.0.peak_frames.store(0, Relaxed);
        self.0.capacity_samples.store(0, Relaxed);
        self.0.overflowed.store(false, Relaxed);
        Ok(())
    }

    pub(crate) fn scope(&self) -> Scope {
        let counted = self
            .0
            .active_scopes
            .try_update(Relaxed, Relaxed, |n| n.checked_add(1))
            .is_ok();
        if !counted {
            self.0.overflowed.store(true, Relaxed);
        }
        Scope {
            state: self.clone(),
            base_bytes: self.0.current_bytes.load(Relaxed),
            base_frames: self.0.current_frames.load(Relaxed),
            counted,
        }
    }
}

impl Scope {
    pub(crate) fn observe(&self, frames: &[Frame]) {
        let totals = frames.iter().try_fold(
            (self.base_bytes, self.base_frames),
            |(bytes, count), frame| {
                let Some(activation) = &frame.register else {
                    return Some((bytes, count));
                };
                let working = activation.slot_capacity_bytes()?.checked_add(
                    frame
                        .labels
                        .capacity()
                        .checked_mul(std::mem::size_of::<crate::execution::Label>())?,
                )?;
                Some((bytes.checked_add(working)?, count.checked_add(1)?))
            },
        );
        let Some((bytes, count)) = totals else {
            self.state.0.overflowed.store(true, Relaxed);
            return;
        };
        self.state.0.current_bytes.store(bytes, Relaxed);
        self.state.0.current_frames.store(count, Relaxed);
        self.state.0.peak_bytes.fetch_max(bytes, Relaxed);
        self.state.0.peak_frames.fetch_max(count, Relaxed);
        if self
            .state
            .0
            .capacity_samples
            .try_update(Relaxed, Relaxed, |n| n.checked_add(1))
            .is_err()
        {
            self.state.0.overflowed.store(true, Relaxed);
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        self.state.0.current_bytes.store(self.base_bytes, Relaxed);
        self.state.0.current_frames.store(self.base_frames, Relaxed);
        if self.counted {
            self.state.0.active_scopes.fetch_sub(1, Relaxed);
        }
    }
}
