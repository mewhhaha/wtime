//! Private linear memory (`Memory`) and multi-threaded `SharedMemory` with `atomic.wait`/`notify`.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicI32, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::abi::StoreMemDesc;
use crate::config::Engine;
use crate::error::{Error, Result};
use crate::signal::InterruptHandle;
use crate::types::MemoryType;

pub const WASM_PAGE_BYTES: u64 = 65_536;
/// Virtual address reservation for private Wasm32 memories (4 GiB + 32 MiB guard).
const PRIVATE_RESERVE_BYTES: usize = (1usize << 32) + (32usize << 20);
/// Default memory cap when no explicit maximum is declared on a 32-bit memory (65,536 pages = 4 GiB).
const MAX_MEM32_PAGES: u64 = 65_536;

/// Owned private linear memory mapping (lives inside `StoreInner::memories`).
pub(crate) struct PrivateMemoryData {
    pub(crate) ty: MemoryType,
    /// Stable heap-boxed `StoreMemDesc` referenced directly by `Ctx + OFF_MEM_RESERVE_BYTES` and `OFF_STORE_MEM_DESCS`.
    pub(crate) desc: Box<StoreMemDesc>,
    /// Total `mmap`ped virtual bytes (`>= max_pages * 65536`).
    pub(crate) reserve_bytes: usize,
    /// Currently committed RW bytes (`== desc.len_bytes`).
    pub(crate) committed_bytes: usize,
    /// Interrupt handle for unregistering on drop and updating committed range on grow.
    pub(crate) interrupt: InterruptHandle,
}

impl PrivateMemoryData {
    pub(crate) fn new(
        ty: MemoryType,
        max_bytes_limit: usize,
        interrupt: InterruptHandle,
    ) -> Result<Self> {
        if ty.is_shared() {
            return Err(Error::msg(
                "PrivateMemoryData cannot hold a shared memory; use SharedMemory",
            ));
        }
        let init_bytes = ty
            .minimum()
            .checked_mul(WASM_PAGE_BYTES)
            .ok_or_else(|| Error::msg("memory initial byte size overflow"))?;
        if init_bytes as usize > max_bytes_limit {
            return Err(Error::msg(format!(
                "memory initial size ({init_bytes} B) exceeds Store limit ({max_bytes_limit} B)"
            )));
        }
        let eff_max_pages = ty.maximum().unwrap_or(MAX_MEM32_PAGES).min(MAX_MEM32_PAGES);
        let needed_reserve = (eff_max_pages as usize)
            .saturating_mul(WASM_PAGE_BYTES as usize)
            .saturating_add(32 << 20);
        let reserve_bytes = needed_reserve.max(PRIVATE_RESERVE_BYTES);

        // SAFETY: Reserve `PROT_NONE` anonymous private address space (zero physical RAM committed).
        let base = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                reserve_bytes,
                libc::PROT_NONE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if base == libc::MAP_FAILED {
            return Err(Error::msg(
                "failed to reserve virtual address space for linear memory",
            ));
        }
        if init_bytes > 0 {
            let rc = unsafe {
                libc::mprotect(
                    base,
                    init_bytes as usize,
                    libc::PROT_READ | libc::PROT_WRITE,
                )
            };
            if rc != 0 {
                unsafe {
                    let _ = libc::munmap(base, reserve_bytes);
                }
                return Err(Error::msg("failed to commit initial linear memory pages"));
            }
            interrupt.set_region(base as usize, init_bytes as usize);
        }
        let flags = if ty.is_64() { 2 } else { 0 };
        let desc = Box::new(StoreMemDesc {
            base: base.cast::<u8>(),
            len_bytes: AtomicU64::new(init_bytes),
            max_pages: eff_max_pages,
            flags,
        });
        Ok(Self {
            ty,
            desc,
            reserve_bytes,
            committed_bytes: init_bytes as usize,
            interrupt,
        })
    }

