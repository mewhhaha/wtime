//! Process-wide fault handler (`SIGSEGV`/`SIGBUS`/`SIGFPE`/`SIGILL`) and fault-based
//! `InterruptHandle` for zero-overhead Wasm execution preemption.
//!
//! When `InterruptHandle::interrupt()` is called on a `Store`, it `mprotect(..., PROT_NONE)`s
//! the Store's active GC heap and private linear memories.
//! - If Wasm code (`RIP` inside a registered `StoreCdylib` code range) faults on an interrupted
//!   range, the handler restores `PROT_READ | PROT_WRITE` and `siglongjmp`s back to the active
//!   export-entry boundary with `Trap::Interrupt`.
//! - If Host Rust code (`RIP` outside Wasm code) faults on an interrupted range first, the
//!   handler restores `PROT_READ | PROT_WRITE` and returns so the host instruction completes
//!   cleanly (the deadline thread re-applies `interrupt()` shortly after).

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};

use crate::abi::{StoreState, TRAP_INTERRUPT_CODE};

#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub(crate) struct SigJmpBuf {
    // Linux glibc `sigjmp_buf` is 200 bytes on x86_64 and 312 bytes on aarch64;
    // reserve 320 bytes aligned to 16.
    pub(crate) raw: [u64; 40],
}

impl Default for SigJmpBuf {
    fn default() -> Self {
        Self { raw: [0; 40] }
    }
}

unsafe extern "C" {
    #[link_name = "__sigsetjmp"]
    pub(crate) fn sigsetjmp(env: *mut SigJmpBuf, savemask: libc::c_int) -> libc::c_int;
    pub(crate) fn siglongjmp(env: *mut SigJmpBuf, val: libc::c_int) -> !;
}

/// Thread-local active Wasm invocation frame (linked list for arbitrary reentrancy).
#[repr(C)]
pub(crate) struct ActiveWasmFrame {
    pub(crate) prev: *mut ActiveWasmFrame,
    pub(crate) jmp_buf: SigJmpBuf,
    pub(crate) store_ptr: *mut StoreState,
    pub(crate) trap_code: u32,
}

thread_local! {
    static ACTIVE_FRAME: Cell<*mut ActiveWasmFrame> = const { Cell::new(std::ptr::null_mut()) };
}

/// Pushes an `ActiveWasmFrame` on the current thread before calling into Wasm machine code.
#[inline]
pub(crate) unsafe fn push_active_frame(frame: *mut ActiveWasmFrame) {
    ACTIVE_FRAME.with(|slot| {
        let prev = slot.get();
        // SAFETY: `frame` is stack-pinned for the duration of the Wasm call.
        unsafe { (*frame).prev = prev };
        slot.set(frame);
    });
}

/// Pops the current `ActiveWasmFrame` on normal return OR after `siglongjmp` trap recovery.
#[inline]
pub(crate) unsafe fn pop_active_frame(frame: *mut ActiveWasmFrame) {
    ACTIVE_FRAME.with(|slot| {
        // SAFETY: `frame` was pushed by `push_active_frame`.
        let prev = unsafe { (*frame).prev };
        slot.set(prev);
    });
}

// ---- Lock-free signal-safe global registries ----
// Signal handlers cannot acquire `Mutex`! We maintain compact fixed-capacity atomic-published
// snapshots (updated under a write mutex outside signal handlers; read lock-free inside handlers).

const MAX_GLOBAL_CODE_RANGES: usize = 4096;
const MAX_INTERRUPT_RANGES: usize = 4096;

struct AddrRange {
    start: AtomicUsize,
    end: AtomicUsize,
}

impl AddrRange {
    const fn empty() -> Self {
        Self {
            start: AtomicUsize::new(0),
            end: AtomicUsize::new(0),
        }
    }
}

struct GlobalCodeTable {
    count: AtomicUsize,
    entries: [AddrRange; MAX_GLOBAL_CODE_RANGES],
}

static CODE_TABLE: GlobalCodeTable = GlobalCodeTable {
    count: AtomicUsize::new(0),
    entries: [const { AddrRange::empty() }; MAX_GLOBAL_CODE_RANGES],
};
static CODE_TABLE_MU: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

pub(crate) fn register_global_code_range(start: usize, end: usize) {
    ensure_signal_handlers_installed();
    let mut list = CODE_TABLE_MU.lock().unwrap_or_else(|e| e.into_inner());
    list.push((start, end));
    let n = list.len().min(MAX_GLOBAL_CODE_RANGES);
    for (i, &(s, e)) in list.iter().take(n).enumerate() {
        CODE_TABLE.entries[i].start.store(s, Ordering::Relaxed);
        CODE_TABLE.entries[i].end.store(e, Ordering::Relaxed);
    }
    CODE_TABLE.count.store(n, Ordering::Release);
}

