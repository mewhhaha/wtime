//! Cheney semi-space copying GC, multi-module return-address stack walker,
//! `WeakAnyRef`, `GcEphemeron` + registered WeakMap tables, `ExternRef` finalization sweep,
//! and rooted GC reference API (`RootScope`, `Rooted`, `OwnedRooted`, `AnyRef`, `EqRef`,
//! `StructRef`, `ArrayRef`, `ExternRef`, `ExnRef`).

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;
use std::sync::Arc;

use wtime_aot::x64::module::{
    GC_EXT_I31_TAG_HI, GC_HDR_KIND_ARRAY, GC_HDR_KIND_INTERNAL_EXTERN, GC_HDR_KIND_STRUCT,
    GcStorageKind,
};

use crate::abi::{
    GcStackChainNode, SafepointEntry, StoreState, decode_host_extern_slot, encode_host_extern_raw,
};
use crate::config::RegisteredCodeRange;
use crate::error::{Error, GcHeapOutOfMemory, Result, Trap};
use crate::signal::InterruptHandle;
use crate::store::{AsContext, AsContextMut, StoreInner};
use crate::types::{ArrayType, CanonComposite, CanonTypeInfo, ExnType, StructType, ValType};

/// Each semi-space reserves 8 GiB of virtual address space inside a single contiguous 16 GiB arena:
/// - Space A: `[arena_base + 64, arena_base + 8 GiB)`
/// - Space B: `[arena_base + 8 GiB + 64, arena_base + 16 GiB)`
///
/// Skipping the first 64 bytes of each space guarantees every live 8-byte-aligned GC object pointer
/// `p` has an offset `off = p - arena_base` in `[64, 16 GiB)` with `off & 7 == 0`. Encoding its
/// 32-bit raw handle as `raw = (off >> 2) as u32` (decoded as `arena_base + ((raw as u64) << 2)`)
/// guarantees `raw != 0`, `raw & 1 == 0` (distinct from null `0` and `i31` `bit 0 == 1`), and
/// moving between Space A (`off < 8 GiB`) and Space B (`off >= 8 GiB`) flips bit 31 so every
/// surviving object changes its raw handle across collection!
pub const SEMI_SPACE_RESERVE_BYTES: usize = 8usize << 30; // 8 GiB
const ARENA_TOTAL_BYTES: usize = SEMI_SPACE_RESERVE_BYTES * 2; // 16 GiB
const SPACE_START_PAD: usize = 64;

/// A 31-bit signed integer stored unboxed in Wasm GC `i31ref` slots (`(val << 1) | 1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct I31(u32);

impl I31 {
    pub const fn wrapping_i32(v: i32) -> Self {
        Self((v as u32) & 0x7fff_ffff)
    }

    pub const fn wrapping_u32(v: u32) -> Self {
        Self(v & 0x7fff_ffff)
    }

    pub const fn new_i32(v: i32) -> Option<Self> {
        if v >= -(1 << 30) && v < (1 << 30) {
            Some(Self((v as u32) & 0x7fff_ffff))
        } else {
            None
        }
    }

    pub const fn new_u32(v: u32) -> Option<Self> {
        if v < (1 << 31) { Some(Self(v)) } else { None }
    }

    pub const fn get_i32(self) -> i32 {
        ((self.0 << 1) as i32) >> 1
    }

    pub const fn get_u32(self) -> u32 {
        self.0
    }

    #[inline]
    pub(crate) const fn to_abi_word(self) -> u64 {
        (((self.0 as u64) << 1) | 1) & 0xffff_ffff
    }

    #[inline]
    pub(crate) const fn from_abi_word(w: u64) -> Option<Self> {
        if (w & 1) != 0 {
            Some(Self(((w as u32) >> 1) & 0x7fff_ffff))
        } else {
            None
        }
    }
}

/// Raw ABI value slot matching `wasmtime::ValRaw` (used by `func_new_unchecked` fast paths).
#[repr(C)]
#[derive(Clone, Copy)]
pub union ValRaw {
    i32: i32,
    i64: i64,
    f32: u32,
    f64: u64,
    v128: u128,
    anyref: u32,
    externref: u32,
    funcref: usize,
    raw_u64: u64,
}

impl ValRaw {
    pub const fn i32(v: i32) -> Self {
        Self { i32: v }
    }
    pub const fn i64(v: i64) -> Self {
        Self { i64: v }
    }
    pub const fn f32(bits: u32) -> Self {
        Self { f32: bits }
    }
    pub const fn f64(bits: u64) -> Self {
        Self { f64: bits }
    }
    pub const fn v128(v: u128) -> Self {
        Self { v128: v }
    }
    pub const fn anyref(raw: u32) -> Self {
        Self { anyref: raw }
    }
    pub const fn externref(raw: u32) -> Self {
        Self { externref: raw }
    }
    pub const fn null() -> Self {
        Self { raw_u64: 0 }
    }
    pub fn get_i32(&self) -> i32 {
        unsafe { self.i32 }
    }
    pub fn get_i64(&self) -> i64 {
        unsafe { self.i64 }
    }
    pub fn get_f32(&self) -> u32 {
        unsafe { self.f32 }
    }
    pub fn get_f64(&self) -> u64 {
        unsafe { self.f64 }
    }
    pub fn get_v128(&self) -> u128 {
        unsafe { self.v128 }
    }
    pub fn get_anyref(&self) -> u32 {
        unsafe { self.anyref }
    }
    pub fn get_externref(&self) -> u32 {
        unsafe { self.externref }
    }
    pub fn get_u64(&self) -> u64 {
        unsafe { self.raw_u64 }
    }
}

/// One slot in the Store's generational host root table (traced & updated by `store_gc_collect`).
pub(crate) struct HostRootSlot {
    pub(crate) value: u64,
    pub(crate) generation: u32,
    pub(crate) is_extern: bool,
    pub(crate) is_exn: bool,
    pub(crate) alive: bool,
}

/// Generational `WeakAnyRef` entry inside `GcHeap`.
pub(crate) struct WeakRefEntry {
    pub(crate) owner: u64,
    pub(crate) target: u64,
    pub(crate) generation: u32,
    pub(crate) live: bool,
}

/// Generational `GcEphemeron` entry inside `GcHeap`.
pub(crate) struct EphemeronEntry {
    pub(crate) owner: u64,
    pub(crate) key: u64,
    pub(crate) value: u64, // 0 = None, i31, or GC heap ptr
    pub(crate) generation: u32,
    pub(crate) live: bool,
    pub(crate) activated: bool,
}

/// Host-data arena slot backing `ExternRef::new(store, data)`.
pub(crate) struct ExternArenaSlot {
    pub(crate) data: Option<Box<dyn Any + Send + Sync>>,
}

/// Host-allocated Wasm exception object (`StoreExnObject`: `[u32 global_tag_id, u32 pad, payload...]`).
pub(crate) struct StoreExnBox {
    pub(crate) ptr: *mut u8,
    pub(crate) layout: std::alloc::Layout,
    pub(crate) global_tag_id: u32,
    /// `(byte_offset, is_extern)` for traced GC/Extern payload fields inside this exception.
    pub(crate) traced_fields: Vec<(i32, bool)>,
}

// SAFETY: Owned exclusively by `StoreInner` (accessed under `&mut Store`).
unsafe impl Send for StoreExnBox {}

impl Drop for StoreExnBox {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                std::alloc::dealloc(self.ptr, self.layout);
            }
        }
    }
}

/// Per-`Store` dual-semi-space Cheney GC heap + root tables + Weak/Ephemeron/Extern arenas.
pub(crate) struct GcHeap {
    pub(crate) arena_base: usize,
    /// Which half is currently active (`false` = Space A @ +0, `true` = Space B @ +8 GiB).
    pub(crate) active_is_b: bool,
    /// Initial committed bytes of a semi-space (floor when shrinking back after bursts).
    pub(crate) init_cap: usize,
    /// Committed bytes of the active semi-space (aligned to 4 KiB).
    pub(crate) from_cap: usize,
    /// Hard cap per semi-space (bounded by `StoreLimits` & `SEMI_SPACE_RESERVE_BYTES - 4096`).
    pub(crate) heap_cap: usize,
    /// Currently committed RW prefix bytes (4 KiB-aligned) of Space A and Space B.
    /// Steady-state bump-alloc GCs keep both semi-spaces warm up to `init_cap` to eliminate
    /// per-GC `mprotect` / `MADV_DONTNEED` kernel `mmap_lock` and cross-core TLB shootdowns.
    pub(crate) space_a_commit: usize,
    pub(crate) space_b_commit: usize,
    /// High-water byte offset (4 KiB-aligned, relative to space start) that may hold non-zero
    /// bytes from prior cycles. Pages strictly at or beyond this watermark are guaranteed OS-zeroed.
    pub(crate) space_a_dirty: usize,
    pub(crate) space_b_dirty: usize,
    /// Active interrupt handle to keep GC heap region registered for `mprotect`.
    pub(crate) interrupt: InterruptHandle,
    /// Generational host root table (`Rooted` & `OwnedRooted`).
    pub(crate) host_roots: Vec<HostRootSlot>,
    pub(crate) host_roots_free: Vec<u32>,
    /// Stack of `RootScope` frames storing slot indices to free on scope drop.
    pub(crate) scope_stack: Vec<Vec<(u32, u32)>>,
    /// Generational `WeakAnyRef` table.
    pub(crate) weak_refs: Vec<WeakRefEntry>,
    pub(crate) weak_refs_free: Vec<u32>,
    /// Incremented whenever GC retires at least one `WeakAnyRef`.
    pub(crate) weak_retire_count: u64,
    /// Generational `GcEphemeron` table.
    pub(crate) ephemerons: Vec<EphemeronEntry>,
    pub(crate) ephemerons_free: Vec<u32>,
    /// Set of active-space addresses of WeakMap reference arrays registered via `register_table_raw`.
    pub(crate) weak_tables: HashSet<u64>,
    /// Arena of host-owned `ExternRef` payloads (swept after ephemeron fixed-point on every GC).
    pub(crate) extern_arena: Vec<ExternArenaSlot>,
    pub(crate) extern_free: Vec<u32>,
    /// All Store-allocated `StoreExnObject`s (kept alive while Store lives, traced during GC).
    pub(crate) exn_objects: Vec<StoreExnBox>,
    pub(crate) exn_by_ptr: HashMap<usize, usize>,
}

impl Drop for GcHeap {
    fn drop(&mut self) {
        if self.arena_base != 0 {
            self.interrupt.remove_region(self.space_a_start());
            self.interrupt.remove_region(self.space_b_start());
            unsafe {
                let _ = libc::munmap(self.arena_base as *mut libc::c_void, ARENA_TOTAL_BYTES);
            }
        }
    }
}