    pub(crate) fn grow(&mut self, delta_pages: u64, max_bytes_limit: usize) -> Option<u64> {
        let cur_bytes = self.desc.len_bytes.load(Ordering::Acquire);
        let cur_pages = cur_bytes / WASM_PAGE_BYTES;
        if delta_pages == 0 {
            return Some(cur_pages);
        }
        let new_pages = cur_pages.checked_add(delta_pages)?;
        if new_pages > self.desc.max_pages {
            return None;
        }
        let new_bytes = new_pages.checked_mul(WASM_PAGE_BYTES)?;
        if new_bytes as usize > max_bytes_limit || (new_bytes as usize) > self.reserve_bytes {
            return None;
        }
        let added = (new_bytes as usize) - self.committed_bytes;
        if added > 0 {
            // SAFETY: Within the reserved `mmap` range `[base, base + reserve_bytes)`.
            let rc = unsafe {
                libc::mprotect(
                    self.desc.base.add(self.committed_bytes).cast(),
                    added,
                    libc::PROT_READ | libc::PROT_WRITE,
                )
            };
            if rc != 0 {
                return None;
            }
            self.committed_bytes = new_bytes as usize;
            self.interrupt
                .set_region(self.desc.base as usize, self.committed_bytes);
        }
        self.desc.len_bytes.store(new_bytes, Ordering::Release);
        Some(cur_pages)
    }

    #[inline]
    pub(crate) fn data(&self) -> &[u8] {
        let len = self.desc.len_bytes.load(Ordering::Acquire) as usize;
        if len == 0 {
            &[]
        } else {
            // SAFETY: `[base, base + len)` is committed RW memory owned by this Store.
            unsafe { std::slice::from_raw_parts(self.desc.base, len) }
        }
    }

    #[inline]
    pub(crate) fn data_mut(&mut self) -> &mut [u8] {
        let len = self.desc.len_bytes.load(Ordering::Acquire) as usize;
        if len == 0 {
            &mut []
        } else {
            // SAFETY: Exclusive `&mut self` guarantees no concurrent Wasm execution.
            unsafe { std::slice::from_raw_parts_mut(self.desc.base, len) }
        }
    }
}

impl Drop for PrivateMemoryData {
    fn drop(&mut self) {
        if !self.desc.base.is_null() {
            self.interrupt.remove_region(self.desc.base as usize);
            // SAFETY: Matches `mmap` in `PrivateMemoryData::new`.
            unsafe {
                let _ = libc::munmap(self.desc.base.cast(), self.reserve_bytes);
            }
        }
    }
}

// =========================================================================
// SharedMemory (Wasm Threads: shared across Stores & OS threads)
// =========================================================================

struct Waiter {
    notified: Mutex<bool>,
    cv: Condvar,
}

pub(crate) struct SharedMemoryInner {
    pub(crate) ty: MemoryType,
    /// Stable `StoreMemDesc` shared by all threads/instances pointing to this shared memory.
    pub(crate) desc: Box<StoreMemDesc>,
    reserve_bytes: usize,
    grow_mu: Mutex<()>,
    wait_queues: Mutex<HashMap<usize, VecDeque<Arc<Waiter>>>>,
}

// SAFETY: Shared memory backing is a fixed-base `mmap` with atomic `len_bytes`, mutex-serialized
// `mprotect` growth, and mutex+condvar wait queues.
unsafe impl Send for SharedMemoryInner {}
unsafe impl Sync for SharedMemoryInner {}

impl Drop for SharedMemoryInner {
    fn drop(&mut self) {
        if !self.desc.base.is_null() {
            unsafe {
                let _ = libc::munmap(self.desc.base.cast(), self.reserve_bytes);
            }
        }
    }
}

impl SharedMemoryInner {
    pub(crate) fn new(ty: MemoryType) -> Result<Arc<Self>> {
        if !ty.is_shared() {
            return Err(Error::msg("SharedMemory::new requires a shared MemoryType"));
        }
        let max_pages = ty
            .maximum()
            .ok_or_else(|| Error::msg("shared memory must declare a maximum size"))?;
        if ty.minimum() > max_pages {
            return Err(Error::msg("shared memory minimum exceeds maximum"));
        }
        let init_bytes = ty
            .minimum()
            .checked_mul(WASM_PAGE_BYTES)
            .ok_or_else(|| Error::msg("shared memory initial byte size overflow"))?;
        let max_bytes = max_pages
            .checked_mul(WASM_PAGE_BYTES)
            .ok_or_else(|| Error::msg("shared memory max byte size overflow"))?;
        let reserve_bytes = (max_bytes as usize)
            .saturating_add(32 << 20)
            .max(PRIVATE_RESERVE_BYTES);

        let base = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                reserve_bytes,
                libc::PROT_NONE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if base == libc::MAP_FAILED {
            return Err(Error::msg(
                "failed to reserve virtual address space for shared memory",
            ));
        }
        if init_bytes > 0 {
            let rc = unsafe {
                libc::mprotect(
                    base,
                    init_bytes as usize,
                    libc::PROT_READ | libc::PROT_WRITE,
                )
            };
            if rc != 0 {
                unsafe {
                    let _ = libc::munmap(base, reserve_bytes);
                }
                return Err(Error::msg("failed to commit initial shared memory pages"));
            }
        }
        let flags = 1 | if ty.is_64() { 2 } else { 0 };
        let desc = Box::new(StoreMemDesc {
            base: base.cast::<u8>(),
            len_bytes: AtomicU64::new(init_bytes),
            max_pages,
            flags,
        });
        Ok(Arc::new(Self {
            ty,
            desc,
            reserve_bytes,
            grow_mu: Mutex::new(()),
            wait_queues: Mutex::new(HashMap::new()),
        }))
    }