pub(crate) fn unregister_global_code_range(start: usize, end: usize) {
    let mut list = CODE_TABLE_MU.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(pos) = list.iter().position(|&(s, e)| s == start && e == end) {
        list.swap_remove(pos);
    }
    let n = list.len().min(MAX_GLOBAL_CODE_RANGES);
    for (i, &(s, e)) in list.iter().take(n).enumerate() {
        CODE_TABLE.entries[i].start.store(s, Ordering::Relaxed);
        CODE_TABLE.entries[i].end.store(e, Ordering::Relaxed);
    }
    CODE_TABLE.count.store(n, Ordering::Release);
}

#[inline]
fn is_in_wasm_code(rip: usize) -> bool {
    let n = CODE_TABLE.count.load(Ordering::Acquire);
    for i in 0..n {
        let s = CODE_TABLE.entries[i].start.load(Ordering::Relaxed);
        let e = CODE_TABLE.entries[i].end.load(Ordering::Relaxed);
        if rip >= s && rip < e {
            return true;
        }
    }
    false
}

/// Signal-safe entry representing one currently `PROT_NONE`-protected Store memory region.
struct InterruptRangeSlot {
    start: AtomicUsize,
    len: AtomicUsize,
    armed: AtomicBool,
}

impl InterruptRangeSlot {
    const fn empty() -> Self {
        Self {
            start: AtomicUsize::new(0),
            len: AtomicUsize::new(0),
            armed: AtomicBool::new(false),
        }
    }
}

static INTERRUPT_SLOTS: [InterruptRangeSlot; MAX_INTERRUPT_RANGES] =
    [const { InterruptRangeSlot::empty() }; MAX_INTERRUPT_RANGES];