impl GcHeap {
    pub(crate) fn new(
        init_bytes: usize,
        heap_cap: usize,
        interrupt: InterruptHandle,
    ) -> Result<Self> {
        let max_allowed = SEMI_SPACE_RESERVE_BYTES - 4096;
        let heap_cap = heap_cap.clamp(64 * 1024, max_allowed);
        let init_cap = ((init_bytes.min(heap_cap) + 4095) & !4095).max(64 * 1024);

        let base = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                ARENA_TOTAL_BYTES,
                libc::PROT_NONE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE,
                -1,
                0,
            )
        };
        if base == libc::MAP_FAILED {
            return Err(Error::msg("failed to reserve 16 GiB GC virtual arena"));
        }
        let arena_base = base as usize;
        // Commit Space A `[arena_base, arena_base + SPACE_START_PAD + init_cap)`.
        let commit_len = (SPACE_START_PAD + init_cap + 4095) & !4095;
        let rc = unsafe {
            libc::mprotect(
                arena_base as *mut libc::c_void,
                commit_len,
                libc::PROT_READ | libc::PROT_WRITE,
            )
        };
        if rc != 0 {
            unsafe {
                let _ = libc::munmap(base, ARENA_TOTAL_BYTES);
            }
            return Err(Error::msg("failed to commit initial GC semi-space"));
        }
        interrupt.set_region(arena_base, commit_len);

        Ok(Self {
            arena_base,
            active_is_b: false,
            init_cap,
            from_cap: init_cap,
            heap_cap,
            space_a_commit: commit_len,
            space_b_commit: 0,
            space_a_dirty: 0,
            space_b_dirty: 0,
            interrupt,
            host_roots: Vec::new(),
            host_roots_free: Vec::new(),
            scope_stack: Vec::new(),
            weak_refs: Vec::new(),
            weak_refs_free: Vec::new(),
            weak_retire_count: 0,
            ephemerons: Vec::new(),
            ephemerons_free: Vec::new(),
            weak_tables: HashSet::new(),
            extern_arena: Vec::new(),
            extern_free: Vec::new(),
            exn_objects: Vec::new(),
            exn_by_ptr: HashMap::new(),
        })
    }

    #[inline]
    pub(crate) fn space_a_start(&self) -> usize {
        self.arena_base
    }

    #[inline]
    pub(crate) fn space_b_start(&self) -> usize {
        self.arena_base + SEMI_SPACE_RESERVE_BYTES
    }

    #[inline]
    pub(crate) fn active_space_start(&self) -> usize {
        if self.active_is_b {
            self.space_b_start()
        } else {
            self.space_a_start()
        }
    }

    #[inline]
    pub(crate) fn active_dirty(&self) -> usize {
        if self.active_is_b {
            self.space_b_dirty
        } else {
            self.space_a_dirty
        }
    }

    #[inline]
    fn space_commit_mut(&mut self, is_b: bool) -> &mut usize {
        if is_b {
            &mut self.space_b_commit
        } else {
            &mut self.space_a_commit
        }
    }

    #[inline]
    fn space_dirty_mut(&mut self, is_b: bool) -> &mut usize {
        if is_b {
            &mut self.space_b_dirty
        } else {
            &mut self.space_a_dirty
        }
    }

    /// Releases physical pages of the currently inactive GC semi-space back to the OS
    /// (`MADV_DONTNEED` + `PROT_NONE`), leaving the active semi-space and all live objects untouched.
    /// Safe and O(1) no-op when the inactive semi-space is already released.
    pub(crate) fn reclaim_inactive_space(&mut self) {
        let inactive_is_b = !self.active_is_b;
        let commit = *self.space_commit_mut(inactive_is_b);
        if commit == 0 {
            return;
        }
        let page_start = if inactive_is_b {
            self.space_b_start()
        } else {
            self.space_a_start()
        };
        self.interrupt.remove_region(page_start);
        unsafe {
            let _ = libc::madvise(page_start as *mut libc::c_void, commit, libc::MADV_DONTNEED);
            let _ = libc::mprotect(page_start as *mut libc::c_void, commit, libc::PROT_NONE);
        }
        *self.space_commit_mut(inactive_is_b) = 0;
        *self.space_dirty_mut(inactive_is_b) = 0;
    }

    #[inline]
    pub(crate) fn active_from_base(&self) -> u64 {
        let base = if self.active_is_b {
            self.space_b_start()
        } else {
            self.space_a_start()
        };
        (base + SPACE_START_PAD) as u64
    }

    #[inline]
    pub(crate) fn inactive_to_base(&self) -> u64 {
        let base = if self.active_is_b {
            self.space_a_start()
        } else {
            self.space_b_start()
        };
        (base + SPACE_START_PAD) as u64
    }

    #[inline]
    pub(crate) fn is_in_from_space(&self, ptr: u64, from_base: u64, from_cap: u64) -> bool {
        ptr >= from_base && ptr < from_base.wrapping_add(from_cap) && (ptr & 7) == 0
    }

    /// Converts an active-space 64-bit GC pointer / i31 / null into a compact 32-bit raw handle.
    #[inline]
    pub(crate) fn abi_word_to_raw_u32(&self, word: u64) -> u32 {
        if word == 0 {
            0
        } else if (word & 1) != 0 {
            word as u32
        } else {
            let off = word.wrapping_sub(self.arena_base as u64);
            debug_assert!(
                off >= SPACE_START_PAD as u64 && off < (ARENA_TOTAL_BYTES as u64) && (off & 7) == 0
            );
            let raw = (off >> 2) as u32;
            debug_assert!(raw != 0 && (raw & 1) == 0);
            raw
        }
    }

    /// Converts a compact 32-bit raw GC handle back to a 64-bit ABI word (validating active space).
    #[inline]
    pub(crate) fn raw_u32_to_abi_word(&self, raw: u32) -> u64 {
        if raw == 0 {
            0
        } else if (raw & 1) != 0 {
            u64::from(raw)
        } else {
            (self.arena_base as u64).wrapping_add(u64::from(raw) << 2)
        }
    }

    #[inline]
    pub(crate) fn is_valid_active_heap_ptr(&self, ptr: u64, state: &StoreState) -> bool {
        ptr != 0 && (ptr & 7) == 0 && ptr >= state.gc_from_base && ptr < state.gc_alloc_ptr
    }

    /// Allocates a host root slot; if a `RootScope` is active, attaches it to the innermost scope.
    pub(crate) fn alloc_host_root(
        &mut self,
        value: u64,
        is_extern: bool,
        is_exn: bool,
        owned: bool,
    ) -> (u32, u32) {
        let (idx, generation) = if let Some(idx) = self.host_roots_free.pop() {
            let slot = &mut self.host_roots[idx as usize];
            slot.generation = slot.generation.wrapping_add(1).max(1);
            slot.value = value;
            slot.is_extern = is_extern;
            slot.is_exn = is_exn;
            slot.alive = true;
            (idx, slot.generation)
        } else {
            let idx = self.host_roots.len() as u32;
            self.host_roots.push(HostRootSlot {
                value,
                generation: 1,
                is_extern,
                is_exn,
                alive: true,
            });
            (idx, 1)
        };
        if !owned && let Some(scope) = self.scope_stack.last_mut() {
            scope.push((idx, generation));
        }
        (idx, generation)
    }

    pub(crate) fn free_host_root(&mut self, idx: u32, generation: u32) {
        if let Some(slot) = self.host_roots.get_mut(idx as usize)
            && slot.alive
            && slot.generation == generation
        {
            slot.alive = false;
            slot.value = 0;
            self.host_roots_free.push(idx);
        }
    }

    #[inline]
    pub(crate) fn read_host_root(&self, idx: u32, generation: u32) -> Result<u64> {
        let Some(slot) = self.host_roots.get(idx as usize) else {
            return Err(Error::msg("invalid rooted GC handle"));
        };
        if !slot.alive || slot.generation != generation {
            return Err(Error::msg("rooted GC handle has expired"));
        }
        Ok(slot.value)
    }

    pub(crate) fn alloc_extern_data(&mut self, data: Box<dyn Any + Send + Sync>) -> u64 {
        let idx = if let Some(idx) = self.extern_free.pop() {
            self.extern_arena[idx as usize].data = Some(data);
            idx
        } else {
            let idx = self.extern_arena.len() as u32;
            self.extern_arena.push(ExternArenaSlot { data: Some(data) });
            idx
        };
        encode_host_extern_raw(idx)
    }

    pub(crate) fn get_extern_data(&self, raw: u64) -> Option<&(dyn Any + Send + Sync)> {
        let slot = decode_host_extern_slot(raw)?;
        self.extern_arena
            .get(slot as usize)
            .and_then(|s| s.data.as_deref())
    }
}

// =========================================================================
// Cheney Semi-Space Copying Garbage Collector (Multi-Module + Weak + Ephemeron + Extern Sweep)
// =========================================================================