    pub(crate) fn size_pages(&self) -> u64 {
        self.desc.len_bytes.load(Ordering::Acquire) / WASM_PAGE_BYTES
    }

    pub(crate) fn data_size(&self) -> usize {
        self.desc.len_bytes.load(Ordering::Acquire) as usize
    }

    pub(crate) fn grow(&self, delta_pages: u64) -> Result<u64> {
        let _guard = self.grow_mu.lock().unwrap_or_else(|e| e.into_inner());
        let cur_bytes = self.desc.len_bytes.load(Ordering::Acquire);
        let cur_pages = cur_bytes / WASM_PAGE_BYTES;
        if delta_pages == 0 {
            return Ok(cur_pages);
        }
        let new_pages = cur_pages
            .checked_add(delta_pages)
            .ok_or_else(|| Error::msg("shared memory grow overflow"))?;
        if new_pages > self.desc.max_pages {
            return Err(Error::msg("shared memory grow exceeds maximum"));
        }
        let new_bytes = new_pages
            .checked_mul(WASM_PAGE_BYTES)
            .ok_or_else(|| Error::msg("shared memory grow byte overflow"))?;
        let added = (new_bytes - cur_bytes) as usize;
        if added > 0 {
            let rc = unsafe {
                libc::mprotect(
                    self.desc.base.add(cur_bytes as usize).cast(),
                    added,
                    libc::PROT_READ | libc::PROT_WRITE,
                )
            };
            if rc != 0 {
                return Err(Error::msg("shared memory mprotect commit failed"));
            }
        }
        self.desc.len_bytes.store(new_bytes, Ordering::Release);
        Ok(cur_pages)
    }

    /// Wasm `memory.atomic.wait32` / `memory.atomic.wait64`.
    /// Returns: `0` = ok (notified), `1` = not-equal, `2` = timed-out.
    pub(crate) fn atomic_wait(
        &self,
        host_ptr: *const u8,
        expected: u64,
        timeout_ns: i64,
        is_64: bool,
    ) -> i32 {
        let addr = host_ptr as usize;
        let waiter = Arc::new(Waiter {
            notified: Mutex::new(false),
            cv: Condvar::new(),
        });
        {
            let mut queues = self.wait_queues.lock().unwrap_or_else(|e| e.into_inner());
            let matches = if is_64 {
                // SAFETY: Alignment & bounds checked by compiled Wasm before calling helper.
                let actual = unsafe { (*(host_ptr as *const AtomicI64)).load(Ordering::SeqCst) };
                actual as u64 == expected
            } else {
                let actual = unsafe { (*(host_ptr as *const AtomicI32)).load(Ordering::SeqCst) };
                (actual as u32) == (expected as u32)
            };
            if !matches {
                return 1; // not-equal
            }
            if timeout_ns == 0 {
                return 2; // timed-out immediately without blocking
            }
            queues
                .entry(addr)
                .or_default()
                .push_back(Arc::clone(&waiter));
        }

        let mut notified = waiter.notified.lock().unwrap_or_else(|e| e.into_inner());
        let woken = if timeout_ns < 0 {
            while !*notified {
                notified = waiter.cv.wait(notified).unwrap_or_else(|e| e.into_inner());
            }
            true
        } else {
            let deadline = Instant::now() + Duration::from_nanos(timeout_ns as u64);
            loop {
                if *notified {
                    break true;
                }
                let Some(rem) = deadline.checked_duration_since(Instant::now()) else {
                    break false;
                };
                if rem.is_zero() {
                    break false;
                }
                let (g, _) = waiter
                    .cv
                    .wait_timeout(notified, rem)
                    .unwrap_or_else(|e| e.into_inner());
                notified = g;
            }
        };
        drop(notified);

        if !woken {
            // Remove ourselves from queue if we timed out before any notify popped us.
            let mut queues = self.wait_queues.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(q) = queues.get_mut(&addr) {
                if let Some(pos) = q.iter().position(|w| Arc::ptr_eq(w, &waiter)) {
                    q.remove(pos);
                }
                if q.is_empty() {
                    queues.remove(&addr);
                }
            }
            // Check whether a concurrent `notify` popped and marked us between wait & lock.
            if *waiter.notified.lock().unwrap_or_else(|e| e.into_inner()) {
                return 0;
            }
            return 2;
        }
        0
    }