/// Allocates a global signal-safe slot for a Store's interruptible range.
fn alloc_interrupt_slot(start: usize, len: usize) -> Option<usize> {
    for (i, slot) in INTERRUPT_SLOTS.iter().enumerate() {
        if slot.len.load(Ordering::Relaxed) == 0
            && slot
                .len
                .compare_exchange(0, len.max(1), Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
        {
            slot.start.store(start, Ordering::Release);
            slot.armed.store(false, Ordering::Release);
            return Some(i);
        }
    }
    None
}

fn free_interrupt_slot(idx: usize) {
    if let Some(slot) = INTERRUPT_SLOTS.get(idx) {
        slot.armed.store(false, Ordering::Release);
        slot.start.store(0, Ordering::Release);
        slot.len.store(0, Ordering::Release);
    }
}

/// Internal shared state behind `InterruptHandle`.
pub(crate) struct InterruptState {
    mu: Mutex<Vec<InterruptRegion>>,
    interrupted: AtomicBool,
}

struct InterruptRegion {
    base: usize,
    /// Committed RW prefix bytes that `interrupt()` revokes and `cancel()` restores.
    rw_len: usize,
    slot_idx: Option<usize>,
}

impl Drop for InterruptState {
    fn drop(&mut self) {
        let regions = self.mu.get_mut().unwrap_or_else(|e| e.into_inner());
        for r in regions.drain(..) {
            if let Some(idx) = r.slot_idx {
                free_interrupt_slot(idx);
            }
        }
    }
}

/// Handle allowing another thread to asynchronously preempt a `Store`'s Wasm execution with `Trap::Interrupt`.
#[derive(Clone)]
pub struct InterruptHandle {
    pub(crate) state: Arc<InterruptState>,
}

impl InterruptHandle {
    pub(crate) fn new() -> Self {
        ensure_signal_handlers_installed();
        Self {
            state: Arc::new(InterruptState {
                mu: Mutex::new(Vec::new()),
                interrupted: AtomicBool::new(false),
            }),
        }
    }

    /// Registers or updates an interruptible committed RW memory range `[base, base + rw_len)`.
    pub(crate) fn set_region(&self, base: usize, rw_len: usize) {
        let mut regions = self.state.mu.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(r) = regions.iter_mut().find(|r| r.base == base) {
            r.rw_len = rw_len;
            if let Some(idx) = r.slot_idx {
                INTERRUPT_SLOTS[idx]
                    .len
                    .store(rw_len.max(1), Ordering::Release);
            }
            return;
        }
        let slot_idx = alloc_interrupt_slot(base, rw_len);
        regions.push(InterruptRegion {
            base,
            rw_len,
            slot_idx,
        });
    }

    /// Unregisters a memory range before `munmap`.
    pub(crate) fn remove_region(&self, base: usize) {
        let mut regions = self.state.mu.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pos) = regions.iter().position(|r| r.base == base) {
            let r = regions.swap_remove(pos);
            if let Some(idx) = r.slot_idx {
                free_interrupt_slot(idx);
            }
        }
    }

    /// Preempts Wasm code executing in the target `Store` by removing access to its GC heap and
    /// private linear memories. Safe to call repeatedly from any thread.
    pub fn interrupt(&self) {
        self.state.interrupted.store(true, Ordering::Release);
        let regions = self.state.mu.lock().unwrap_or_else(|e| e.into_inner());
        for r in regions.iter() {
            if r.base == 0 || r.rw_len == 0 {
                continue;
            }
            if let Some(idx) = r.slot_idx {
                INTERRUPT_SLOTS[idx].start.store(r.base, Ordering::Relaxed);
                INTERRUPT_SLOTS[idx].len.store(r.rw_len, Ordering::Relaxed);
                INTERRUPT_SLOTS[idx].armed.store(true, Ordering::Release);
            }
            // SAFETY: `[r.base, r.base + r.rw_len)` is a live committed RW region owned by the Store.
            unsafe {
                let _ = libc::mprotect(r.base as *mut libc::c_void, r.rw_len, libc::PROT_NONE);
            }
        }
    }

    /// Withdraws any pending interrupt and restores RW access to all protected Store regions.
    pub fn cancel(&self) {
        self.state.interrupted.store(false, Ordering::Release);
        let regions = self.state.mu.lock().unwrap_or_else(|e| e.into_inner());
        for r in regions.iter() {
            if r.base == 0 || r.rw_len == 0 {
                continue;
            }
            if let Some(idx) = r.slot_idx {
                INTERRUPT_SLOTS[idx].armed.store(false, Ordering::Release);
            }
            // SAFETY: Restores RW permissions on the Store's own committed mapping.
            unsafe {
                let _ = libc::mprotect(
                    r.base as *mut libc::c_void,
                    r.rw_len,
                    libc::PROT_READ | libc::PROT_WRITE,
                );
            }
        }
    }

    /// Restores RW permissions on all regions after trapping or before host GC/memory operations.
    pub(crate) fn restore_all_rw(&self) {
        let regions = self.state.mu.lock().unwrap_or_else(|e| e.into_inner());
        for r in regions.iter() {
            if r.base == 0 || r.rw_len == 0 {
                continue;
            }
            if let Some(idx) = r.slot_idx {
                INTERRUPT_SLOTS[idx].armed.store(false, Ordering::Release);
            }
            unsafe {
                let _ = libc::mprotect(
                    r.base as *mut libc::c_void,
                    r.rw_len,
                    libc::PROT_READ | libc::PROT_WRITE,
                );
            }
        }
    }

    #[inline]
    pub fn is_interrupted(&self) -> bool {
        self.state.interrupted.load(Ordering::Acquire)
    }
}

// ---- Process-wide signal installation ----

static INSTALL_ONCE: Once = Once::new();
static mut PREV_SIGSEGV: libc::sigaction = unsafe { std::mem::zeroed() };
static mut PREV_SIGBUS: libc::sigaction = unsafe { std::mem::zeroed() };
static mut PREV_SIGFPE: libc::sigaction = unsafe { std::mem::zeroed() };
static mut PREV_SIGILL: libc::sigaction = unsafe { std::mem::zeroed() };
#[cfg(target_arch = "aarch64")]
static mut PREV_SIGTRAP: libc::sigaction = unsafe { std::mem::zeroed() };