/// Performs a full stop-the-world Cheney copying collection for `store`, ensuring at least
/// `req_bytes` of free bump-allocation space (growing the semi-space up to `heap_cap` if needed).
///
/// When `reclaim_old` is `true` (explicit host `Store::gc()`), the entire evacuated old semi-space
/// is immediately returned to Linux (`MADV_DONTNEED` + `PROT_NONE`) so idle isolates hold minimal
/// RSS. When `reclaim_old` is `false` (automatic bump-alloc GC during active execution), the old
/// semi-space stays committed RW up to `init_cap` (reclaiming only any burst tail above `init_cap`)
/// so steady-state Cheney flips execute with zero `mprotect` / `madvise` kernel VM locks or
/// cross-core TLB shootdowns.
///
/// Returns `Ok(())` on success, or `Err(GcHeapOutOfMemory)` if `live_bytes + req_bytes > heap_cap`.
pub(crate) unsafe fn collect_store_gc(
    store: &mut StoreInner,
    req_bytes: u64,
    gc_call_ret_rsp: u64,
    reclaim_old: bool,
) -> std::result::Result<(), GcHeapOutOfMemory<()>> {
    // Free any `OwnedRooted<T>` handles dropped since the last GC before tracing roots.
    store.drain_dropped_owned_roots();

    // Ensure GC arena pages are writable before copying (O(1) atomic fast-path when un-interrupted).
    store.interrupt.restore_all_rw();

    let old_is_b = store.gc.active_is_b;
    let to_is_b = !old_is_b;
    let from_base = store.state.gc_from_base;
    let from_cap = store.state.gc_from_cap;
    let old_alloc_ptr = store.state.gc_alloc_ptr;
    let heap_cap = store.gc.heap_cap as u64;

    // Size `to_space` to `from_cap` during copying: surviving live bytes are strictly bounded
    // by `from_cap`, so we never prematurely double the semi-space based on pre-collection
    // garbage! Growth (or shrinking after a burst) is decided in step 7 using exact
    // post-collection `live_bytes + req_bytes`.
    let copy_cap = ((from_cap as usize).max(64 * 1024) + 4095) & !4095;
    let target_cap = copy_cap.min(heap_cap as usize);

    let to_space_page_start = if to_is_b {
        store.gc.space_b_start()
    } else {
        store.gc.space_a_start()
    };
    let to_base = (to_space_page_start + SPACE_START_PAD) as u64;
    let to_commit_needed = (SPACE_START_PAD + target_cap + 4095) & !4095;
    let prev_to_commit = *store.gc.space_commit_mut(to_is_b);
    if to_commit_needed > prev_to_commit {
        let delta_ptr = (to_space_page_start + prev_to_commit) as *mut libc::c_void;
        let delta_len = to_commit_needed - prev_to_commit;
        let rc =
            unsafe { libc::mprotect(delta_ptr, delta_len, libc::PROT_READ | libc::PROT_WRITE) };
        if rc != 0 {
            return Err(GcHeapOutOfMemory::new((), req_bytes));
        }
        *store.gc.space_commit_mut(to_is_b) = to_commit_needed;
        store
            .interrupt
            .set_region(to_space_page_start, to_commit_needed);
    }

    // Lock-free per-Store snapshots so concurrent Stores across CPU cores never stall each other
    // during stop-the-world Cheney collections.
    store.sync_type_descs();
    let types_snap = Arc::clone(&store.types_snapshot);
    let code_ranges = store.sync_code_ranges();

    let mut extern_live = vec![false; store.gc.extern_arena.len()];
    let old_weak_tables = std::mem::take(&mut store.gc.weak_tables);
    let mut live_weak_tables: Vec<u64> = Vec::new();
    let mut live_weak_table_set: HashSet<u64> = HashSet::new();

    struct Collector<'a> {
        infos: &'a [CanonTypeInfo],
        from_base: u64,
        from_cap: u64,
        to_alloc: u64,
        to_scan: u64,
        to_limit: u64,
        extern_live: &'a mut [bool],
        old_weak_tables: &'a HashSet<u64>,
        live_weak_tables: &'a mut Vec<u64>,
        live_weak_table_set: &'a mut HashSet<u64>,
        oom: bool,
    }

    impl<'a> Collector<'a> {
        #[inline]
        fn in_from(&self, ptr: u64) -> bool {
            ptr >= self.from_base
                && ptr < self.from_base.wrapping_add(self.from_cap)
                && (ptr & 7) == 0
        }

        #[inline]
        fn mark_extern_word(&mut self, word: u64) {
            if let Some(slot) = decode_host_extern_slot(word)
                && let Some(flag) = self.extern_live.get_mut(slot as usize)
            {
                *flag = true;
            }
        }

        #[inline]
        unsafe fn forwarded_addr(&self, old_ptr: u64) -> Option<u64> {
            if !self.in_from(old_ptr) {
                return None;
            }
            let hdr = unsafe { *(old_ptr as *const u64) };
            if (hdr & 1) != 0 { Some(hdr & !1) } else { None }
        }

        /// Evacuates a single `from_space` object into `to_space` (idempotent via forwarding bit 0).
        unsafe fn evacuate(&mut self, old_ptr: u64) -> u64 {
            if !self.in_from(old_ptr) {
                self.mark_extern_word(old_ptr);
                return old_ptr;
            }
            let hdr = unsafe { *(old_ptr as *const u64) };
            if (hdr & 1) != 0 {
                return hdr & !1;
            }
            let kind = (hdr as u32) & 6;
            let obj_bytes: u64 = match kind {
                GC_HDR_KIND_STRUCT => {
                    let cid = (hdr as u32) >> 3;
                    match &self.infos[cid as usize].composite {
                        CanonComposite::Struct { total_bytes, .. } => u64::from(*total_bytes),
                        _ => unreachable!("GC struct header has non-struct canon_id {cid}"),
                    }
                }
                GC_HDR_KIND_ARRAY => {
                    let cid = (hdr as u32) >> 3;
                    let len = unsafe { *((old_ptr + 4) as *const u32) } as u64;
                    let elem_sz = match &self.infos[cid as usize].composite {
                        CanonComposite::Array { elem, .. } => u64::from(elem.byte_size()),
                        _ => unreachable!("GC array header has non-array canon_id {cid}"),
                    };
                    ((8u64 + len * elem_sz) + 7) & !7
                }
                GC_HDR_KIND_INTERNAL_EXTERN => 16,
                other => unreachable!("corrupt GC object header kind {other} at {old_ptr:#x}"),
            };

            if self.to_alloc + obj_bytes > self.to_limit {
                self.oom = true;
                return old_ptr;
            }
            let new_ptr = self.to_alloc;
            self.to_alloc += obj_bytes;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    old_ptr as *const u8,
                    new_ptr as *mut u8,
                    obj_bytes as usize,
                );
                // Install forwarding pointer in old header (`bit 0 = 1`).
                *(old_ptr as *mut u64) = new_ptr | 1;
            }
            if kind == GC_HDR_KIND_ARRAY
                && !self.old_weak_tables.is_empty()
                && self.old_weak_tables.contains(&old_ptr)
            {
                self.live_weak_tables.push(new_ptr);
                self.live_weak_table_set.insert(new_ptr);
            }
            new_ptr
        }

        #[inline]
        unsafe fn trace_slot(&mut self, slot: *mut u64) {
            let v = unsafe { *slot };
            if v == 0 || (v & 1) != 0 {
                return;
            }
            if self.in_from(v) {
                let nv = unsafe { self.evacuate(v) };
                unsafe { *slot = nv };
            } else {
                self.mark_extern_word(v);
            }
        }

        /// Drains the Cheney BFS scan queue `[to_scan .. to_alloc)` to quiescence.
        unsafe fn scan_to_completion(&mut self) {
            while self.to_scan < self.to_alloc && !self.oom {
                let obj = self.to_scan;
                let hdr = unsafe { *(obj as *const u64) };
                let kind = (hdr as u32) & 6;
                match kind {
                    GC_HDR_KIND_STRUCT => {
                        let cid = (hdr as u32) >> 3;
                        let (total_bytes, gc_offs, ext_offs) =
                            match &self.infos[cid as usize].composite {
                                CanonComposite::Struct {
                                    total_bytes,
                                    gc_ref_offs,
                                    ext_ref_offs,
                                    ..
                                } => (*total_bytes, gc_ref_offs, ext_ref_offs),
                                _ => unreachable!(),
                            };
                        for &off in gc_offs {
                            unsafe { self.trace_slot((obj + u64::from(off)) as *mut u64) };
                        }
                        for &off in ext_offs {
                            unsafe { self.trace_slot((obj + u64::from(off)) as *mut u64) };
                        }
                        self.to_scan += u64::from(total_bytes);
                    }
                    GC_HDR_KIND_ARRAY => {
                        let cid = (hdr as u32) >> 3;
                        let len = unsafe { *((obj + 4) as *const u32) } as u64;
                        let elem = match &self.infos[cid as usize].composite {
                            CanonComposite::Array { elem, .. } => *elem,
                            _ => unreachable!(),
                        };
                        let obj_bytes = ((8u64 + len * u64::from(elem.byte_size())) + 7) & !7;
                        // Registered WeakMap reference arrays are NOT strongly traced during BFS;
                        // their `(key, value)` pairs are resolved in the ephemeron fixed-point loop!
                        if matches!(elem, GcStorageKind::GcRef | GcStorageKind::ExternRef)
                            && (self.live_weak_table_set.is_empty()
                                || !self.live_weak_table_set.contains(&obj))
                        {
                            for i in 0..len {
                                unsafe { self.trace_slot((obj + 8 + i * 8) as *mut u64) };
                            }
                        }
                        self.to_scan += obj_bytes;
                    }
                    GC_HDR_KIND_INTERNAL_EXTERN => {
                        let payload = unsafe { *((obj + 8) as *const u64) };
                        self.mark_extern_word(payload);
                        self.to_scan += 16;
                    }
                    other => unreachable!("corrupt scanned GC object kind {other}"),
                }
            }
        }

        /// Walks one contiguous Wasm call-stack segment from `start_ret_rsp` up to `stop_rsp`.
        unsafe fn walk_stack_segment(
            &mut self,
            code_ranges: &[RegisteredCodeRange],
            mut cur_ret_rsp: u64,
            stop_rsp: u64,
        ) {
            if cur_ret_rsp == 0 || stop_rsp == 0 {
                return;
            }
            while cur_ret_rsp < stop_rsp {
                let ret_pc = unsafe { *(cur_ret_rsp as *const usize) };
                let Some(sp) = lookup_safepoint(code_ranges, ret_pc) else {
                    break;
                };
                let frame_rsp = cur_ret_rsp + 8;
                let slots_base = sp.slots_ptr;
                for i in 0..sp.entry.num_slots {
                    let slot_off =
                        unsafe { *slots_base.add((sp.entry.first_slot_idx + i) as usize) };
                    let addr = (frame_rsp as i64 + i64::from(slot_off)) as *mut u64;
                    unsafe { self.trace_slot(addr) };
                }
                cur_ret_rsp = frame_rsp + u64::from(sp.entry.frame_caller_ret_off);
            }
        }
    }

    let mut col = Collector {
        infos: &types_snap,
        from_base,
        from_cap,
        to_alloc: to_base,
        to_scan: to_base,
        to_limit: to_base + target_cap as u64,
        extern_live: &mut extern_live,
        old_weak_tables: &old_weak_tables,
        live_weak_tables: &mut live_weak_tables,
        live_weak_table_set: &mut live_weak_table_set,
        oom: false,
    };

    // ---- 1. Trace ALL strong roots ----

    // (a) Active Wasm stack segment (if GC was triggered from compiled Wasm code).
    if gc_call_ret_rsp != 0 && store.state.gc_walk_stop_rsp != 0 {
        unsafe {
            col.walk_stack_segment(&code_ranges, gc_call_ret_rsp, store.state.gc_walk_stop_rsp);
        }
    }

    // (b) Paused outer Wasm stack segments across reentrant Host -> Wasm -> Host calls.
    let mut chain_ptr = store.state.gc_stack_chain;
    while chain_ptr != 0 {
        let node = unsafe { *(chain_ptr as *const GcStackChainNode) };
        if node.outer_paused_ret_rsp != 0 && node.outer_stop_rsp != 0 {
            unsafe {
                col.walk_stack_segment(
                    &code_ranges,
                    node.outer_paused_ret_rsp,
                    node.outer_stop_rsp,
                );
            }
        }
        chain_ptr = node.prev_chain;
    }

    // (c) In-flight `init_instance` constant-expression root buffer.
    if store.state.gc_init_roots_ptr != 0 && store.state.gc_init_roots_len != 0 {
        let ptr = store.state.gc_init_roots_ptr as *mut u64;
        for i in 0..store.state.gc_init_roots_len {
            unsafe { col.trace_slot(ptr.add(i as usize)) };
        }
    }

    // (d) All instantiated modules' defined traced globals + live passive GC elem buffers.
    for inst in &store.instances {
        let ctx_ptr = inst.ctx_buf.as_ptr() as *mut u8;
        for &goff in &inst.traced_defined_global_offs {
            unsafe { col.trace_slot(ctx_ptr.offset(goff as isize).cast::<u64>()) };
        }
        if !inst.elem_buf.is_null() {
            let elem_dropped_base = inst
                .module
                .inner
                .meta
                .elem_dropped_off
                .map(|off| unsafe { ctx_ptr.offset(off as isize) });
            for &(seg_idx, byte_off, count) in &inst.module.inner.meta.passive_gc_elem_ranges {
                let dropped =
                    elem_dropped_base.is_some_and(|p| unsafe { *p.add(seg_idx as usize) != 0 });
                if dropped {
                    continue;
                }
                let base = unsafe { inst.elem_buf.offset(byte_off as isize).cast::<u64>() };
                for i in 0..count {
                    unsafe { col.trace_slot(base.add(i as usize)) };
                }
            }
        }
    }

    // (e) Standalone host-created traced globals (`Global::new`).
    for g in &mut store.standalone_globals {
        if g.is_traced {
            unsafe { col.trace_slot(g.cell.as_mut() as *mut [u64; 2] as *mut u64) };
        }
    }

    // (f) All traced dynamic tables (`Any` / `Extern` ref families).
    for tbl in &store.tables {
        if tbl.is_traced && !tbl.desc.ptr.is_null() {
            for i in 0..tbl.desc.cur_len {
                unsafe { col.trace_slot(tbl.desc.ptr.add(i as usize)) };
            }
        }
    }

    // (g) All host `RootScope` / `Rooted` / `OwnedRooted` slots.
    for slot in &mut store.gc.host_roots {
        if slot.alive && !slot.is_exn {
            unsafe { col.trace_slot(&mut slot.value as *mut u64) };
        }
    }

    // (h) All Store exception objects' traced payload fields.
    for exn in &store.gc.exn_objects {
        for &(foff, _is_ext) in &exn.traced_fields {
            unsafe { col.trace_slot(exn.ptr.offset(foff as isize).cast::<u64>()) };
        }
    }

    // ---- 2. Drain strong Cheney BFS ----
    unsafe { col.scan_to_completion() };

    // ---- 3. Ephemeron + WeakMap Table Fixed-Point Loop ----
    for e in &mut store.gc.ephemerons {
        e.activated = false;
    }
    let mut progress = true;
    while progress && !col.oom {
        progress = false;

        // (a) Explicit `GcEphemeron` entries.
        for e in &mut store.gc.ephemerons {
            if !e.live || e.activated {
                continue;
            }
            let owner_alive = unsafe { col.forwarded_addr(e.owner).is_some() };
            let key_alive = unsafe { col.forwarded_addr(e.key).is_some() };
            if owner_alive && key_alive {
                e.activated = true;
                if e.value != 0 && (e.value & 1) == 0 {
                    let old_v = e.value;
                    let was_fwd = unsafe { col.forwarded_addr(old_v).is_some() };
                    let new_v = unsafe { col.evacuate(old_v) };
                    e.value = new_v;
                    if !was_fwd {
                        progress = true;
                        unsafe { col.scan_to_completion() };
                    }
                }
            }
        }

        // (b) Registered WeakMap reference arrays (`(key, value)` interleaved pairs).
        let mut ti = 0usize;
        while ti < col.live_weak_tables.len() && !col.oom {
            let arr_ptr = col.live_weak_tables[ti];
            ti += 1;
            let len = unsafe { *((arr_ptr + 4) as *const u32) } as usize;
            let pairs = len / 2;
            for p in 0..pairs {
                let key_slot = (arr_ptr + 8 + (p * 16) as u64) as *mut u64;
                let val_slot = (arr_ptr + 8 + (p * 16 + 8) as u64) as *mut u64;
                let k = unsafe { *key_slot };
                if k == 0 {
                    continue;
                }
                if col.in_from(k) {
                    if let Some(k_fwd) = unsafe { col.forwarded_addr(k) } {
                        unsafe { *key_slot = k_fwd };
                        let v = unsafe { *val_slot };
                        if v != 0 && (v & 1) == 0 && col.in_from(v) {
                            let nv = unsafe { col.evacuate(v) };
                            unsafe { *val_slot = nv };
                            progress = true;
                            unsafe { col.scan_to_completion() };
                        } else if v != 0 && (v & 1) == 0 {
                            col.mark_extern_word(v);
                        }
                    }
                } else if (k & 1) != 0 {
                    // Immediate i31 key (always alive if ever placed in a WeakMap table).
                    let v = unsafe { *val_slot };
                    if v != 0 && (v & 1) == 0 && col.in_from(v) {
                        let nv = unsafe { col.evacuate(v) };
                        unsafe { *val_slot = nv };
                        progress = true;
                        unsafe { col.scan_to_completion() };
                    }
                }
            }
        }
    }

    // ---- 4. Finalize weak tables, ephemerons, weak refs, and check OOM ----
    if col.oom {
        let cur_to_commit = *store.gc.space_commit_mut(to_is_b);
        if cur_to_commit > 0 {
            store.interrupt.remove_region(to_space_page_start);
            unsafe {
                let _ = libc::madvise(
                    to_space_page_start as *mut libc::c_void,
                    cur_to_commit,
                    libc::MADV_DONTNEED,
                );
                let _ = libc::mprotect(
                    to_space_page_start as *mut libc::c_void,
                    cur_to_commit,
                    libc::PROT_NONE,
                );
            }
            *store.gc.space_commit_mut(to_is_b) = 0;
            *store.gc.space_dirty_mut(to_is_b) = 0;
        }
        return Err(GcHeapOutOfMemory::new((), req_bytes));
    }

    // Clear dead `(key, value)` slots inside surviving registered WeakMap arrays.
    for &arr_ptr in col.live_weak_tables.iter() {
        let len = unsafe { *((arr_ptr + 4) as *const u32) } as usize;
        let pairs = len / 2;
        for p in 0..pairs {
            let key_slot = (arr_ptr + 8 + (p * 16) as u64) as *mut u64;
            let val_slot = (arr_ptr + 8 + (p * 16 + 8) as u64) as *mut u64;
            let k = unsafe { *key_slot };
            if k != 0 && col.in_from(k) {
                unsafe {
                    *key_slot = 0;
                    *val_slot = 0;
                }
            }
        }
    }

    // Update or retire `GcEphemeron` entries.
    for (idx, e) in store.gc.ephemerons.iter_mut().enumerate() {
        if !e.live {
            continue;
        }
        if e.activated {
            e.owner = unsafe { col.forwarded_addr(e.owner).expect("activated owner") };
            e.key = unsafe { col.forwarded_addr(e.key).expect("activated key") };
            e.activated = false;
        } else {
            e.live = false;
            e.owner = 0;
            e.key = 0;
            e.value = 0;
            store.gc.ephemerons_free.push(idx as u32);
        }
    }

    // Update or retire `WeakAnyRef` entries.
    let mut retired_any_weak = false;
    for (idx, w) in store.gc.weak_refs.iter_mut().enumerate() {
        if !w.live {
            continue;
        }
        let o_fwd = unsafe { col.forwarded_addr(w.owner) };
        let t_fwd = unsafe { col.forwarded_addr(w.target) };
        if let (Some(o), Some(t)) = (o_fwd, t_fwd) {
            w.owner = o;
            w.target = t;
        } else {
            w.live = false;
            w.owner = 0;
            w.target = 0;
            store.gc.weak_refs_free.push(idx as u32);
            retired_any_weak = true;
        }
    }
    if retired_any_weak {
        store.gc.weak_retire_count = store.gc.weak_retire_count.wrapping_add(1);
    }

    let final_alloc = col.to_alloc;
    let final_limit = col.to_limit;
    let _ = col;
    drop(types_snap);

    store.gc.weak_tables = live_weak_table_set;

    // ---- 5. Sweep unreachable `ExternRef` host payloads (strictly AFTER ephemeron fixed-point!) ----
    let mut dropped_payloads: Vec<Box<dyn Any + Send + Sync>> = Vec::new();
    for (idx, (slot, &alive)) in store
        .gc
        .extern_arena
        .iter_mut()
        .zip(extern_live.iter())
        .enumerate()
    {
        if !alive && let Some(payload) = slot.data.take() {
            store.gc.extern_free.push(idx as u32);
            dropped_payloads.push(payload);
        }
    }

    // ---- 6. Flip semi-spaces & return excess / reclaimed old pages to Linux ----
    let old_space_page_start = if old_is_b {
        store.gc.space_b_start()
    } else {
        store.gc.space_a_start()
    };
    let old_commit = *store.gc.space_commit_mut(old_is_b);
    let old_used_pages =
        ((old_alloc_ptr.saturating_sub(old_space_page_start as u64) as usize + 4095) & !4095)
            .min(old_commit);
    {
        let old_dirty = store.gc.space_dirty_mut(old_is_b);
        *old_dirty = (*old_dirty).max(old_used_pages).min(old_commit);
    }
    let warm_commit = (SPACE_START_PAD + store.gc.init_cap.min(store.gc.heap_cap) + 4095) & !4095;
    let keep_old_commit = if reclaim_old {
        0
    } else {
        warm_commit.min(old_commit)
    };
    if old_commit > keep_old_commit {
        if keep_old_commit == 0 {
            store.interrupt.remove_region(old_space_page_start);
        }
        let tail_ptr = (old_space_page_start + keep_old_commit) as *mut libc::c_void;
        let tail_len = old_commit - keep_old_commit;
        unsafe {
            let _ = libc::madvise(tail_ptr, tail_len, libc::MADV_DONTNEED);
            let _ = libc::mprotect(tail_ptr, tail_len, libc::PROT_NONE);
        }
        *store.gc.space_commit_mut(old_is_b) = keep_old_commit;
        let old_dirty = store.gc.space_dirty_mut(old_is_b);
        *old_dirty = (*old_dirty).min(keep_old_commit);
        if keep_old_commit > 0 {
            store
                .interrupt
                .set_region(old_space_page_start, keep_old_commit);
        }
    }

    store.gc.active_is_b = to_is_b;
    store.gc.from_cap = target_cap;
    store.state.gc_from_base = to_base;
    store.state.gc_from_cap = target_cap as u64;
    store.state.gc_to_base = (old_space_page_start + SPACE_START_PAD) as u64;
    store.state.gc_to_cap = 0;
    store.state.gc_alloc_ptr = final_alloc;
    store.state.gc_alloc_limit = final_limit;
    store.state.gc_count = store.state.gc_count.wrapping_add(1);

    // Drop swept `ExternRef` payloads AFTER Store GC state is completely consistent!
    drop(dropped_payloads);

    // ---- 7. Resize semi-space from actual post-GC live bytes (grow if >50% full or req_bytes
    //         doesn't fit; shrink back toward `init_cap` after transient allocation bursts) ----
    let live_bytes = store.state.gc_alloc_ptr - store.state.gc_from_base;
    let needed_total = live_bytes.saturating_add(req_bytes);
    if needed_total > heap_cap {
        return Err(GcHeapOutOfMemory::new((), req_bytes));
    }
    let cur_cap = store.gc.from_cap;
    let live_usize = live_bytes as usize;
    let needed_usize = needed_total as usize;
    let heap_cap_usize = heap_cap as usize;
    if needed_total > (cur_cap as u64)
        || (live_usize.saturating_mul(2) >= cur_cap && cur_cap < heap_cap_usize)
    {
        let goal = needed_usize.max(live_usize.saturating_mul(2));
        let mut grown_cap = cur_cap.max(64 * 1024);
        while grown_cap < goal && grown_cap < heap_cap_usize {
            grown_cap = grown_cap.saturating_mul(2).min(heap_cap_usize);
        }
        grown_cap = ((grown_cap + 4095) & !4095).min(heap_cap_usize);
        if grown_cap > cur_cap {
            let grown_commit = (SPACE_START_PAD + grown_cap + 4095) & !4095;
            let cur_to_commit = *store.gc.space_commit_mut(to_is_b);
            if grown_commit > cur_to_commit {
                let delta_ptr = (to_space_page_start + cur_to_commit) as *mut libc::c_void;
                let delta_len = grown_commit - cur_to_commit;
                let rc = unsafe {
                    libc::mprotect(delta_ptr, delta_len, libc::PROT_READ | libc::PROT_WRITE)
                };
                if rc != 0 {
                    return Err(GcHeapOutOfMemory::new((), req_bytes));
                }
                *store.gc.space_commit_mut(to_is_b) = grown_commit;
                store
                    .interrupt
                    .set_region(to_space_page_start, grown_commit);
            }
            store.gc.from_cap = grown_cap;
            store.state.gc_from_cap = grown_cap as u64;
            store.state.gc_alloc_limit = store.state.gc_from_base + grown_cap as u64;
        }
    } else if cur_cap > store.gc.init_cap
        && live_usize.saturating_mul(4) < cur_cap
        && needed_usize <= cur_cap / 2
    {
        let shrunk_cap = ((live_usize
            .saturating_mul(2)
            .max(needed_usize)
            .max(store.gc.init_cap)
            + 4095)
            & !4095)
            .min(cur_cap);
        if shrunk_cap < cur_cap {
            let shrunk_commit = (SPACE_START_PAD + shrunk_cap + 4095) & !4095;
            let cur_to_commit = *store.gc.space_commit_mut(to_is_b);
            if shrunk_commit < cur_to_commit {
                let tail_ptr = (to_space_page_start + shrunk_commit) as *mut libc::c_void;
                let tail_len = cur_to_commit - shrunk_commit;
                unsafe {
                    let _ = libc::madvise(tail_ptr, tail_len, libc::MADV_DONTNEED);
                    let _ = libc::mprotect(tail_ptr, tail_len, libc::PROT_NONE);
                }
                *store.gc.space_commit_mut(to_is_b) = shrunk_commit;
                let to_dirty = store.gc.space_dirty_mut(to_is_b);
                *to_dirty = (*to_dirty).min(shrunk_commit);
                store
                    .interrupt
                    .set_region(to_space_page_start, shrunk_commit);
            }
            store.gc.from_cap = shrunk_cap;
            store.state.gc_from_cap = shrunk_cap as u64;
            store.state.gc_alloc_limit = store.state.gc_from_base + shrunk_cap as u64;
        }
    }

    Ok(())
}

