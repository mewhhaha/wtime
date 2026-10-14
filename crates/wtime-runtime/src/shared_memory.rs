//! Safe shared-memory backend. Lock order: SC coordinator, memory, waiter.
//! Ordinary access takes only the memory lock. Waiters block holding neither
//! coordinator nor memory; a durable waiter state resolves timeout/notify races.
use std::{
    collections::BTreeMap,
    sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock},
    time::{Duration, Instant},
};
use wtime_types::{Error, ErrorKind, Result, TrapCode};
static SC: OnceLock<Mutex<()>> = OnceLock::new();
fn lock<T>(m: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    m.lock()
        .map_err(|_| Error::new(ErrorKind::Host, "shared memory lock poisoned"))
}
fn coordinator() -> Result<MutexGuard<'static, ()>> {
    lock(SC.get_or_init(|| Mutex::new(())))
}
fn trap(message: &str) -> Error {
    Error::trap(
        if message.contains("unaligned") {
            TrapCode::UnalignedAtomic
        } else if message.contains("bounds") || message.contains("overflow") {
            TrapCode::MemoryOutOfBounds
        } else {
            TrapCode::Other
        },
        message,
    )
}
fn bounds(address: u64, width: usize, len: usize) -> Result<std::ops::Range<usize>> {
    let end = address
        .checked_add(width as u64)
        .ok_or_else(|| trap("memory address overflow"))?;
    if end > len as u64 {
        return Err(trap("out of bounds shared memory access"));
    }
    Ok(address as usize..end as usize)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WaitState {
    Pending,
    Notified,
    TimedOut,
    Cancelled,
}
struct Waiter {
    state: Mutex<WaitState>,
    wake: Condvar,
}
struct State {
    bytes: Vec<u8>,
    waiters: BTreeMap<u64, Vec<Arc<Waiter>>>,
}
#[derive(Clone)]
pub(crate) struct SharedMemory {
    state: Arc<Mutex<State>>,
    maximum: usize,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Rmw {
    Add,
    Sub,
    And,
    Or,
    Xor,
    Exchange,
}
impl SharedMemory {
    pub(crate) fn same_backing(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
    pub(crate) fn new(initial: usize, maximum: usize) -> Result<Self> {
        if initial > maximum {
            return Err(Error::new(
                ErrorKind::ResourceLimit,
                "shared memory initial exceeds maximum",
            ));
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(initial)
            .map_err(|_| Error::new(ErrorKind::ResourceLimit, "shared memory allocation failed"))?;
        bytes.resize(initial, 0);
        Ok(Self {
            state: Arc::new(Mutex::new(State {
                bytes,
                waiters: BTreeMap::new(),
            })),
            maximum,
        })
    }
    pub(crate) fn len(&self) -> Result<usize> {
        Ok(lock(&self.state)?.bytes.len())
    }
    pub(crate) fn read(&self, address: u64, width: usize) -> Result<Vec<u8>> {
        let state = lock(&self.state)?;
        Ok(state.bytes[bounds(address, width, state.bytes.len())?].to_vec())
    }
    pub(crate) fn write(&self, address: u64, bytes: &[u8]) -> Result<()> {
        let mut state = lock(&self.state)?;
        let range = bounds(address, bytes.len(), state.bytes.len())?;
        state.bytes[range].copy_from_slice(bytes);
        Ok(())
    }
    pub(crate) fn grow(&self, new_len: usize) -> Result<bool> {
        let mut state = lock(&self.state)?;
        if new_len < state.bytes.len() || new_len > self.maximum {
            return Ok(false);
        }
        let additional = new_len - state.bytes.len();
        if state.bytes.try_reserve_exact(additional).is_err() {
            return Ok(false);
        }
        state.bytes.resize(new_len, 0);
        Ok(true)
    }
    fn atomic_range(address: u64, width: usize, len: usize) -> Result<std::ops::Range<usize>> {
        if !matches!(width, 1 | 2 | 4 | 8) || !address.is_multiple_of(width as u64) {
            return Err(trap("unaligned atomic memory access"));
        }
        bounds(address, width, len)
    }
    pub(crate) fn load(&self, address: u64, width: usize) -> Result<u64> {
        let _sc = coordinator()?;
        let state = lock(&self.state)?;
        let range = Self::atomic_range(address, width, state.bytes.len())?;
        let mut bytes = [0; 8];
        bytes[..width].copy_from_slice(&state.bytes[range]);
        Ok(u64::from_le_bytes(bytes))
    }
    pub(crate) fn store(&self, address: u64, width: usize, value: u64) -> Result<()> {
        let _sc = coordinator()?;
        let mut state = lock(&self.state)?;
        let range = Self::atomic_range(address, width, state.bytes.len())?;
        state.bytes[range].copy_from_slice(&value.to_le_bytes()[..width]);
        Ok(())
    }
    pub(crate) fn rmw(&self, address: u64, width: usize, value: u64, op: Rmw) -> Result<u64> {
        let _sc = coordinator()?;
        let mut state = lock(&self.state)?;
        let range = Self::atomic_range(address, width, state.bytes.len())?;
        let mut bytes = [0; 8];
        bytes[..width].copy_from_slice(&state.bytes[range.clone()]);
        let old = u64::from_le_bytes(bytes);
        let next = match op {
            Rmw::Add => old.wrapping_add(value),
            Rmw::Sub => old.wrapping_sub(value),
            Rmw::And => old & value,
            Rmw::Or => old | value,
            Rmw::Xor => old ^ value,
            Rmw::Exchange => value,
        };
        state.bytes[range].copy_from_slice(&next.to_le_bytes()[..width]);
        Ok(old)
    }
    pub(crate) fn compare_exchange(
        &self,
        address: u64,
        width: usize,
        expected: u64,
        replacement: u64,
    ) -> Result<u64> {
        let _sc = coordinator()?;
        let mut state = lock(&self.state)?;
        let range = Self::atomic_range(address, width, state.bytes.len())?;
        let mut bytes = [0; 8];
        bytes[..width].copy_from_slice(&state.bytes[range.clone()]);
        let old = u64::from_le_bytes(bytes);
        let mask = if width == 8 {
            u64::MAX
        } else {
            (1u64 << (width * 8)) - 1
        };
        if old == expected & mask {
            state.bytes[range].copy_from_slice(&replacement.to_le_bytes()[..width]);
        }
        Ok(old)
    }
    pub(crate) fn fence() -> Result<()> {
        let _sc = coordinator()?;
        Ok(())
    }
    /// Return 0 when notified, 1 on mismatch, 2 on timeout. Any negative timeout is infinite.
    #[cfg(test)]
    pub(crate) fn wait(
        &self,
        address: u64,
        width: usize,
        expected: u64,
        timeout_ns: i64,
    ) -> Result<u32> {
        self.wait_with_cancel(address, width, expected, timeout_ns, None)
    }
    pub(crate) fn wait_with_cancel(
        &self,
        address: u64,
        width: usize,
        expected: u64,
        timeout_ns: i64,
        cancellation: Option<&crate::CancellationToken>,
    ) -> Result<u32> {
        if !matches!(width, 4 | 8) {
            return Err(trap("invalid atomic wait width"));
        }
        let waiter = Arc::new(Waiter {
            state: Mutex::new(WaitState::Pending),
            wake: Condvar::new(),
        });
        {
            let _sc = coordinator()?;
            let mut state = lock(&self.state)?;
            let range = Self::atomic_range(address, width, state.bytes.len())?;
            let mut bytes = [0; 8];
            bytes[..width].copy_from_slice(&state.bytes[range]);
            let mask = if width == 8 {
                u64::MAX
            } else {
                u32::MAX as u64
            };
            if u64::from_le_bytes(bytes) != expected & mask {
                return Ok(1);
            }
            if timeout_ns == 0 {
                return Ok(2);
            }
            state
                .waiters
                .entry(address)
                .or_default()
                .push(waiter.clone());
        }
        let mut outcome = lock(&waiter.state)?;
        let start = Instant::now();
        while *outcome == WaitState::Pending {
            if cancellation.is_some_and(crate::CancellationToken::is_cancelled) {
                *outcome = WaitState::Cancelled;
                break;
            }
            if timeout_ns < 0 && cancellation.is_none() {
                outcome = waiter
                    .wake
                    .wait(outcome)
                    .map_err(|_| Error::new(ErrorKind::Host, "waiter lock poisoned"))?;
            } else {
                let remaining = if timeout_ns < 0 {
                    Duration::from_millis(20)
                } else {
                    Duration::from_nanos(timeout_ns as u64).saturating_sub(start.elapsed())
                };
                if remaining.is_zero() {
                    *outcome = WaitState::TimedOut;
                    break;
                }
                let (next, _) = waiter
                    .wake
                    .wait_timeout(
                        outcome,
                        if cancellation.is_some() {
                            remaining.min(Duration::from_millis(20))
                        } else {
                            remaining
                        },
                    )
                    .map_err(|_| Error::new(ErrorKind::Host, "waiter lock poisoned"))?;
                outcome = next;
            }
        }
        let cancelled = *outcome == WaitState::Cancelled;
        let result = if *outcome == WaitState::Notified {
            0
        } else {
            2
        };
        drop(outcome);
        let mut state = lock(&self.state)?;
        if let Some(list) = state.waiters.get_mut(&address) {
            list.retain(|w| !Arc::ptr_eq(w, &waiter));
            if list.is_empty() {
                state.waiters.remove(&address);
            }
        }
        if cancelled {
            Err(Error::new(
                ErrorKind::ResourceLimit,
                "execution cancelled during atomic wait",
            ))
        } else {
            Ok(result)
        }
    }
    pub(crate) fn notify(&self, address: u64, count: u32) -> Result<u32> {
        let _sc = coordinator()?;
        let state = lock(&self.state)?;
        Self::atomic_range(address, 4, state.bytes.len())?;
        let mut notified = 0;
        if let Some(waiters) = state.waiters.get(&address) {
            for waiter in waiters {
                if notified == count {
                    break;
                }
                let mut outcome = lock(&waiter.state)?;
                if *outcome == WaitState::Pending {
                    *outcome = WaitState::Notified;
                    notified += 1;
                    waiter.wake.notify_one();
                }
            }
        }
        Ok(notified)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn registered(memory: &SharedMemory, address: u64, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if lock(&memory.state)
                .unwrap()
                .waiters
                .get(&address)
                .map_or(0, Vec::len)
                == count
            {
                return;
            }
            assert!(Instant::now() < deadline, "waiter registration timed out");
            std::thread::yield_now();
        }
    }
    #[test]
    fn t133_cancel_infinite_wait_removes_registration() {
        let memory = SharedMemory::new(8, 8).unwrap();
        let engine = crate::Engine::new(crate::Config::default()).unwrap();
        let store = crate::Store::new(&engine);
        let cancellation = store.cancellation_token();
        let peer = memory.clone();
        let token = cancellation.clone();
        let child = std::thread::spawn(move || peer.wait_with_cancel(0, 4, 0, -1, Some(&token)));
        registered(&memory, 0, 1);
        cancellation.cancel();
        assert_eq!(
            child.join().unwrap().unwrap_err().kind,
            ErrorKind::ResourceLimit
        );
        registered(&memory, 0, 0);
        assert_eq!(memory.notify(0, u32::MAX).unwrap(), 0);
        cancellation.reset();
        assert_eq!(
            memory
                .wait_with_cancel(0, 4, 0, 0, Some(&cancellation))
                .unwrap(),
            2
        );
    }
    #[test]
    fn t133_aliases_counted_notification_and_registration_races() {
        for _ in 0..20 {
            let memory = SharedMemory::new(8, 8).unwrap();
            let other = SharedMemory::new(8, 8).unwrap();
            let (send, receive) = std::sync::mpsc::channel();
            let mut children = Vec::new();
            for _ in 0..2 {
                let peer = memory.clone();
                let send = send.clone();
                children.push(std::thread::spawn(move || {
                    send.send(peer.wait(0, 4, 0, -9).unwrap()).unwrap()
                }));
            }
            registered(&memory, 0, 2);
            assert_eq!(other.notify(0, 2).unwrap(), 0);
            assert_eq!(memory.notify(4, 2).unwrap(), 0);
            assert_eq!(memory.clone().notify(0, 1).unwrap(), 1);
            assert_eq!(receive.recv_timeout(Duration::from_secs(2)).unwrap(), 0);
            assert!(receive.try_recv().is_err());
            assert_eq!(memory.notify(0, 1).unwrap(), 1);
            assert_eq!(receive.recv_timeout(Duration::from_secs(2)).unwrap(), 0);
            for child in children {
                child.join().unwrap();
            }
            registered(&memory, 0, 0);
        }
    }
    #[test]
    fn t130_two_memory_sc_store_buffering_and_fence_visibility() {
        for _ in 0..100 {
            let a = SharedMemory::new(8, 8).unwrap();
            let b = SharedMemory::new(8, 8).unwrap();
            let barrier = Arc::new(std::sync::Barrier::new(3));
            let (aa, bb, start) = (a.clone(), b.clone(), barrier.clone());
            let left = std::thread::spawn(move || {
                start.wait();
                aa.store(0, 8, 1).unwrap();
                bb.load(0, 8).unwrap()
            });
            let start = barrier.clone();
            let right = std::thread::spawn(move || {
                start.wait();
                b.store(0, 8, 1).unwrap();
                a.load(0, 8).unwrap()
            });
            barrier.wait();
            assert_ne!((left.join().unwrap(), right.join().unwrap()), (0, 0));
        }
        let data = SharedMemory::new(8, 8).unwrap();
        let flag = SharedMemory::new(8, 8).unwrap();
        let (d, f) = (data.clone(), flag.clone());
        let writer = std::thread::spawn(move || {
            d.store(0, 8, 42).unwrap();
            SharedMemory::fence().unwrap();
            f.store(0, 8, 1).unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        while flag.load(0, 8).unwrap() == 0 {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        SharedMemory::fence().unwrap();
        assert_eq!(data.load(0, 8).unwrap(), 42);
        writer.join().unwrap();
    }
    #[test]
    fn t131_atomic_subword_and_alignment() {
        let m = SharedMemory::new(16, 32).unwrap();
        m.store(0, 4, 0xffff_fffe).unwrap();
        assert_eq!(m.rmw(0, 4, 3, Rmw::Add).unwrap(), 0xffff_fffe);
        assert_eq!(m.load(0, 4).unwrap(), 1);
        assert_eq!(m.compare_exchange(0, 1, 0x101, 0x55).unwrap(), 1);
        assert_eq!(m.load(0, 1).unwrap(), 0x55);
        assert!(m.load(1, 4).is_err());
        assert!(m.load(16, 4).is_err());
    }
    #[test]
    fn t133_wait_mismatch_timeout_notify() {
        let m = SharedMemory::new(16, 16).unwrap();
        assert_eq!(m.wait(0, 4, 1, -2).unwrap(), 1);
        assert_eq!(m.wait(0, 4, 0, 0).unwrap(), 2);
        assert_eq!(m.wait(0, 4, 0, 1).unwrap(), 2);
        assert_eq!(m.notify(0, 2).unwrap(), 0);
        let peer = m.clone();
        let child = std::thread::spawn(move || peer.wait(0, 4, 0, -7).unwrap());
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if m.notify(0, 1).unwrap() == 1 {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(child.join().unwrap(), 0);
    }
    #[test]
    fn t130_concurrent_updates_and_distinct_memories() {
        let m = SharedMemory::new(8, 16).unwrap();
        let other = SharedMemory::new(8, 8).unwrap();
        let mut children = vec![];
        for _ in 0..4 {
            let m = m.clone();
            children.push(std::thread::spawn(move || {
                for _ in 0..1000 {
                    m.rmw(0, 8, 1, Rmw::Add).unwrap();
                }
            }));
        }
        for child in children {
            child.join().unwrap();
        }
        assert_eq!(m.load(0, 8).unwrap(), 4000);
        assert_eq!(other.load(0, 8).unwrap(), 0);
        assert!(m.grow(16).unwrap());
        assert!(!m.grow(17).unwrap());
    }
}