pub(crate) fn ensure_signal_handlers_installed() {
    INSTALL_ONCE.call_once(|| unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = trap_signal_handler as *const () as usize;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_NODEFER;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGSEGV, &sa, std::ptr::addr_of_mut!(PREV_SIGSEGV));
        libc::sigaction(libc::SIGBUS, &sa, std::ptr::addr_of_mut!(PREV_SIGBUS));
        libc::sigaction(libc::SIGFPE, &sa, std::ptr::addr_of_mut!(PREV_SIGFPE));
        libc::sigaction(libc::SIGILL, &sa, std::ptr::addr_of_mut!(PREV_SIGILL));
        #[cfg(target_arch = "aarch64")]
        libc::sigaction(libc::SIGTRAP, &sa, std::ptr::addr_of_mut!(PREV_SIGTRAP));
    });
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[inline]
unsafe fn ucontext_pc(ucontext: *const libc::c_void) -> usize {
    let uc = unsafe { &*(ucontext as *const libc::ucontext_t) };
    uc.uc_mcontext.gregs[libc::REG_RIP as usize] as usize
}

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
#[inline]
unsafe fn ucontext_pc(ucontext: *const libc::c_void) -> usize {
    let uc = unsafe { &*(ucontext as *const libc::ucontext_t) };
    uc.uc_mcontext.pc as usize
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
unsafe extern "C" fn trap_signal_handler(
    sig: libc::c_int,
    info: *mut libc::siginfo_t,
    ucontext: *mut libc::c_void,
) {
    let pc = unsafe { ucontext_pc(ucontext) };
    let fault_addr = if !info.is_null() {
        unsafe { (*info).si_addr() as usize }
    } else {
        0
    };
    let in_wasm = is_in_wasm_code(pc);

    // 1. Check if this SIGSEGV hit an `InterruptHandle`-protected region.
    if sig == libc::SIGSEGV && fault_addr != 0 {
        for slot in &INTERRUPT_SLOTS {
            if !slot.armed.load(Ordering::Acquire) {
                continue;
            }
            let s = slot.start.load(Ordering::Acquire);
            let len = slot.len.load(Ordering::Acquire);
            if s != 0 && len != 0 && fault_addr >= s && fault_addr < s.saturating_add(len) {
                // Always restore RW permissions immediately so the page isn't stuck PROT_NONE.
                slot.armed.store(false, Ordering::Release);
                unsafe {
                    let _ = libc::mprotect(
                        s as *mut libc::c_void,
                        len,
                        libc::PROT_READ | libc::PROT_WRITE,
                    );
                }
                if !in_wasm {
                    // Host Rust code touched the page first: let the host instruction retry cleanly!
                    return;
                }
                // Wasm code hit the interrupt: unwind via active frame with `Trap::Interrupt`.
                let frame = ACTIVE_FRAME.with(|c| c.get());
                if !frame.is_null() {
                    unsafe {
                        (*frame).trap_code = TRAP_INTERRUPT_CODE;
                        siglongjmp(std::ptr::addr_of_mut!((*frame).jmp_buf), 1);
                    }
                }
            }
        }
    }

    // 2. If RIP is inside compiled Wasm code and we have an active frame, unwind with the exact trap.
    if in_wasm {
        let frame = ACTIVE_FRAME.with(|c| c.get());
        if !frame.is_null() {
            let code: u32 = match sig {
                libc::SIGSEGV | libc::SIGBUS => {
                    if fault_addr < 4096 {
                        wtime_aot::x64::TRAP_UNREACHABLE as u32
                    } else {
                        wtime_aot::x64::TRAP_OOB as u32
                    }
                }
                libc::SIGFPE => wtime_aot::x64::TRAP_DIV_ZERO as u32,
                libc::SIGILL | libc::SIGTRAP => {
                    let store_ptr = unsafe { (*frame).store_ptr };
                    let stored = if !store_ptr.is_null() {
                        unsafe { (*store_ptr).last_trap_code }
                    } else {
                        0
                    };
                    if stored != 0 {
                        stored
                    } else {
                        wtime_aot::x64::TRAP_UNREACHABLE as u32
                    }
                }
                _ => wtime_aot::x64::TRAP_UNREACHABLE as u32,
            };
            unsafe {
                (*frame).trap_code = code;
                siglongjmp(std::ptr::addr_of_mut!((*frame).jmp_buf), 1);
            }
        }
    }

    // 3. Not a Wasm trap or interrupt: chain to previous handler or default action.
    let prev = match sig {
        libc::SIGSEGV => unsafe { PREV_SIGSEGV },
        libc::SIGBUS => unsafe { PREV_SIGBUS },
        libc::SIGFPE => unsafe { PREV_SIGFPE },
        libc::SIGILL => unsafe { PREV_SIGILL },
        #[cfg(target_arch = "aarch64")]
        libc::SIGTRAP => unsafe { PREV_SIGTRAP },
        _ => unsafe { std::mem::zeroed() },
    };
    if (prev.sa_flags & libc::SA_SIGINFO) != 0 && prev.sa_sigaction != 0 {
        let f: unsafe extern "C" fn(libc::c_int, *mut libc::siginfo_t, *mut libc::c_void) =
            unsafe { std::mem::transmute(prev.sa_sigaction) };
        unsafe { f(sig, info, ucontext) };
        return;
    }
    if prev.sa_sigaction != libc::SIG_DFL && prev.sa_sigaction != libc::SIG_IGN {
        let f: unsafe extern "C" fn(libc::c_int) =
            unsafe { std::mem::transmute(prev.sa_sigaction) };
        unsafe { f(sig) };
        return;
    }
    unsafe {
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
}