struct SafepointHit {
    entry: SafepointEntry,
    slots_ptr: *const i32,
}

#[inline]
fn lookup_safepoint(ranges: &[RegisteredCodeRange], ret_pc: usize) -> Option<SafepointHit> {
    let idx = ranges.partition_point(|r| r.code_end <= ret_pc);
    let range = ranges.get(idx)?;
    if ret_pc < range.code_start || ret_pc >= range.code_end || range.safepoint_header.is_null() {
        return None;
    }
    let rel_pc = u32::try_from(ret_pc - range.code_start).ok()?;
    let hdr = unsafe { *range.safepoint_header };
    if hdr.num_entries == 0 {
        return None;
    }
    let hdr_bytes = range.safepoint_header.cast::<u8>();
    let entries = unsafe {
        std::slice::from_raw_parts(
            hdr_bytes
                .offset(hdr.entries_rel_off as isize)
                .cast::<SafepointEntry>(),
            hdr.num_entries as usize,
        )
    };
    let pos = entries
        .binary_search_by_key(&rel_pc, |e| e.ret_pc_rel)
        .ok()?;
    let slots_ptr = unsafe { hdr_bytes.offset(hdr.slots_rel_off as isize).cast::<i32>() };
    Some(SafepointHit {
        entry: entries[pos],
        slots_ptr,
    })
}

/// Allocates `size_bytes` (8-byte aligned) in the Store's active GC semi-space, triggering GC
/// if necessary and returning `Err(GcHeapOutOfMemory)` on recoverable OOM.
pub(crate) unsafe fn gc_bump_alloc(
    store: &mut StoreInner,
    size_bytes: u64,
) -> std::result::Result<u64, GcHeapOutOfMemory<()>> {
    debug_assert!((size_bytes & 7) == 0 && size_bytes >= 8);
    if store.state.gc_alloc_ptr.saturating_add(size_bytes) > store.state.gc_alloc_limit {
        unsafe { collect_store_gc(store, size_bytes, 0, false)? };
    }
    let ptr = store.state.gc_alloc_ptr;
    store.state.gc_alloc_ptr = ptr + size_bytes;
    Ok(ptr)
}

// =========================================================================
// Public Rooted GC Reference Types (`RootScope`, `Rooted<T>`, `OwnedRooted<T>`, ...)
// =========================================================================

pub trait GcRefType: Copy + 'static {}

#[derive(Clone, Copy, Debug)]
pub struct AnyRef;
#[derive(Clone, Copy, Debug)]
pub struct EqRef;
#[derive(Clone, Copy, Debug)]
pub struct StructRef;
#[derive(Clone, Copy, Debug)]
pub struct ArrayRef;
#[derive(Clone, Copy, Debug)]
pub struct ExternRef;
#[derive(Clone, Copy, Debug)]
pub struct ExnRef;

impl GcRefType for AnyRef {}
impl GcRefType for EqRef {}
impl GcRefType for StructRef {}
impl GcRefType for ArrayRef {}
impl GcRefType for ExternRef {}
impl GcRefType for ExnRef {}

/// A GC reference rooted for the lifetime of the enclosing `RootScope` (or `Store` if created outside a scope).
#[derive(Debug)]
pub struct Rooted<T: GcRefType> {
    pub(crate) store_id: u64,
    pub(crate) slot_idx: u32,
    pub(crate) generation: u32,
    pub(crate) _marker: PhantomData<T>,
}

impl<T: GcRefType> Clone for Rooted<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: GcRefType> Copy for Rooted<T> {}

struct OwnedRootToken {
    slot_idx: u32,
    generation: u32,
    cleanup: Arc<std::sync::Mutex<Vec<(u32, u32)>>>,
}

impl Drop for OwnedRootToken {
    fn drop(&mut self) {
        self.cleanup
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((self.slot_idx, self.generation));
    }
}

/// An owned GC root that stays alive across `RootScope`s until all clones are dropped.
pub struct OwnedRooted<T: GcRefType> {
    pub(crate) store_id: u64,
    token: Arc<OwnedRootToken>,
    pub(crate) _marker: PhantomData<T>,
}

impl<T: GcRefType> Clone for OwnedRooted<T> {
    fn clone(&self) -> Self {
        Self {
            store_id: self.store_id,
            token: Arc::clone(&self.token),
            _marker: PhantomData,
        }
    }
}

impl<T: GcRefType> std::fmt::Debug for OwnedRooted<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedRooted")
            .field("store_id", &self.store_id)
            .field("slot_idx", &self.token.slot_idx)
            .field("generation", &self.token.generation)
            .finish()
    }
}

impl<T: GcRefType> OwnedRooted<T> {
    pub fn to_rooted(&self, mut store: impl AsContextMut) -> Rooted<T> {
        let inner = store.as_store_inner_mut();
        assert_eq!(
            inner.id, self.store_id,
            "OwnedRooted belongs to another Store"
        );
        let val = inner
            .gc
            .read_host_root(self.token.slot_idx, self.token.generation)
            .expect("live OwnedRooted");
        let slot = &inner.gc.host_roots[self.token.slot_idx as usize];
        let (is_ext, is_exn) = (slot.is_extern, slot.is_exn);
        let (idx, gen_id) = inner.gc.alloc_host_root(val, is_ext, is_exn, false);
        Rooted {
            store_id: self.store_id,
            slot_idx: idx,
            generation: gen_id,
            _marker: PhantomData,
        }
    }

    pub fn to_raw(&self, mut store: impl AsContextMut) -> Result<u32> {
        let inner = store.as_store_inner_mut();
        if inner.id != self.store_id {
            return Err(Error::msg("OwnedRooted belongs to another Store"));
        }
        let val = inner
            .gc
            .read_host_root(self.token.slot_idx, self.token.generation)?;
        Ok(inner.gc.abi_word_to_raw_u32(val))
    }
}

/// Trait allowing `Rooted::ref_eq` and `WeakAnyRef::new` to accept both `Rooted<T>` and `OwnedRooted<T>`.
pub trait RootedRef<T: GcRefType> {
    fn root_handle(&self) -> (u64, u32, u32);
}

impl<T: GcRefType> RootedRef<T> for Rooted<T> {
    fn root_handle(&self) -> (u64, u32, u32) {
        (self.store_id, self.slot_idx, self.generation)
    }
}

impl<T: GcRefType> RootedRef<T> for OwnedRooted<T> {
    fn root_handle(&self) -> (u64, u32, u32) {
        (self.store_id, self.token.slot_idx, self.token.generation)
    }
}

impl<T: GcRefType, R: RootedRef<T> + ?Sized> RootedRef<T> for &R {
    fn root_handle(&self) -> (u64, u32, u32) {
        (**self).root_handle()
    }
}

/// Lexically scoped GC root frame: frees all `Rooted<T>` created inside it on `Drop`.
pub struct RootScope<'a, S: AsContextMut> {
    pub(crate) store: &'a mut S,
}

impl<'a, S: AsContextMut> RootScope<'a, S> {
    pub fn new(store: &'a mut S) -> Self {
        store.as_store_inner_mut().gc.scope_stack.push(Vec::new());
        Self { store }
    }
}

impl<S: AsContextMut> Drop for RootScope<'_, S> {
    fn drop(&mut self) {
        let inner = self.store.as_store_inner_mut();
        if let Some(frame) = inner.gc.scope_stack.pop() {
            for (idx, gen_id) in frame {
                inner.gc.free_host_root(idx, gen_id);
            }
        }
    }
}

impl<S: AsContextMut> AsContext for RootScope<'_, S> {
    type Data = S::Data;
    fn as_context(&self) -> crate::store::StoreContext<'_, Self::Data> {
        self.store.as_context()
    }
    fn as_store_inner(&self) -> &StoreInner {
        self.store.as_store_inner()
    }
}

impl<S: AsContextMut> AsContextMut for RootScope<'_, S> {
    fn as_context_mut(&mut self) -> crate::store::StoreContextMut<'_, Self::Data> {
        self.store.as_context_mut()
    }
    fn as_store_inner_mut(&mut self) -> &mut StoreInner {
        self.store.as_store_inner_mut()
    }
}

impl<T: GcRefType> Rooted<T> {
    pub fn ref_eq(
        store: impl AsContext,
        a: &impl RootedRef<T>,
        b: &impl RootedRef<T>,
    ) -> Result<bool> {
        let inner = store.as_store_inner();
        let (sid_a, idx_a, gen_a) = a.root_handle();
        let (sid_b, idx_b, gen_b) = b.root_handle();
        if sid_a != inner.id || sid_b != inner.id {
            return Err(Error::msg(
                "Rooted::ref_eq called with handle from another Store",
            ));
        }
        let va = inner.gc.read_host_root(idx_a, gen_a)?;
        let vb = inner.gc.read_host_root(idx_b, gen_b)?;
        Ok(va == vb)
    }

    pub fn to_owned_rooted(&self, mut store: impl AsContextMut) -> Result<OwnedRooted<T>> {
        let inner = store.as_store_inner_mut();
        if inner.id != self.store_id {
            return Err(Error::msg("Rooted belongs to another Store"));
        }
        let val = inner.gc.read_host_root(self.slot_idx, self.generation)?;
        let slot = &inner.gc.host_roots[self.slot_idx as usize];
        let (is_ext, is_exn) = (slot.is_extern, slot.is_exn);
        let (idx, gen_id) = inner.gc.alloc_host_root(val, is_ext, is_exn, true);
        Ok(OwnedRooted {
            store_id: self.store_id,
            token: Arc::new(OwnedRootToken {
                slot_idx: idx,
                generation: gen_id,
                cleanup: Arc::clone(&inner.dropped_owned_roots),
            }),
            _marker: PhantomData,
        })
    }

    pub fn to_raw(&self, mut store: impl AsContextMut) -> Result<u32> {
        let inner = store.as_store_inner_mut();
        if inner.id != self.store_id {
            return Err(Error::msg("Rooted belongs to another Store"));
        }
        let val = inner.gc.read_host_root(self.slot_idx, self.generation)?;
        Ok(inner.gc.abi_word_to_raw_u32(val))
    }

    pub(crate) fn read_abi_word(&self, store: &StoreInner) -> Result<u64> {
        if store.id != self.store_id {
            return Err(Error::msg("Rooted belongs to another Store"));
        }
        store.gc.read_host_root(self.slot_idx, self.generation)
    }

    pub(crate) fn from_abi_word(
        store: &mut StoreInner,
        word: u64,
        is_extern: bool,
        is_exn: bool,
    ) -> Self {
        let (idx, gen_id) = store.gc.alloc_host_root(word, is_extern, is_exn, false);
        Self {
            store_id: store.id,
            slot_idx: idx,
            generation: gen_id,
            _marker: PhantomData,
        }
    }
}

// ---- Reference conversions & downcasts ----

impl Rooted<EqRef> {
    pub fn to_anyref(self) -> Rooted<AnyRef> {
        Rooted {
            store_id: self.store_id,
            slot_idx: self.slot_idx,
            generation: self.generation,
            _marker: PhantomData,
        }
    }

    pub fn ty(&self, store: impl AsContext) -> Result<crate::types::HeapType> {
        self.to_anyref().ty(store)
    }

    pub fn matches_ty(
        &self,
        store: impl AsContext,
        expected: &crate::types::HeapType,
    ) -> Result<bool> {
        Ok(self.ty(store)?.matches(expected))
    }

    pub fn as_struct(&self, store: impl AsContext) -> Result<Option<Rooted<StructRef>>> {
        self.to_anyref().as_struct(store)
    }

    pub fn as_array(&self, store: impl AsContext) -> Result<Option<Rooted<ArrayRef>>> {
        self.to_anyref().as_array(store)
    }

    pub fn as_i31(&self, store: impl AsContext) -> Result<Option<I31>> {
        self.to_anyref().as_i31(store)
    }
}

impl Rooted<StructRef> {
    pub fn to_anyref(self) -> Rooted<AnyRef> {
        Rooted {
            store_id: self.store_id,
            slot_idx: self.slot_idx,
            generation: self.generation,
            _marker: PhantomData,
        }
    }

    pub fn to_eqref(self) -> Rooted<EqRef> {
        Rooted {
            store_id: self.store_id,
            slot_idx: self.slot_idx,
            generation: self.generation,
            _marker: PhantomData,
        }
    }

    pub fn ty(&self, store: impl AsContext) -> Result<StructType> {
        let inner = store.as_store_inner();
        let word = self.read_abi_word(inner)?;
        if word == 0 || (word & 1) != 0 {
            return Err(Error::msg("StructRef is null or i31"));
        }
        let hdr = unsafe { *(word as *const u64) };
        if (hdr as u32 & 6) != GC_HDR_KIND_STRUCT {
            return Err(Error::msg("GC object is not a struct"));
        }
        let cid = (hdr as u32) >> 3;
        Ok(StructType::from_canon(Arc::clone(&inner.engine.inner), cid))
    }