    /// Wasm `memory.atomic.notify`. Returns the number of woken waiters.
    pub(crate) fn atomic_notify(&self, host_ptr: *const u8, count: u32) -> u32 {
        if count == 0 {
            return 0;
        }
        let addr = host_ptr as usize;
        let to_wake: Vec<Arc<Waiter>> = {
            let mut queues = self.wait_queues.lock().unwrap_or_else(|e| e.into_inner());
            let Some(q) = queues.get_mut(&addr) else {
                return 0;
            };
            let take = (count as usize).min(q.len());
            let mut out = Vec::with_capacity(take);
            for _ in 0..take {
                if let Some(w) = q.pop_front() {
                    out.push(w);
                }
            }
            if q.is_empty() {
                queues.remove(&addr);
            }
            out
        };
        let woken = to_wake.len() as u32;
        for w in to_wake {
            let mut flag = w.notified.lock().unwrap_or_else(|e| e.into_inner());
            *flag = true;
            w.cv.notify_one();
        }
        woken
    }
}

/// Thread-safe handle to a Wasm shared linear memory.
#[derive(Clone)]
pub struct SharedMemory {
    pub(crate) inner: Arc<SharedMemoryInner>,
}

impl std::fmt::Debug for SharedMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedMemory")
            .field("ty", &self.inner.ty)
            .finish()
    }
}

impl SharedMemory {
    pub fn new(_engine: &Engine, ty: MemoryType) -> Result<Self> {
        Ok(Self {
            inner: SharedMemoryInner::new(ty)?,
        })
    }

    pub fn ty(&self) -> MemoryType {
        self.inner.ty.clone()
    }

    pub fn size(&self) -> u64 {
        self.inner.size_pages()
    }

    pub fn data_size(&self) -> usize {
        self.inner.data_size()
    }

    pub fn grow(&self, delta: u64) -> Result<u64> {
        self.inner.grow(delta)
    }

    pub fn data(&self) -> &[std::cell::UnsafeCell<u8>] {
        let len = self.inner.data_size();
        if len == 0 {
            &[]
        } else {
            // SAFETY: `[base, base + len)` is committed shared memory; `UnsafeCell<u8>` matches Wasmtime's API.
            unsafe {
                std::slice::from_raw_parts(
                    self.inner.desc.base.cast::<std::cell::UnsafeCell<u8>>(),
                    len,
                )
            }
        }
    }

    pub fn atomic_notify(&self, addr: u64, count: u32) -> Result<u32> {
        let len = self.inner.desc.len_bytes.load(Ordering::Acquire);
        if (addr & 3) != 0 || addr.saturating_add(4) > len {
            return Err(Error::msg(
                "shared memory atomic_notify out of bounds or unaligned",
            ));
        }
        let ptr = unsafe { self.inner.desc.base.add(addr as usize) };
        Ok(self.inner.atomic_notify(ptr, count))
    }

    pub fn atomic_wait32(&self, addr: u64, expected: u32) -> Result<i32> {
        let len = self.inner.desc.len_bytes.load(Ordering::Acquire);
        if (addr & 3) != 0 || addr.saturating_add(4) > len {
            return Err(Error::msg(
                "shared memory atomic_wait32 out of bounds or unaligned",
            ));
        }
        let ptr = unsafe { self.inner.desc.base.add(addr as usize) };
        Ok(self.inner.atomic_wait(ptr, u64::from(expected), -1, false))
    }

    pub fn atomic_wait64(&self, addr: u64, expected: u64) -> Result<i32> {
        let len = self.inner.desc.len_bytes.load(Ordering::Acquire);
        if (addr & 7) != 0 || addr.saturating_add(8) > len {
            return Err(Error::msg(
                "shared memory atomic_wait64 out of bounds or unaligned",
            ));
        }
        let ptr = unsafe { self.inner.desc.base.add(addr as usize) };
        Ok(self.inner.atomic_wait(ptr, expected, -1, true))
    }

    pub fn data_ptr(&self) -> *mut u8 {
        self.inner.desc.base
    }
}