    pub fn matches_ty(&self, store: impl AsContext, expected: &StructType) -> Result<bool> {
        Ok(self.ty(store)?.matches(expected))
    }

    pub fn field(&self, mut store: impl AsContextMut, index: usize) -> Result<Val> {
        let inner = store.as_store_inner_mut();
        let word = self.read_abi_word(inner)?;
        if word == 0 || (word & 1) != 0 {
            return Err(Error::msg("StructRef is null or i31"));
        }
        let hdr = unsafe { *(word as *const u64) };
        let cid = (hdr as u32) >> 3;
        let (offset, storage) = {
            let CanonComposite::Struct { fields, .. } = &inner.ensure_type_info(cid).composite
            else {
                return Err(Error::msg("GC object is not a struct"));
            };
            let f = fields
                .get(index)
                .ok_or_else(|| Error::msg("struct field index out of bounds"))?;
            (f.offset, f.storage)
        };
        unsafe { read_gc_storage(inner, (word + u64::from(offset)) as *const u8, storage) }
    }

    pub fn set_field(&self, mut store: impl AsContextMut, index: usize, val: Val) -> Result<()> {
        let inner = store.as_store_inner_mut();
        let word = self.read_abi_word(inner)?;
        if word == 0 || (word & 1) != 0 {
            return Err(Error::msg("StructRef is null or i31"));
        }
        let hdr = unsafe { *(word as *const u64) };
        let cid = (hdr as u32) >> 3;
        let (offset, storage, mutable) = {
            let CanonComposite::Struct { fields, .. } = &inner.ensure_type_info(cid).composite
            else {
                return Err(Error::msg("GC object is not a struct"));
            };
            let f = fields
                .get(index)
                .ok_or_else(|| Error::msg("struct field index out of bounds"))?;
            (f.offset, f.storage, f.mutable)
        };
        if !mutable {
            return Err(Error::msg("cannot mutate immutable struct field"));
        }
        unsafe { write_gc_storage(inner, (word + u64::from(offset)) as *mut u8, storage, &val) }
    }
}

impl Rooted<ArrayRef> {
    pub fn to_anyref(self) -> Rooted<AnyRef> {
        Rooted {
            store_id: self.store_id,
            slot_idx: self.slot_idx,
            generation: self.generation,
            _marker: PhantomData,
        }
    }

    pub fn to_eqref(self) -> Rooted<EqRef> {
        Rooted {
            store_id: self.store_id,
            slot_idx: self.slot_idx,
            generation: self.generation,
            _marker: PhantomData,
        }
    }

    pub fn ty(&self, store: impl AsContext) -> Result<ArrayType> {
        let inner = store.as_store_inner();
        let word = self.read_abi_word(inner)?;
        let hdr = unsafe { *(word as *const u64) };
        if (hdr as u32 & 6) != GC_HDR_KIND_ARRAY {
            return Err(Error::msg("GC object is not an array"));
        }
        let cid = (hdr as u32) >> 3;
        Ok(ArrayType::from_canon(Arc::clone(&inner.engine.inner), cid))
    }

    pub fn matches_ty(&self, store: impl AsContext, expected: &ArrayType) -> Result<bool> {
        Ok(self.ty(store)?.matches(expected))
    }

    pub fn elems(&self, mut store: impl AsContextMut) -> Result<std::vec::IntoIter<Val>> {
        let inner = store.as_store_inner_mut();
        let word = self.read_abi_word(inner)?;
        let len = unsafe { *((word + 4) as *const u32) };
        let hdr = unsafe { *(word as *const u64) };
        let cid = (hdr as u32) >> 3;
        let CanonComposite::Array { elem, .. } = &inner.ensure_type_info(cid).composite else {
            return Err(Error::msg("GC object is not an array"));
        };
        let elem = *elem;
        let elem_sz = u64::from(elem.byte_size());
        let mut out = Vec::with_capacity(len as usize);
        for i in 0..len {
            let off = 8u64 + u64::from(i) * elem_sz;
            out.push(unsafe { read_gc_storage(inner, (word + off) as *const u8, elem)? });
        }
        Ok(out.into_iter())
    }

    pub fn len(&self, store: impl AsContext) -> Result<u32> {
        let inner = store.as_store_inner();
        let word = self.read_abi_word(inner)?;
        Ok(unsafe { *((word + 4) as *const u32) })
    }

    pub fn is_empty(&self, store: impl AsContext) -> Result<bool> {
        Ok(self.len(store)? == 0)
    }

    pub fn get(&self, mut store: impl AsContextMut, index: u32) -> Result<Val> {
        let inner = store.as_store_inner_mut();
        let word = self.read_abi_word(inner)?;
        let len = unsafe { *((word + 4) as *const u32) };
        if index >= len {
            return Err(Trap::ArrayOutOfBounds.into());
        }
        let hdr = unsafe { *(word as *const u64) };
        let cid = (hdr as u32) >> 3;
        let CanonComposite::Array { elem, .. } = &inner.ensure_type_info(cid).composite else {
            return Err(Error::msg("GC object is not an array"));
        };
        let elem = *elem;
        let off = 8u64 + u64::from(index) * u64::from(elem.byte_size());
        unsafe { read_gc_storage(inner, (word + off) as *const u8, elem) }
    }

    pub fn set(&self, mut store: impl AsContextMut, index: u32, val: Val) -> Result<()> {
        let inner = store.as_store_inner_mut();
        let word = self.read_abi_word(inner)?;
        let len = unsafe { *((word + 4) as *const u32) };
        if index >= len {
            return Err(Trap::ArrayOutOfBounds.into());
        }
        let hdr = unsafe { *(word as *const u64) };
        let cid = (hdr as u32) >> 3;
        let CanonComposite::Array { elem, mutable, .. } = &inner.ensure_type_info(cid).composite
        else {
            return Err(Error::msg("GC object is not an array"));
        };
        let (elem, mutable) = (*elem, *mutable);
        if !mutable {
            return Err(Error::msg("cannot mutate immutable array element"));
        }
        let off = 8u64 + u64::from(index) * u64::from(elem.byte_size());
        unsafe { write_gc_storage(inner, (word + off) as *mut u8, elem, &val) }
    }

    /// Bulk-copies `[start..start + dst.len()]` of an `i8`-packed GC array into `dst`.
    pub fn copy_i8_range_to_slice(
        &self,
        mut store: impl AsContextMut,
        start: u32,
        dst: &mut [u8],
    ) -> Result<()> {
        let inner = store.as_store_inner_mut();
        let word = self.read_abi_word(inner)?;
        let hdr = unsafe { *(word as *const u64) };
        let cid = (hdr as u32) >> 3;
        let is_i8 = matches!(
            &inner.ensure_type_info(cid).composite,
            CanonComposite::Array {
                elem: GcStorageKind::I8,
                ..
            }
        );
        if !is_i8 {
            return Err(Error::msg("copy_i8_range_to_slice requires an i8 array"));
        }
        let len = unsafe { *((word + 4) as *const u32) } as usize;
        let start = start as usize;
        let end = start
            .checked_add(dst.len())
            .filter(|&e| e <= len)
            .ok_or(Trap::ArrayOutOfBounds)?;
        let _ = end;
        if !dst.is_empty() {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    (word + 8 + start as u64) as *const u8,
                    dst.as_mut_ptr(),
                    dst.len(),
                );
            }
        }
        Ok(())
    }

    /// Bulk-copies all bytes of an `i8`-packed GC array directly into `dst` (zero per-element overhead!).
    pub fn copy_to_i8_slice(&self, mut store: impl AsContextMut, dst: &mut [u8]) -> Result<()> {
        let inner = store.as_store_inner_mut();
        let word = self.read_abi_word(inner)?;
        let hdr = unsafe { *(word as *const u64) };
        let cid = (hdr as u32) >> 3;
        let is_i8 = matches!(
            &inner.ensure_type_info(cid).composite,
            CanonComposite::Array {
                elem: GcStorageKind::I8,
                ..
            }
        );
        if !is_i8 {
            return Err(Error::msg("copy_to_i8_slice requires an i8 array"));
        }
        let len = unsafe { *((word + 4) as *const u32) } as usize;
        if dst.len() != len {
            return Err(Trap::ArrayOutOfBounds.into());
        }
        if len > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping((word + 8) as *const u8, dst.as_mut_ptr(), len);
            }
        }
        Ok(())
    }

    /// Bulk-copies all UTF-16 code units of an `i16`-packed GC array directly into `dst`.
    pub fn copy_to_i16_slice(&self, mut store: impl AsContextMut, dst: &mut [u16]) -> Result<()> {
        let inner = store.as_store_inner_mut();
        let word = self.read_abi_word(inner)?;
        let hdr = unsafe { *(word as *const u64) };
        let cid = (hdr as u32) >> 3;
        let is_i16 = matches!(
            &inner.ensure_type_info(cid).composite,
            CanonComposite::Array {
                elem: GcStorageKind::I16,
                ..
            }
        );
        if !is_i16 {
            return Err(Error::msg("copy_to_i16_slice requires an i16 array"));
        }
        let len = unsafe { *((word + 4) as *const u32) } as usize;
        if dst.len() != len {
            return Err(Trap::ArrayOutOfBounds.into());
        }
        if len > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    (word + 8) as *const u8,
                    dst.as_mut_ptr().cast::<u8>(),
                    len * 2,
                );
            }
        }
        Ok(())
    }
}

impl Rooted<AnyRef> {
    pub fn ty(&self, store: impl AsContext) -> Result<crate::types::HeapType> {
        let inner = store.as_store_inner();
        let word = self.read_abi_word(inner)?;
        if word == 0 {
            return Ok(crate::types::HeapType::None);
        }
        if (word & 1) != 0 {
            return Ok(crate::types::HeapType::I31);
        }
        let hdr = unsafe { *(word as *const u64) };
        let kind = hdr as u32 & 6;
        let cid = (hdr as u32) >> 3;
        match kind {
            GC_HDR_KIND_STRUCT => Ok(crate::types::HeapType::ConcreteStruct(
                StructType::from_canon(Arc::clone(&inner.engine.inner), cid),
            )),
            GC_HDR_KIND_ARRAY => Ok(crate::types::HeapType::ConcreteArray(
                ArrayType::from_canon(Arc::clone(&inner.engine.inner), cid),
            )),
            _ => Ok(crate::types::HeapType::Any),
        }
    }

    pub fn matches_ty(
        &self,
        store: impl AsContext,
        expected: &crate::types::HeapType,
    ) -> Result<bool> {
        Ok(self.ty(store)?.matches(expected))
    }

    pub fn as_i31(&self, store: impl AsContext) -> Result<Option<I31>> {
        let inner = store.as_store_inner();
        let word = self.read_abi_word(inner)?;
        Ok(I31::from_abi_word(word))
    }

    pub fn as_eqref(&self, store: impl AsContext) -> Result<Option<Rooted<EqRef>>> {
        let inner = store.as_store_inner();
        let word = self.read_abi_word(inner)?;
        if word == 0 {
            return Ok(None);
        }
        if (word & 1) != 0 {
            return Ok(Some(Rooted {
                store_id: self.store_id,
                slot_idx: self.slot_idx,
                generation: self.generation,
                _marker: PhantomData,
            }));
        }
        let hdr = unsafe { *(word as *const u64) };
        if (hdr as u32 & 6) == GC_HDR_KIND_INTERNAL_EXTERN {
            Ok(None)
        } else {
            Ok(Some(Rooted {
                store_id: self.store_id,
                slot_idx: self.slot_idx,
                generation: self.generation,
                _marker: PhantomData,
            }))
        }
    }

    pub fn as_struct(&self, store: impl AsContext) -> Result<Option<Rooted<StructRef>>> {
        let inner = store.as_store_inner();
        let word = self.read_abi_word(inner)?;
        if word == 0 || (word & 1) != 0 {
            return Ok(None);
        }
        let hdr = unsafe { *(word as *const u64) };
        if (hdr as u32 & 6) == GC_HDR_KIND_STRUCT {
            Ok(Some(Rooted {
                store_id: self.store_id,
                slot_idx: self.slot_idx,
                generation: self.generation,
                _marker: PhantomData,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn as_array(&self, store: impl AsContext) -> Result<Option<Rooted<ArrayRef>>> {
        let inner = store.as_store_inner();
        let word = self.read_abi_word(inner)?;
        if word == 0 || (word & 1) != 0 {
            return Ok(None);
        }
        let hdr = unsafe { *(word as *const u64) };
        if (hdr as u32 & 6) == GC_HDR_KIND_ARRAY {
            Ok(Some(Rooted {
                store_id: self.store_id,
                slot_idx: self.slot_idx,
                generation: self.generation,
                _marker: PhantomData,
            }))
        } else {
            Ok(None)
        }
    }
}

impl AnyRef {
    pub fn from_i31(mut store: impl AsContextMut, val: I31) -> Rooted<AnyRef> {
        let inner = store.as_store_inner_mut();
        Rooted::from_abi_word(inner, val.to_abi_word(), false, false)
    }

    pub fn convert_extern(
        mut store: impl AsContextMut,
        extern_ref: Rooted<ExternRef>,
    ) -> Result<Rooted<AnyRef>> {
        let inner = store.as_store_inner_mut();
        let word = extern_ref.read_abi_word(inner)?;
        if word == 0 {
            return Err(Error::msg(
                "null externref cannot be converted to non-null anyref",
            ));
        }
        if (word & 0xffff_ffff_0000_0000) == GC_EXT_I31_TAG_HI {
            return Ok(Rooted::from_abi_word(
                inner,
                word & 0xffff_ffff,
                false,
                false,
            ));
        }
        if inner
            .gc
            .is_in_from_space(word, inner.state.gc_from_base, inner.state.gc_from_cap)
        {
            return Ok(Rooted::from_abi_word(inner, word, false, false));
        }
        // Box host externref into a 16-byte `InternalExternBox` on the GC heap!
        let box_ptr = unsafe { gc_bump_alloc(inner, 16) }.map_err(Error::from)?;
        // Re-read `extern_ref` in case `gc_bump_alloc` collected (host externref word doesn't move,
        // and its slot stays marked live because `extern_ref` is rooted in `host_roots`).
        let word = extern_ref.read_abi_word(inner)?;
        unsafe {
            *(box_ptr as *mut u64) = u64::from(GC_HDR_KIND_INTERNAL_EXTERN);
            *((box_ptr + 8) as *mut u64) = word;
        }
        Ok(Rooted::from_abi_word(inner, box_ptr, false, false))
    }
}

impl ExternRef {
    pub fn new<T: Any + Send + Sync + 'static>(
        mut store: impl AsContextMut,
        data: T,
    ) -> Result<Rooted<ExternRef>> {
        let inner = store.as_store_inner_mut();
        let word = inner.gc.alloc_extern_data(Box::new(data));
        Ok(Rooted::from_abi_word(inner, word, true, false))
    }

    pub fn convert_any(
        mut store: impl AsContextMut,
        any_ref: Rooted<AnyRef>,
    ) -> Result<Rooted<ExternRef>> {
        let inner = store.as_store_inner_mut();
        let word = any_ref.read_abi_word(inner)?;
        let ext_word = if word == 0 {
            0
        } else if (word & 1) != 0 {
            GC_EXT_I31_TAG_HI | (word & 0xffff_ffff)
        } else {
            let hdr = unsafe { *(word as *const u64) };
            if (hdr as u32 & 6) == GC_HDR_KIND_INTERNAL_EXTERN {
                unsafe { *((word + 8) as *const u64) }
            } else {
                word
            }
        };
        Ok(Rooted::from_abi_word(inner, ext_word, true, false))
    }
}

impl Rooted<ExternRef> {
    /// Returns a reference to the host payload stored inside this `ExternRef`.
    pub fn data<'a, T: 'static>(
        &self,
        store: impl Into<crate::store::StoreContext<'a, T>>,
    ) -> Result<Option<&'a (dyn Any + Send + Sync)>> {
        let ctx: crate::store::StoreContext<'a, T> = store.into();
        let word = self.read_abi_word(ctx.inner)?;
        Ok(ctx.inner.gc.get_extern_data(word))
    }
}

/// Pre-validated struct allocator (`StructRefPre`).
#[derive(Clone, Debug)]
pub struct StructRefPre {
    pub(crate) ty: StructType,
}

impl StructRefPre {
    pub fn new(_store: impl AsContext, ty: StructType) -> Self {
        Self { ty }
    }
}

impl StructRef {
    pub fn new(
        mut store: impl AsContextMut,
        pre: &StructRefPre,
        fields: &[Val],
    ) -> Result<Rooted<StructRef>> {
        let inner = store.as_store_inner_mut();
        let cid = pre.ty.canon_id;
        let (total_bytes, field_specs) = {
            let CanonComposite::Struct {
                fields: fdefs,
                total_bytes,
                ..
            } = &inner.ensure_type_info(cid).composite
            else {
                return Err(Error::msg("StructRefPre is not a struct type"));
            };
            if fields.len() != fdefs.len() {
                return Err(Error::msg("StructRef::new field count mismatch"));
            }
            let specs: Vec<(u32, GcStorageKind)> =
                fdefs.iter().map(|f| (f.offset, f.storage)).collect();
            (*total_bytes, specs)
        };
        let ptr = unsafe { gc_bump_alloc(inner, u64::from(total_bytes)) }.map_err(Error::from)?;
        let hdr = u64::from((cid << 3) | GC_HDR_KIND_STRUCT);
        unsafe {
            std::ptr::write_bytes(ptr as *mut u8, 0, total_bytes as usize);
            *(ptr as *mut u64) = hdr;
        }
        // Write fields AFTER allocation (values are already rooted in `Val`, so `read_abi_word` reads
        // post-GC moved pointers if `gc_bump_alloc` triggered a collection!).
        for ((off, sk), val) in field_specs.into_iter().zip(fields.iter()) {
            unsafe { write_gc_storage(inner, (ptr + u64::from(off)) as *mut u8, sk, val)? };
        }
        Ok(Rooted::from_abi_word(inner, ptr, false, false))
    }
}

/// Pre-validated array allocator (`ArrayRefPre`).
#[derive(Clone, Debug)]
pub struct ArrayRefPre {
    pub(crate) ty: ArrayType,
}

impl ArrayRefPre {
    pub fn new(_store: impl AsContext, ty: ArrayType) -> Self {
        Self { ty }
    }
}

impl ArrayRef {
    pub fn new(
        mut store: impl AsContextMut,
        pre: &ArrayRefPre,
        init: &Val,
        len: u32,
    ) -> Result<Rooted<ArrayRef>> {
        let inner = store.as_store_inner_mut();
        let cid = pre.ty.canon_id;
        let elem = {
            let CanonComposite::Array { elem, .. } = &inner.ensure_type_info(cid).composite else {
                return Err(Error::msg("ArrayRefPre is not an array type"));
            };
            *elem
        };
        let elem_sz = u64::from(elem.byte_size());
        let data_bytes = u64::from(len)
            .checked_mul(elem_sz)
            .ok_or_else(|| Error::from(GcHeapOutOfMemory::new((), u64::MAX)))?;
        let total_bytes = 8u64
            .checked_add(data_bytes)
            .and_then(|v| v.checked_add(7))
            .map(|v| v & !7)
            .ok_or_else(|| Error::from(GcHeapOutOfMemory::new((), u64::MAX)))?;

        let ptr = unsafe { gc_bump_alloc(inner, total_bytes) }.map_err(Error::from)?;
        let hdr_u32 = (cid << 3) | GC_HDR_KIND_ARRAY;
        unsafe {
            *(ptr as *mut u32) = hdr_u32;
            *((ptr + 4) as *mut u32) = len;
        }
        if len > 0 {
            // Fast path for zero-filled byte/scalar arrays.
            if matches!(
                init,
                Val::I32(0)
                    | Val::I64(0)
                    | Val::AnyRef(None)
                    | Val::ExternRef(None)
                    | Val::FuncRef(None)
            ) {
                let data_start = ptr + 8;
                // Zero any bytes overlapping the current partial page OR this semi-space's
                // warm-reuse dirty watermark; pages strictly beyond both are freshly committed /
                // `MADV_DONTNEED`ed anonymous memory guaranteed OS-zeroed (avoiding faulting
                // multi-GiB zero arrays while remaining sound across warm semi-space reuse).
                let active_dirty_addr =
                    (inner.gc.active_space_start() + inner.gc.active_dirty()) as u64;
                let first_page_end = (data_start + 4095) & !4095;
                let must_zero_end = first_page_end
                    .max(active_dirty_addr)
                    .min(data_start + data_bytes);
                let zero_len = must_zero_end.saturating_sub(data_start);
                if zero_len > 0 {
                    unsafe {
                        std::ptr::write_bytes(data_start as *mut u8, 0, zero_len as usize);
                    }
                }
            } else {
                for i in 0..len {
                    let off = 8u64 + u64::from(i) * elem_sz;
                    unsafe { write_gc_storage(inner, (ptr + off) as *mut u8, elem, init)? };
                }
            }
        }
        Ok(Rooted::from_abi_word(inner, ptr, false, false))
    }

    /// Allocates a GC array initialized with the given slice of element values.
    pub fn new_fixed(
        mut store: impl AsContextMut,
        pre: &ArrayRefPre,
        elems: &[Val],
    ) -> Result<Rooted<ArrayRef>> {
        let len = u32::try_from(elems.len())
            .map_err(|_| Error::from(GcHeapOutOfMemory::new((), elems.len() as u64)))?;
        let inner = store.as_store_inner_mut();
        let cid = pre.ty.canon_id;
        let elem = {
            let CanonComposite::Array { elem, .. } = &inner.ensure_type_info(cid).composite else {
                return Err(Error::msg("ArrayRefPre is not an array type"));
            };
            *elem
        };
        let elem_sz = u64::from(elem.byte_size());
        let data_bytes = u64::from(len)
            .checked_mul(elem_sz)
            .ok_or_else(|| Error::from(GcHeapOutOfMemory::new((), u64::MAX)))?;
        let total_bytes = 8u64
            .checked_add(data_bytes)
            .and_then(|v| v.checked_add(7))
            .map(|v| v & !7)
            .ok_or_else(|| Error::from(GcHeapOutOfMemory::new((), u64::MAX)))?;

        let ptr = unsafe { gc_bump_alloc(inner, total_bytes) }.map_err(Error::from)?;
        let hdr_u32 = (cid << 3) | GC_HDR_KIND_ARRAY;
        unsafe {
            *(ptr as *mut u32) = hdr_u32;
            *((ptr + 4) as *mut u32) = len;
        }
        for (i, val) in elems.iter().enumerate() {
            let off = 8u64 + (i as u64) * elem_sz;
            unsafe { write_gc_storage(inner, (ptr + off) as *mut u8, elem, val)? };
        }
        Ok(Rooted::from_abi_word(inner, ptr, false, false))
    }

    /// Allocates an `i8`-packed GC array initialized directly from `bytes` via bulk `memcpy`.
    pub fn new_from_i8_slice(
        mut store: impl AsContextMut,
        pre: &ArrayRefPre,
        bytes: &[u8],
    ) -> Result<Rooted<ArrayRef>> {
        let len = u32::try_from(bytes.len())
            .map_err(|_| Error::from(GcHeapOutOfMemory::new((), bytes.len() as u64)))?;
        let inner = store.as_store_inner_mut();
        let cid = pre.ty.canon_id;
        let is_i8 = matches!(
            &inner.ensure_type_info(cid).composite,
            CanonComposite::Array {
                elem: GcStorageKind::I8,
                ..
            }
        );
        if !is_i8 {
            return Err(Error::msg("new_from_i8_slice requires an i8 array type"));
        }
        let total_bytes = (8u64 + u64::from(len) + 7) & !7;
        let ptr = unsafe { gc_bump_alloc(inner, total_bytes) }.map_err(Error::from)?;
        let hdr_u32 = (cid << 3) | GC_HDR_KIND_ARRAY;
        unsafe {
            *(ptr as *mut u32) = hdr_u32;
            *((ptr + 4) as *mut u32) = len;
            if !bytes.is_empty() {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), (ptr + 8) as *mut u8, bytes.len());
            }
        }
        Ok(Rooted::from_abi_word(inner, ptr, false, false))
    }

    /// Allocates an `i16`-packed GC array initialized directly from UTF-16 `units` via bulk `memcpy`.
    pub fn new_from_i16_slice(
        mut store: impl AsContextMut,
        pre: &ArrayRefPre,
        units: &[u16],
    ) -> Result<Rooted<ArrayRef>> {
        let len = u32::try_from(units.len())
            .map_err(|_| Error::from(GcHeapOutOfMemory::new((), units.len() as u64 * 2)))?;
        let inner = store.as_store_inner_mut();
        let cid = pre.ty.canon_id;
        let is_i16 = matches!(
            &inner.ensure_type_info(cid).composite,
            CanonComposite::Array {
                elem: GcStorageKind::I16,
                ..
            }
        );
        if !is_i16 {
            return Err(Error::msg("new_from_i16_slice requires an i16 array type"));
        }
        let data_bytes = u64::from(len) * 2;
        let total_bytes = (8u64 + data_bytes + 7) & !7;
        let ptr = unsafe { gc_bump_alloc(inner, total_bytes) }.map_err(Error::from)?;
        let hdr_u32 = (cid << 3) | GC_HDR_KIND_ARRAY;
        unsafe {
            *(ptr as *mut u32) = hdr_u32;
            *((ptr + 4) as *mut u32) = len;
            if !units.is_empty() {
                std::ptr::copy_nonoverlapping(
                    units.as_ptr().cast::<u8>(),
                    (ptr + 8) as *mut u8,
                    data_bytes as usize,
                );
            }
        }
        Ok(Rooted::from_abi_word(inner, ptr, false, false))
    }
}

/// Pre-validated exception allocator (`ExnRefPre`).
#[derive(Clone, Debug)]
pub struct ExnRefPre {
    pub(crate) ty: ExnType,
}

impl ExnRefPre {
    pub fn new(_store: impl AsContext, ty: ExnType) -> Self {
        Self { ty }
    }
}

impl ExnRef {
    pub fn new(
        mut store: impl AsContextMut,
        pre: &ExnRefPre,
        tag: &crate::instance::Tag,
        fields: &[Val],
    ) -> Result<Rooted<ExnRef>> {
        let inner = store.as_store_inner_mut();
        if tag.store_id != inner.id {
            return Err(Error::msg("ExnRef::new tag belongs to another Store"));
        }
        let param_tys: Vec<ValType> = pre.ty.tag_type().params().collect();
        if fields.len() != param_tys.len() {
            return Err(Error::msg("ExnRef::new field count mismatch"));
        }
        let tag_info = inner
            .tags
            .get(tag.index)
            .ok_or_else(|| Error::msg("invalid tag handle"))?
            .clone();
        let rec_size = tag_info.rec_size.max(8) as usize;
        let layout = std::alloc::Layout::from_size_align(rec_size, 16)
            .map_err(|_| Error::msg("invalid exception layout"))?;
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
        if ptr.is_null() {
            return Err(Error::msg("exception allocation failed"));
        }
        unsafe {
            *(ptr as *mut u32) = tag_info.global_tag_id;
        }
        let mut traced_fields = Vec::new();
        for ((&(ty_code, foff, ref_kind), val), _vt) in tag_info
            .fields
            .iter()
            .zip(fields.iter())
            .zip(param_tys.iter())
        {
            if ref_kind == 1 || ref_kind == 2 {
                traced_fields.push((foff, ref_kind == 2));
            }
            let dst = unsafe { ptr.offset(foff as isize) };
            let raw_word = val.to_abi_word(inner)?;
            unsafe {
                match ty_code {
                    wtime_aot::x64::store_rt::TY_I32 | wtime_aot::x64::store_rt::TY_F32 => {
                        *(dst as *mut u32) = raw_word as u32;
                    }
                    wtime_aot::x64::store_rt::TY_I64 | wtime_aot::x64::store_rt::TY_F64 => {
                        *(dst as *mut u64) = raw_word;
                    }
                    wtime_aot::x64::store_rt::TY_V128 => {
                        let v128 = match val {
                            Val::V128(v) => *v,
                            _ => return Err(Error::msg("expected V128 exception payload")),
                        };
                        *(dst as *mut u128) = v128;
                    }
                    _ => return Err(Error::msg("invalid exception field type code")),
                }
            }
        }
        let obj_idx = inner.gc.exn_objects.len();
        inner.gc.exn_by_ptr.insert(ptr as usize, obj_idx);
        inner.gc.exn_objects.push(StoreExnBox {
            ptr,
            layout,
            global_tag_id: tag_info.global_tag_id,
            traced_fields,
        });
        Ok(Rooted::from_abi_word(inner, ptr as u64, false, true))
    }
}

impl Rooted<ExnRef> {
    pub fn tag(&self, store: impl AsContext) -> Result<crate::instance::Tag> {
        let inner = store.as_store_inner();
        let ptr = self.read_abi_word(inner)? as usize;
        let &obj_idx = inner
            .gc
            .exn_by_ptr
            .get(&ptr)
            .ok_or_else(|| Error::msg("unknown ExnRef pointer"))?;
        let gid = inner.gc.exn_objects[obj_idx].global_tag_id;
        let tag_idx = inner
            .tags
            .iter()
            .position(|t| t.global_tag_id == gid)
            .ok_or_else(|| Error::msg("ExnRef tag not found in Store"))?;
        Ok(crate::instance::Tag {
            store_id: inner.id,
            index: tag_idx,
        })
    }

    pub fn field(&self, mut store: impl AsContextMut, index: usize) -> Result<Val> {
        let inner = store.as_store_inner_mut();
        let ptr = self.read_abi_word(inner)? as *const u8;
        let &obj_idx = inner
            .gc
            .exn_by_ptr
            .get(&(ptr as usize))
            .ok_or_else(|| Error::msg("unknown ExnRef pointer"))?;
        let gid = inner.gc.exn_objects[obj_idx].global_tag_id;
        let tag_info = inner
            .tags
            .iter()
            .find(|t| t.global_tag_id == gid)
            .ok_or_else(|| Error::msg("ExnRef tag not found"))?
            .clone();
        let &(ty_code, foff, ref_kind) = tag_info
            .fields
            .get(index)
            .ok_or_else(|| Error::msg("ExnRef field index out of bounds"))?;
        let src = unsafe { ptr.offset(foff as isize) };
        let vt = tag_info.param_types.get(index).cloned();
        unsafe {
            match (ty_code, ref_kind, vt) {
                (wtime_aot::x64::store_rt::TY_I32, _, _) => Ok(Val::I32(*(src as *const i32))),
                (wtime_aot::x64::store_rt::TY_F32, _, _) => Ok(Val::F32(*(src as *const u32))),
                (wtime_aot::x64::store_rt::TY_F64, _, _) => Ok(Val::F64(*(src as *const u64))),
                (wtime_aot::x64::store_rt::TY_V128, _, _) => Ok(Val::V128(*(src as *const u128))),
                (_, 1, _) => {
                    let w = *(src as *const u64);
                    Ok(Val::AnyRef(if w == 0 {
                        None
                    } else {
                        Some(Rooted::from_abi_word(inner, w, false, false))
                    }))
                }
                (_, 2, _) => {
                    let w = *(src as *const u64);
                    Ok(Val::ExternRef(if w == 0 {
                        None
                    } else {
                        Some(Rooted::from_abi_word(inner, w, true, false))
                    }))
                }
                _ => Ok(Val::I64(*(src as *const i64))),
            }
        }
    }
}

// =========================================================================
// `WeakAnyRef` and `GcEphemeron` + `RawGcEphemerons`
// =========================================================================

/// Weak reference to a GC heap object owned by an `owner` object; clears if either dies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WeakAnyRef {
    store_id: u64,
    slot_idx: u32,
    generation: u32,
}

impl WeakAnyRef {
    pub fn new<O: GcRefType, T: GcRefType>(
        mut store: impl AsContextMut,
        owner: &impl RootedRef<O>,
        target: &impl RootedRef<T>,
    ) -> Result<Self> {
        let inner = store.as_store_inner_mut();
        let (sid_o, idx_o, gen_o) = owner.root_handle();
        let (sid_t, idx_t, gen_t) = target.root_handle();
        if sid_o != inner.id || sid_t != inner.id {
            return Err(Error::msg(
                "WeakAnyRef::new operand belongs to another Store",
            ));
        }
        let owner_w = inner.gc.read_host_root(idx_o, gen_o)?;
        let target_w = inner.gc.read_host_root(idx_t, gen_t)?;
        if !inner.gc.is_valid_active_heap_ptr(owner_w, &inner.state)
            || !inner.gc.is_valid_active_heap_ptr(target_w, &inner.state)
        {
            return Err(Error::msg(
                "WeakAnyRef requires non-null, non-i31 GC heap objects",
            ));
        }
        let (slot_idx, generation) = if let Some(idx) = inner.gc.weak_refs_free.pop() {
            let e = &mut inner.gc.weak_refs[idx as usize];
            e.generation = e.generation.wrapping_add(1).max(1);
            e.owner = owner_w;
            e.target = target_w;
            e.live = true;
            (idx, e.generation)
        } else {
            let idx = inner.gc.weak_refs.len() as u32;
            inner.gc.weak_refs.push(WeakRefEntry {
                owner: owner_w,
                target: target_w,
                generation: 1,
                live: true,
            });
            (idx, 1)
        };
        Ok(Self {
            store_id: inner.id,
            slot_idx,
            generation,
        })
    }

    pub fn upgrade(&self, mut store: impl AsContextMut) -> Result<Option<Rooted<AnyRef>>> {
        let inner = store.as_store_inner_mut();
        if inner.id != self.store_id {
            return Err(Error::msg("WeakAnyRef belongs to another Store"));
        }
        let Some(e) = inner.gc.weak_refs.get(self.slot_idx as usize) else {
            return Ok(None);
        };
        if !e.live || e.generation != self.generation {
            return Ok(None);
        }
        let target = e.target;
        Ok(Some(Rooted::from_abi_word(inner, target, false, false)))
    }

    pub fn remove(&self, mut store: impl AsContextMut) -> Result<()> {
        let inner = store.as_store_inner_mut();
        if inner.id != self.store_id {
            return Err(Error::msg("WeakAnyRef belongs to another Store"));
        }
        if let Some(e) = inner.gc.weak_refs.get_mut(self.slot_idx as usize)
            && e.live
            && e.generation == self.generation
        {
            e.live = false;
            e.owner = 0;
            e.target = 0;
            inner.gc.weak_refs_free.push(self.slot_idx);
        }
        Ok(())
    }
}

/// Outcome of `RawGcEphemerons::set_value_if_matches_raw`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GcEphemeronUpdate {
    Updated,
    Mismatch,
    Retired,
}

/// Collector-managed ephemeron edge `(owner, key) -> Option<value>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GcEphemeron {
    store_id: u64,
    slot_idx: u32,
    generation: u32,
}

/// Borrowed non-collecting view of a Store's ephemeron table (passed to `GcEphemeron::with_raw_access`).
pub struct RawGcEphemerons<'a> {
    store_id: u64,
    gc: &'a mut GcHeap,
    state: &'a StoreState,
}

impl RawGcEphemerons<'_> {
    pub fn is_live(&self, entry: GcEphemeron) -> Result<bool> {
        if entry.store_id != self.store_id {
            return Err(Error::msg("GcEphemeron belongs to another Store"));
        }
        Ok(self
            .gc
            .ephemerons
            .get(entry.slot_idx as usize)
            .is_some_and(|e| e.live && e.generation == entry.generation))
    }

    pub fn value_if_matches_raw(
        &self,
        entry: GcEphemeron,
        owner: u32,
        key: u32,
    ) -> Result<Option<u32>> {
        if entry.store_id != self.store_id {
            return Err(Error::msg("GcEphemeron belongs to another Store"));
        }
        let owner_w = self.gc.raw_u32_to_abi_word(owner);
        let key_w = self.gc.raw_u32_to_abi_word(key);
        let Some(e) = self.gc.ephemerons.get(entry.slot_idx as usize) else {
            return Ok(None);
        };
        if !e.live || e.generation != entry.generation || e.owner != owner_w || e.key != key_w {
            return Ok(None);
        }
        Ok(Some(self.gc.abi_word_to_raw_u32(e.value)))
    }

    /// # Safety
    /// `owner`, `key`, and `value` must be valid raw handles from the current non-collecting Store state.
    pub unsafe fn set_value_if_matches_raw(
        &mut self,
        entry: GcEphemeron,
        owner: u32,
        key: u32,
        value: u32,
    ) -> Result<GcEphemeronUpdate> {
        if entry.store_id != self.store_id {
            return Err(Error::msg("GcEphemeron belongs to another Store"));
        }
        let owner_w = self.gc.raw_u32_to_abi_word(owner);
        let key_w = self.gc.raw_u32_to_abi_word(key);
        let val_w = self.gc.raw_u32_to_abi_word(value);
        let val_ok =
            val_w == 0 || (val_w & 1) != 0 || self.gc.is_valid_active_heap_ptr(val_w, self.state);
        let Some(e) = self.gc.ephemerons.get_mut(entry.slot_idx as usize) else {
            return Ok(GcEphemeronUpdate::Retired);
        };
        if !e.live || e.generation != entry.generation {
            return Ok(GcEphemeronUpdate::Retired);
        }
        if e.owner != owner_w || e.key != key_w {
            return Ok(GcEphemeronUpdate::Mismatch);
        }
        if !val_ok {
            return Err(Error::msg("invalid raw ephemeron value"));
        }
        e.value = val_w;
        Ok(GcEphemeronUpdate::Updated)
    }

    /// # Safety
    /// `owner`, `key`, and `value` must be valid raw handles from the current non-collecting Store state.
    pub unsafe fn insert_raw(&mut self, owner: u32, key: u32, value: u32) -> Result<GcEphemeron> {
        let owner_w = self.gc.raw_u32_to_abi_word(owner);
        let key_w = self.gc.raw_u32_to_abi_word(key);
        let val_w = self.gc.raw_u32_to_abi_word(value);
        if !self.gc.is_valid_active_heap_ptr(owner_w, self.state)
            || !self.gc.is_valid_active_heap_ptr(key_w, self.state)
        {
            return Err(Error::msg(
                "GcEphemeron owner and key must be non-null, non-i31 GC heap objects",
            ));
        }
        if val_w != 0 && (val_w & 1) == 0 && !self.gc.is_valid_active_heap_ptr(val_w, self.state) {
            return Err(Error::msg("invalid raw ephemeron value"));
        }
        let (slot_idx, generation) = if let Some(idx) = self.gc.ephemerons_free.pop() {
            let e = &mut self.gc.ephemerons[idx as usize];
            e.generation = e.generation.wrapping_add(1).max(1);
            e.owner = owner_w;
            e.key = key_w;
            e.value = val_w;
            e.live = true;
            e.activated = false;
            (idx, e.generation)
        } else {
            let idx = self.gc.ephemerons.len() as u32;
            self.gc.ephemerons.push(EphemeronEntry {
                owner: owner_w,
                key: key_w,
                value: val_w,
                generation: 1,
                live: true,
                activated: false,
            });
            (idx, 1)
        };
        Ok(GcEphemeron {
            store_id: self.store_id,
            slot_idx,
            generation,
        })
    }
}

impl GcEphemeron {
    pub fn new<O: GcRefType, K: GcRefType, V: GcRefType>(
        mut store: impl AsContextMut,
        owner: &impl RootedRef<O>,
        key: &impl RootedRef<K>,
        value: Option<&impl RootedRef<V>>,
    ) -> Result<Self> {
        let inner = store.as_store_inner_mut();
        let (sid_o, idx_o, gen_o) = owner.root_handle();
        let (sid_k, idx_k, gen_k) = key.root_handle();
        if sid_o != inner.id || sid_k != inner.id {
            return Err(Error::msg(
                "GcEphemeron::new operand belongs to another Store",
            ));
        }
        let owner_w = inner.gc.read_host_root(idx_o, gen_o)?;
        let key_w = inner.gc.read_host_root(idx_k, gen_k)?;
        let val_w = match value {
            Some(v) => {
                let (sid_v, idx_v, gen_v) = v.root_handle();
                if sid_v != inner.id {
                    return Err(Error::msg(
                        "GcEphemeron::new value belongs to another Store",
                    ));
                }
                inner.gc.read_host_root(idx_v, gen_v)?
            }
            None => 0,
        };
        let owner_raw = inner.gc.abi_word_to_raw_u32(owner_w);
        let key_raw = inner.gc.abi_word_to_raw_u32(key_w);
        let val_raw = inner.gc.abi_word_to_raw_u32(val_w);
        unsafe { Self::new_raw(store, owner_raw, key_raw, val_raw) }
    }

    /// # Safety
    /// Raw handles must belong to this Store's current non-collecting state.
    pub unsafe fn new_raw(
        mut store: impl AsContextMut,
        owner: u32,
        key: u32,
        value: u32,
    ) -> Result<Self> {
        let inner = store.as_store_inner_mut();
        let mut raw = RawGcEphemerons {
            store_id: inner.id,
            gc: &mut inner.gc,
            state: &inner.state,
        };
        unsafe { raw.insert_raw(owner, key, value) }
    }

    /// Registers a WeakMap backing reference array `(array (mut (ref null eq)))` of `(key, value)` pairs.
    ///
    /// # Safety
    /// `array_raw` must be a valid non-null GC array reference in the current Store state.
    pub unsafe fn register_table_raw(mut store: impl AsContextMut, array_raw: u32) -> Result<()> {
        let inner = store.as_store_inner_mut();
        let ptr = inner.gc.raw_u32_to_abi_word(array_raw);
        if !inner.gc.is_valid_active_heap_ptr(ptr, &inner.state) {
            return Err(Error::msg(
                "register_table_raw requires a non-null GC array",
            ));
        }
        let hdr = unsafe { *(ptr as *const u64) };
        if (hdr as u32 & 6) != GC_HDR_KIND_ARRAY {
            return Err(Error::msg("register_table_raw target is not a GC array"));
        }
        inner.gc.weak_tables.insert(ptr);
        Ok(())
    }

    /// Grants simultaneous mutable access to `Store` user data `T` and `RawGcEphemerons` without collecting.
    pub fn with_raw_access<T, R>(
        mut store: impl AsContextMut<Data = T>,
        f: impl FnOnce(&mut T, &mut RawGcEphemerons<'_>) -> Result<R>,
    ) -> Result<R> {
        let mut ctx = store.as_context_mut();
        let (data, inner) = ctx.split_data_and_inner_mut();
        let mut raw = RawGcEphemerons {
            store_id: inner.id,
            gc: &mut inner.gc,
            state: &inner.state,
        };
        f(data, &mut raw)
    }

    pub fn is_live(&self, store: impl AsContext) -> Result<bool> {
        let inner = store.as_store_inner();
        if inner.id != self.store_id {
            return Err(Error::msg("GcEphemeron belongs to another Store"));
        }
        Ok(inner
            .gc
            .ephemerons
            .get(self.slot_idx as usize)
            .is_some_and(|e| e.live && e.generation == self.generation))
    }

    pub fn matches_key<K: GcRefType>(
        &self,
        store: impl AsContext,
        key: &impl RootedRef<K>,
    ) -> Result<bool> {
        let inner = store.as_store_inner();
        if inner.id != self.store_id {
            return Err(Error::msg("GcEphemeron belongs to another Store"));
        }
        let (sid, idx, gen_id) = key.root_handle();
        if sid != inner.id {
            return Err(Error::msg(
                "GcEphemeron::matches_key key belongs to another Store",
            ));
        }
        let key_w = inner.gc.read_host_root(idx, gen_id)?;
        Ok(inner
            .gc
            .ephemerons
            .get(self.slot_idx as usize)
            .is_some_and(|e| e.live && e.generation == self.generation && e.key == key_w))
    }

    pub fn matches_raw(&self, store: impl AsContext, owner: u32, key: u32) -> Result<bool> {
        let inner = store.as_store_inner();
        if inner.id != self.store_id {
            return Err(Error::msg("GcEphemeron belongs to another Store"));
        }
        let owner_w = inner.gc.raw_u32_to_abi_word(owner);
        let key_w = inner.gc.raw_u32_to_abi_word(key);
        Ok(inner
            .gc
            .ephemerons
            .get(self.slot_idx as usize)
            .is_some_and(|e| {
                e.live && e.generation == self.generation && e.owner == owner_w && e.key == key_w
            }))
    }

    pub fn value(&self, mut store: impl AsContextMut) -> Result<Option<Rooted<AnyRef>>> {
        let inner = store.as_store_inner_mut();
        if inner.id != self.store_id {
            return Err(Error::msg("GcEphemeron belongs to another Store"));
        }
        let Some(e) = inner.gc.ephemerons.get(self.slot_idx as usize) else {
            return Ok(None);
        };
        if !e.live || e.generation != self.generation || e.value == 0 {
            return Ok(None);
        }
        let v = e.value;
        Ok(Some(Rooted::from_abi_word(inner, v, false, false)))
    }

    pub fn value_raw(&self, mut store: impl AsContextMut) -> Result<u32> {
        let inner = store.as_store_inner_mut();
        if inner.id != self.store_id {
            return Err(Error::msg("GcEphemeron belongs to another Store"));
        }
        let Some(e) = inner.gc.ephemerons.get(self.slot_idx as usize) else {
            return Ok(0);
        };
        if !e.live || e.generation != self.generation || e.value == 0 {
            return Ok(0);
        }
        Ok(inner.gc.abi_word_to_raw_u32(e.value))
    }

    pub fn set_value<V: GcRefType>(
        &self,
        mut store: impl AsContextMut,
        value: Option<&impl RootedRef<V>>,
    ) -> Result<()> {
        let inner = store.as_store_inner_mut();
        if inner.id != self.store_id {
            return Err(Error::msg("GcEphemeron belongs to another Store"));
        }
        let val_w = match value {
            Some(v) => {
                let (sid, idx, gen_id) = v.root_handle();
                if sid != inner.id {
                    return Err(Error::msg(
                        "GcEphemeron::set_value belongs to another Store",
                    ));
                }
                inner.gc.read_host_root(idx, gen_id)?
            }
            None => 0,
        };
        let Some(e) = inner.gc.ephemerons.get_mut(self.slot_idx as usize) else {
            return Err(Error::msg("GcEphemeron is retired"));
        };
        if !e.live || e.generation != self.generation {
            return Err(Error::msg("GcEphemeron is retired"));
        }
        e.value = val_w;
        Ok(())
    }

    /// # Safety
    /// `value` must be a valid raw GC handle (or `0`) in the current non-collecting Store state.
    pub unsafe fn set_value_raw(&self, mut store: impl AsContextMut, value: u32) -> Result<()> {
        let inner = store.as_store_inner_mut();
        if inner.id != self.store_id {
            return Err(Error::msg("GcEphemeron belongs to another Store"));
        }
        let val_w = inner.gc.raw_u32_to_abi_word(value);
        if val_w != 0 && (val_w & 1) == 0 && !inner.gc.is_valid_active_heap_ptr(val_w, &inner.state)
        {
            return Err(Error::msg("invalid raw ephemeron value"));
        }
        let Some(e) = inner.gc.ephemerons.get_mut(self.slot_idx as usize) else {
            return Err(Error::msg("GcEphemeron is retired"));
        };
        if !e.live || e.generation != self.generation {
            return Err(Error::msg("GcEphemeron is retired"));
        }
        e.value = val_w;
        Ok(())
    }

    pub fn remove(&self, mut store: impl AsContextMut) -> Result<()> {
        let inner = store.as_store_inner_mut();
        if inner.id != self.store_id {
            return Err(Error::msg("GcEphemeron belongs to another Store"));
        }
        if let Some(e) = inner.gc.ephemerons.get_mut(self.slot_idx as usize)
            && e.live
            && e.generation == self.generation
        {
            e.live = false;
            e.owner = 0;
            e.key = 0;
            e.value = 0;
            inner.gc.ephemerons_free.push(self.slot_idx);
        }
        Ok(())
    }
}

// =========================================================================
// Dynamic `Val` Enum + GC Storage Helpers
// =========================================================================

#[derive(Clone, Copy, Debug)]
pub enum Val {
    I32(i32),
    I64(i64),
    F32(u32),
    F64(u64),
    V128(u128),
    FuncRef(Option<crate::instance::Func>),
    ExternRef(Option<Rooted<ExternRef>>),
    AnyRef(Option<Rooted<AnyRef>>),
    ExnRef(Option<Rooted<ExnRef>>),
}

impl From<Rooted<AnyRef>> for Val {
    fn from(r: Rooted<AnyRef>) -> Self {
        Self::AnyRef(Some(r))
    }
}
impl From<Rooted<EqRef>> for Val {
    fn from(r: Rooted<EqRef>) -> Self {
        Self::AnyRef(Some(r.to_anyref()))
    }
}
impl From<Rooted<StructRef>> for Val {
    fn from(r: Rooted<StructRef>) -> Self {
        Self::AnyRef(Some(r.to_anyref()))
    }
}
impl From<Rooted<ArrayRef>> for Val {
    fn from(r: Rooted<ArrayRef>) -> Self {
        Self::AnyRef(Some(r.to_anyref()))
    }
}
impl From<Rooted<ExternRef>> for Val {
    fn from(r: Rooted<ExternRef>) -> Self {
        Self::ExternRef(Some(r))
    }
}
impl From<Rooted<ExnRef>> for Val {
    fn from(r: Rooted<ExnRef>) -> Self {
        Self::ExnRef(Some(r))
    }
}
impl From<crate::instance::Func> for Val {
    fn from(f: crate::instance::Func) -> Self {
        Self::FuncRef(Some(f))
    }
}

impl Val {
    pub fn unwrap_i32(&self) -> i32 {
        match *self {
            Self::I32(v) => v,
            _ => panic!("Val is not I32"),
        }
    }
    pub fn unwrap_i64(&self) -> i64 {
        match *self {
            Self::I64(v) => v,
            _ => panic!("Val is not I64"),
        }
    }
    pub fn unwrap_f32(&self) -> f32 {
        match *self {
            Self::F32(b) => f32::from_bits(b),
            _ => panic!("Val is not F32"),
        }
    }
    pub fn unwrap_f64(&self) -> f64 {
        match *self {
            Self::F64(b) => f64::from_bits(b),
            _ => panic!("Val is not F64"),
        }
    }
    pub fn unwrap_anyref(&self) -> Option<Rooted<AnyRef>> {
        match *self {
            Self::AnyRef(r) => r,
            _ => panic!("Val is not AnyRef"),
        }
    }
    pub fn unwrap_externref(&self) -> Option<Rooted<ExternRef>> {
        match *self {
            Self::ExternRef(r) => r,
            _ => panic!("Val is not ExternRef"),
        }
    }
    pub fn unwrap_funcref(&self) -> Option<crate::instance::Func> {
        match *self {
            Self::FuncRef(f) => f,
            _ => panic!("Val is not FuncRef"),
        }
    }
    pub fn i32(&self) -> Option<i32> {
        match *self {
            Self::I32(v) => Some(v),
            _ => None,
        }
    }
    pub fn i64(&self) -> Option<i64> {
        match *self {
            Self::I64(v) => Some(v),
            _ => None,
        }
    }
    pub fn f32(&self) -> Option<f32> {
        match *self {
            Self::F32(b) => Some(f32::from_bits(b)),
            _ => None,
        }
    }
    pub fn f64(&self) -> Option<f64> {
        match *self {
            Self::F64(b) => Some(f64::from_bits(b)),
            _ => None,
        }
    }
    pub fn anyref(&self) -> Option<Option<Rooted<AnyRef>>> {
        match *self {
            Self::AnyRef(r) => Some(r),
            _ => None,
        }
    }
    pub fn externref(&self) -> Option<Option<Rooted<ExternRef>>> {
        match *self {
            Self::ExternRef(r) => Some(r),
            _ => None,
        }
    }

    pub(crate) fn to_abi_word(self, store: &StoreInner) -> Result<u64> {
        match &self {
            Self::I32(v) => Ok(u64::from(*v as u32)),
            Self::I64(v) => Ok(*v as u64),
            Self::F32(b) => Ok(u64::from(*b)),
            Self::F64(b) => Ok(*b),
            Self::V128(v) => Ok(*v as u64),
            Self::FuncRef(None) => Ok(0),
            Self::FuncRef(Some(f)) => {
                if f.store_id != store.id {
                    return Err(Error::msg("Func belongs to another Store"));
                }
                Ok(f.func_ref_ptr as u64)
            }
            Self::ExternRef(None) => Ok(0),
            Self::ExternRef(Some(r)) => r.read_abi_word(store),
            Self::AnyRef(None) => Ok(0),
            Self::AnyRef(Some(r)) => r.read_abi_word(store),
            Self::ExnRef(None) => Ok(0),
            Self::ExnRef(Some(r)) => r.read_abi_word(store),
        }
    }

    pub(crate) fn from_abi_words(
        store: &mut StoreInner,
        vt: wtime_types::ValType,
        words: &[u64],
    ) -> Result<Self> {
        use wtime_types::{HeapType as WH, ValType as WV};
        match vt {
            WV::I32 => Ok(Self::I32(words[0] as i32)),
            WV::I64 => Ok(Self::I64(words[0] as i64)),
            WV::F32 => Ok(Self::F32(words[0] as u32)),
            WV::F64 => Ok(Self::F64(words[0])),
            WV::V128 => Ok(Self::V128(
                u128::from(words[0]) | (u128::from(words[1]) << 64),
            )),
            WV::Ref(rt) => {
                let w = words[0];
                match rt.heap {
                    WH::Extern | WH::NoExtern => Ok(Self::ExternRef(if w == 0 {
                        None
                    } else {
                        Some(Rooted::from_abi_word(store, w, true, false))
                    })),
                    WH::Func | WH::NoFunc => Ok(Self::FuncRef(if w == 0 {
                        None
                    } else {
                        Some(crate::instance::Func::from_raw_ptr(
                            store.id,
                            w as *const crate::abi::StoreFuncRef,
                        ))
                    })),
                    WH::Exn | WH::NoExn => Ok(Self::ExnRef(if w == 0 {
                        None
                    } else {
                        Some(Rooted::from_abi_word(store, w, false, true))
                    })),
                    WH::Defined(idx) | WH::ExactDefined(idx) => {
                        let is_func = matches!(
                            store.ensure_type_info(idx.0).composite,
                            CanonComposite::Func { .. }
                        );
                        if is_func {
                            Ok(Self::FuncRef(if w == 0 {
                                None
                            } else {
                                Some(crate::instance::Func::from_raw_ptr(
                                    store.id,
                                    w as *const crate::abi::StoreFuncRef,
                                ))
                            }))
                        } else {
                            Ok(Self::AnyRef(if w == 0 {
                                None
                            } else {
                                Some(Rooted::from_abi_word(store, w, false, false))
                            }))
                        }
                    }
                    _ => Ok(Self::AnyRef(if w == 0 {
                        None
                    } else {
                        Some(Rooted::from_abi_word(store, w, false, false))
                    })),
                }
            }
        }
    }
}

unsafe fn read_gc_storage(
    store: &mut StoreInner,
    ptr: *const u8,
    sk: GcStorageKind,
) -> Result<Val> {
    unsafe {
        match sk {
            GcStorageKind::I8 => Ok(Val::I32(i32::from(*ptr))),
            GcStorageKind::I16 => Ok(Val::I32(i32::from(*(ptr as *const u16)))),
            GcStorageKind::I32 => Ok(Val::I32(*(ptr as *const i32))),
            GcStorageKind::I64 => Ok(Val::I64(*(ptr as *const i64))),
            GcStorageKind::F32 => Ok(Val::F32(*(ptr as *const u32))),
            GcStorageKind::F64 => Ok(Val::F64(*(ptr as *const u64))),
            GcStorageKind::V128 => Ok(Val::V128(*(ptr as *const u128))),
            GcStorageKind::GcRef => {
                let w = *(ptr as *const u64);
                Ok(Val::AnyRef(if w == 0 {
                    None
                } else {
                    Some(Rooted::from_abi_word(store, w, false, false))
                }))
            }
            GcStorageKind::ExternRef => {
                let w = *(ptr as *const u64);
                Ok(Val::ExternRef(if w == 0 {
                    None
                } else {
                    Some(Rooted::from_abi_word(store, w, true, false))
                }))
            }
            GcStorageKind::OtherRef => {
                let w = *(ptr as *const u64);
                Ok(Val::FuncRef(if w == 0 {
                    None
                } else {
                    Some(crate::instance::Func::from_raw_ptr(
                        store.id,
                        w as *const crate::abi::StoreFuncRef,
                    ))
                }))
            }
        }
    }
}

unsafe fn write_gc_storage(
    store: &StoreInner,
    ptr: *mut u8,
    sk: GcStorageKind,
    val: &Val,
) -> Result<()> {
    unsafe {
        match sk {
            GcStorageKind::I8 => {
                let Val::I32(v) = *val else {
                    return Err(Error::msg("expected I32 for i8 GC storage"));
                };
                *ptr = v as u8;
            }
            GcStorageKind::I16 => {
                let Val::I32(v) = *val else {
                    return Err(Error::msg("expected I32 for i16 GC storage"));
                };
                *(ptr as *mut u16) = v as u16;
            }
            GcStorageKind::I32 => {
                let Val::I32(v) = *val else {
                    return Err(Error::msg("expected I32 for i32 GC storage"));
                };
                *(ptr as *mut i32) = v;
            }
            GcStorageKind::I64 => {
                let Val::I64(v) = *val else {
                    return Err(Error::msg("expected I64 for i64 GC storage"));
                };
                *(ptr as *mut i64) = v;
            }
            GcStorageKind::F32 => {
                let Val::F32(b) = *val else {
                    return Err(Error::msg("expected F32 for f32 GC storage"));
                };
                *(ptr as *mut u32) = b;
            }
            GcStorageKind::F64 => {
                let Val::F64(b) = *val else {
                    return Err(Error::msg("expected F64 for f64 GC storage"));
                };
                *(ptr as *mut u64) = b;
            }
            GcStorageKind::V128 => {
                let Val::V128(v) = *val else {
                    return Err(Error::msg("expected V128 for v128 GC storage"));
                };
                *(ptr as *mut u128) = v;
            }
            GcStorageKind::GcRef | GcStorageKind::ExternRef | GcStorageKind::OtherRef => {
                *(ptr as *mut u64) = val.to_abi_word(store)?;
            }
        }
    }
    Ok(())
}
