//! `Store<T>`, `StoreContext`, `StoreContextMut`, `AsContext`, `AsContextMut`,
//! `StoreLimits`, `StoreLimitsBuilder`, and C-ABI runtime callbacks invoked by `StoreCdylib`.

use std::fmt;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use wtime_aot::x64::module::GcRefFamily;
use wtime_aot::x64::{INTERNAL_EXN_SIGNAL, TRAP_ALLOC_FAILED};

use crate::abi::{
    HostFuncCtx, StoreFuncRef, StoreMemDesc, StoreState, StoreTableDesc, TRAP_HOST_PANIC,
    TRAP_HOST_STORED_ERROR,
};
use crate::config::{Engine, PoolStoreGuard, RegisteredCodeRange};
use crate::error::{Error, Result};
use crate::gc::{ExnRef, GcHeap, Rooted, StoreExnBox, collect_store_gc};
use crate::memory::{PrivateMemoryData, SharedMemory, SharedMemoryInner};
use crate::module::Module;
use crate::signal::InterruptHandle;
use crate::types::{CanonTypeInfo, FuncType, GlobalType, TableType, TagType, ValType};

static NEXT_STORE_ID: AtomicU64 = AtomicU64::new(1);

/// Resource limits for a `Store` (`StoreLimits` / `StoreLimitsBuilder`).
#[derive(Clone, Debug)]
pub struct StoreLimits {
    pub(crate) memory_size: usize,
    pub(crate) table_elements: usize,
    pub(crate) instances: usize,
    pub(crate) tables: usize,
    pub(crate) memories: usize,
}

impl Default for StoreLimits {
    fn default() -> Self {
        Self {
            memory_size: usize::MAX,
            table_elements: 10_000_000,
            instances: 10_000,
            tables: 10_000,
            memories: 10_000,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct StoreLimitsBuilder {
    limits: StoreLimits,
}

impl StoreLimitsBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn memory_size(mut self, bytes: usize) -> Self {
        self.limits.memory_size = bytes;
        self
    }

    pub fn table_elements(mut self, count: usize) -> Self {
        self.limits.table_elements = count;
        self
    }

    pub fn instances(mut self, count: usize) -> Self {
        self.limits.instances = count;
        self
    }

    pub fn tables(mut self, count: usize) -> Self {
        self.limits.tables = count;
        self
    }

    pub fn memories(mut self, count: usize) -> Self {
        self.limits.memories = count;
        self
    }

    pub fn trap_on_grow_failure(self, _trap: bool) -> Self {
        self
    }

    pub fn build(self) -> StoreLimits {
        self.limits
    }
}

/// Internal sentinel error carrying a host-thrown Wasm exception (`store.throw(exn)`).
#[derive(Debug)]
pub(crate) struct ThrownExnSignal {
    pub(crate) exn_ptr: u64,
}

impl fmt::Display for ThrownExnSignal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "uncaught wasm exception (0x{:x})", self.exn_ptr)
    }
}

impl std::error::Error for ThrownExnSignal {}

/// Type-erased host callback stored inside `StoreInner::host_funcs`.
#[doc(hidden)]
pub type ErasedHostCallback =
    Arc<dyn Fn(*mut StoreInner, *mut (), *const u64, *mut u64) -> Result<()> + Send + Sync>;

pub(crate) struct HostFuncEntry {
    pub(crate) ty: FuncType,
    pub(crate) cb: ErasedHostCallback,
    pub(crate) _host_ctx: Box<HostFuncCtx>,
    pub(crate) func_ref: Box<StoreFuncRef>,
}

pub(crate) struct StandaloneGlobalData {
    pub(crate) _ty: GlobalType,
    pub(crate) cell: Box<[u64; 2]>,
    pub(crate) is_traced: bool,
}

pub(crate) struct DynamicTableData {
    pub(crate) ty: TableType,
    pub(crate) desc: Box<StoreTableDesc>,
    pub(crate) storage: Vec<u64>,
    pub(crate) is_traced: bool,
    pub(crate) _ref_family: GcRefFamily,
}

impl DynamicTableData {
    pub(crate) fn new(ty: TableType, init_word: u64, ref_family: GcRefFamily) -> Result<Self> {
        let min =
            u32::try_from(ty.minimum()).map_err(|_| Error::msg("table minimum exceeds u32"))?;
        let max = ty
            .maximum()
            .map(|m| u32::try_from(m).unwrap_or(u32::MAX))
            .unwrap_or(u32::MAX);
        let mut storage = vec![init_word; min as usize];
        let ptr = if storage.is_empty() {
            std::ptr::null_mut()
        } else {
            storage.as_mut_ptr()
        };
        let cap = storage.capacity() as u64;
        let desc = Box::new(StoreTableDesc {
            ptr,
            cur_len: min,
            max_len: max,
            cap_slots: cap,
        });
        Ok(Self {
            ty,
            desc,
            storage,
            is_traced: ref_family.is_traced(),
            _ref_family: ref_family,
        })
    }

    pub(crate) fn grow(&mut self, init_val: u64, delta: u64, max_elements: usize) -> Option<u64> {
        let cur = u64::from(self.desc.cur_len);
        if delta == 0 {
            return Some(cur);
        }
        let new_len = cur.checked_add(delta)?;
        if new_len > u64::from(self.desc.max_len) || (new_len as usize) > max_elements {
            return None;
        }
        self.storage.try_reserve(delta as usize).ok()?;
        self.storage.resize(new_len as usize, init_val);
        self.desc.ptr = self.storage.as_mut_ptr();
        self.desc.cur_len = new_len as u32;
        self.desc.cap_slots = self.storage.capacity() as u64;
        Some(cur)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TagInfo {
    pub(crate) global_tag_id: u32,
    pub(crate) ty: TagType,
    pub(crate) rec_size: u32,
    /// `(ty_code, field_offset, ref_kind_u8)` matching `StoreModuleMeta::all_tag_layouts`.
    pub(crate) fields: Vec<(u8, i32, u8)>,
    pub(crate) param_types: Vec<ValType>,
}

/// Live instantiated module state owned by `StoreInner::instances`.
pub(crate) struct InstanceData {
    pub(crate) module: Module,
    /// Pinned 16-byte-aligned instance `Ctx` buffer (`R15` points here).
    pub(crate) ctx_buf: Box<[u8]>,
    /// Optional page-aligned passive GC element buffer (`OFF_STORE_ELEM_BUF_PTR`).
    pub(crate) elem_buf: *mut u8,
    pub(crate) elem_buf_bytes: usize,
    /// Owned `StoreFuncRef` structs for this instance's defined functions (`0..n_defs`).
    pub(crate) _def_func_refs: Box<[StoreFuncRef]>,
    /// Pointer table `[0..n_imports + n_defs]` of `*const StoreFuncRef` (`OFF_STORE_FUNC_REFS`).
    pub(crate) func_ref_ptrs: Box<[*const StoreFuncRef]>,
    /// Pointer table `[0..total_mems]` of `*mut StoreMemDesc` (`OFF_STORE_MEM_DESCS`).
    pub(crate) _mem_desc_ptrs: Box<[*mut StoreMemDesc]>,
    /// Pointer table `[0..total_tables]` of `*mut StoreTableDesc` (`OFF_STORE_TABLE_DESCS`).
    pub(crate) _table_desc_ptrs: Box<[*mut StoreTableDesc]>,
    /// Table `[0..total_tags]` of global tag IDs (`OFF_STORE_TAG_IDS`).
    pub(crate) _tag_ids: Box<[u32]>,
    /// Store-local indices for every memory `0..total_mems` (for export extraction).
    pub(crate) mem_handles: Vec<MemRef>,
    /// Store-local indices for every table `0..total_tables` (for export extraction).
    pub(crate) table_indices: Vec<usize>,
    /// Store-local indices for every tag `0..total_tags` (for export extraction).
    pub(crate) tag_indices: Vec<usize>,
    /// `Ctx` byte offsets of this instance's defined traced globals (for Cheney GC root walk).
    pub(crate) traced_defined_global_offs: Vec<i32>,
    /// Global handles for every global `0..total_globals` (imported pointer OR own `ctx_buf + off`).
    pub(crate) global_ptrs: Vec<(*mut u8, GlobalType)>,
}

impl Drop for InstanceData {
    fn drop(&mut self) {
        if !self.elem_buf.is_null() && self.elem_buf_bytes > 0 {
            unsafe {
                let _ = libc::munmap(self.elem_buf.cast(), self.elem_buf_bytes);
            }
        }
    }
}

#[derive(Clone)]
pub(crate) enum MemRef {
    Private(usize),
    Shared(SharedMemory),
}

/// Core non-generic Store runtime state.
///
/// `#[repr(C)]` with `state: StoreState` at offset `0` so C callbacks receiving `*mut StoreState`
/// cast directly to `*mut StoreInner` with zero pointer chasing!
#[doc(hidden)]
#[repr(C)]
pub struct StoreInner {
    pub(crate) state: StoreState,
    pub(crate) id: u64,
    pub(crate) engine: Engine,
    pub(crate) user_data_ptr: *mut (),
    pub(crate) interrupt: InterruptHandle,
    pub(crate) gc: GcHeap,
    pub(crate) memory_limit_bytes: usize,
    pub(crate) table_elements_limit: usize,
    pub(crate) instances: Vec<InstanceData>,
    pub(crate) memories: Vec<PrivateMemoryData>,
    pub(crate) shared_memories: Vec<Arc<SharedMemoryInner>>,
    pub(crate) tables: Vec<DynamicTableData>,
    pub(crate) tags: Vec<TagInfo>,
    pub(crate) next_global_tag_id: u32,
    pub(crate) standalone_globals: Vec<StandaloneGlobalData>,
    /// Store-wide table of `(cell_ptr, GlobalType)` backing `Copy` [`crate::instance::Global`] handles.
    pub(crate) global_handles: Vec<(*mut u8, GlobalType)>,
    pub(crate) host_funcs: Vec<HostFuncEntry>,
    pub(crate) dropped_owned_roots: Arc<Mutex<Vec<(u32, u32)>>>,
    pub(crate) pending_host_error: Option<Error>,
    pub(crate) pending_host_panic: Option<Box<dyn std::any::Any + Send>>,
    pub(crate) pending_exn_ptr: Option<u64>,
    pub(crate) fuel: Option<u64>,
    pub(crate) active_entry_instance: Option<usize>,
    pub(crate) active_caller_instance: Option<usize>,
    pub(crate) types_snapshot: Arc<[CanonTypeInfo]>,
    pub(crate) synced_types_version: u64,
    pub(crate) code_ranges_snapshot: Arc<[RegisteredCodeRange]>,
    pub(crate) synced_code_ranges_version: u64,
    pub(crate) _pool_guard: Option<PoolStoreGuard>,
}

// SAFETY: `StoreInner` is owned exclusively by `Store<T>` (moved across threads only when `T: Send`).
unsafe impl Send for StoreInner {}

impl StoreInner {
    pub(crate) fn new(engine: &Engine, user_data_ptr: *mut ()) -> Result<Box<Self>> {
        let cfg = engine.config();
        let interrupt = InterruptHandle::new();
        let heap_cap =
            cfg.gc_heap_reservation
                .min((crate::gc::SEMI_SPACE_RESERVE_BYTES - 4096) as u64) as usize;
        let init_gc = cfg.gc_heap_initial_size.min(heap_cap as u64) as usize;
        let gc = GcHeap::new(init_gc, heap_cap, interrupt.clone())?;

        let (descs_ptr, descs_len, types_snapshot, synced_types_version) = {
            let reg = engine.inner.types.read().unwrap_or_else(|e| e.into_inner());
            let snap = Arc::clone(
                &*engine
                    .inner
                    .types_snapshot
                    .read()
                    .unwrap_or_else(|e| e.into_inner()),
            );
            let ver = engine.inner.types_version.load(Ordering::Acquire);
            (reg.descs_ptr(), reg.len() as u64, snap, ver)
        };
        let (code_ranges_snapshot, synced_code_ranges_version) = {
            let snap = Arc::clone(
                &*engine
                    .inner
                    .code_ranges_snapshot
                    .read()
                    .unwrap_or_else(|e| e.into_inner()),
            );
            let ver = engine.inner.code_ranges_version.load(Ordering::Acquire);
            (snap, ver)
        };

        let nursery_base = gc.nursery_base();
        let nursery_cap = gc.nursery_cap as u64;

        let state = StoreState {
            gc_alloc_ptr: nursery_base,
            gc_alloc_limit: nursery_base + nursery_cap,
            gc_from_base: gc.arena_base as u64,
            gc_from_cap: crate::gc::GC_ADDR_RANGE_BYTES as u64,
            gc_to_base: gc.card_table_base(),
            gc_to_cap: nursery_base,
            gc_heap_cap: gc.heap_cap as u64,
            gc_count: 0,
            gc_walk_stop_rsp: 0,
            gc_stack_chain: 0,
            gc_init_roots_ptr: 0,
            gc_init_roots_len: 0,
            gc_elem_buf_ptr: 0,
            gc_elem_buf_bytes: 0,
            type_descs_ptr: descs_ptr,
            type_descs_len: descs_len,
            exn_active_ptr: 0,
            gc_collect_fn: store_gc_collect_cb,
            mem_grow_fn: store_mem_grow_cb,
            atomic_wait_fn: store_atomic_wait_cb,
            atomic_notify_fn: store_atomic_notify_cb,
            exn_alloc_fn: store_exn_alloc_cb,
            table_grow_fn: store_table_grow_cb,
            host_call_fn: store_host_call_cb,
            ret_spill_ptr: std::ptr::null_mut(),
            last_trap_code: 0,
            _pad: 0,
        };

        Ok(Box::new(Self {
            state,
            id: NEXT_STORE_ID.fetch_add(1, Ordering::Relaxed),
            engine: engine.clone(),
            user_data_ptr,
            interrupt,
            gc,
            memory_limit_bytes: usize::MAX,
            table_elements_limit: 10_000_000,
            instances: Vec::new(),
            memories: Vec::new(),
            shared_memories: Vec::new(),
            tables: Vec::new(),
            tags: Vec::new(),
            next_global_tag_id: 1,
            standalone_globals: Vec::new(),
            global_handles: Vec::new(),
            host_funcs: Vec::new(),
            dropped_owned_roots: Arc::new(Mutex::new(Vec::new())),
            pending_host_error: None,
            pending_host_panic: None,
            pending_exn_ptr: None,
            fuel: if cfg.consume_fuel { Some(0) } else { None },
            active_entry_instance: None,
            active_caller_instance: None,
            types_snapshot,
            synced_types_version,
            code_ranges_snapshot,
            synced_code_ranges_version,
            _pool_guard: None,
        }))
    }

    pub(crate) fn consume_entry_fuel(&mut self) -> Result<()> {
        if !self.engine.config().consume_fuel {
            return Ok(());
        }
        let fuel = self.fuel.get_or_insert(0);
        if *fuel == 0 {
            return Err(crate::error::Trap::OutOfFuel.into());
        }
        *fuel -= 1;
        Ok(())
    }

    pub(crate) fn sync_type_descs(&mut self) {
        let cur_ver = self.engine.inner.types_version.load(Ordering::Acquire);
        if cur_ver == self.synced_types_version {
            return;
        }
        let reg = self
            .engine
            .inner
            .types
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let snap = Arc::clone(
            &*self
                .engine
                .inner
                .types_snapshot
                .read()
                .unwrap_or_else(|e| e.into_inner()),
        );
        let final_ver = self.engine.inner.types_version.load(Ordering::Acquire);
        self.state.type_descs_ptr = reg.descs_ptr();
        self.state.type_descs_len = reg.len() as u64;
        self.types_snapshot = snap;
        self.synced_types_version = final_ver;
    }

    #[inline]
    pub(crate) fn ensure_type_info(&mut self, cid: u32) -> &CanonTypeInfo {
        if (cid as usize) >= self.types_snapshot.len() {
            self.sync_type_descs();
        }
        &self.types_snapshot[cid as usize]
    }

    #[inline]
    pub(crate) fn sync_code_ranges(&mut self) -> Arc<[RegisteredCodeRange]> {
        let cur_ver = self
            .engine
            .inner
            .code_ranges_version
            .load(Ordering::Acquire);
        if cur_ver != self.synced_code_ranges_version {
            let snap = Arc::clone(
                &*self
                    .engine
                    .inner
                    .code_ranges_snapshot
                    .read()
                    .unwrap_or_else(|e| e.into_inner()),
            );
            let final_ver = self
                .engine
                .inner
                .code_ranges_version
                .load(Ordering::Acquire);
            self.code_ranges_snapshot = snap;
            self.synced_code_ranges_version = final_ver;
        }
        Arc::clone(&self.code_ranges_snapshot)
    }

    pub(crate) fn drain_dropped_owned_roots(&mut self) {
        let mut q = self
            .dropped_owned_roots
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for (idx, gen_id) in q.drain(..) {
            self.gc.free_host_root(idx, gen_id);
        }
    }

    /// Stamps `OFF_STACK_FLOOR` on every instantiated module `Ctx` for the current OS thread/fiber.
    pub(crate) fn stamp_stack_floors(&mut self) {
        let floor = current_thread_stack_floor(self.engine.config().max_wasm_stack);
        for inst in &mut self.instances {
            let ptr = unsafe {
                inst.ctx_buf
                    .as_mut_ptr()
                    .offset(wtime_aot::x64::OFF_STACK_FLOOR as isize)
                    .cast::<u64>()
            };
            unsafe { *ptr = floor };
        }
    }

    /// Applies `StoreLimits` (clamping private memory cap, table element cap, and GC heap cap).
    pub(crate) fn apply_limits(&mut self, limits: &StoreLimits) {
        self.memory_limit_bytes = limits.memory_size;
        self.table_elements_limit = limits.table_elements;
        let cap = limits
            .memory_size
            .clamp(64 * 1024, crate::gc::SEMI_SPACE_RESERVE_BYTES - 4096);
        let cap_aligned = ((cap + 4095) & !4095).min(crate::gc::SEMI_SPACE_RESERVE_BYTES - 4096);
        self.gc.heap_cap = cap_aligned;
        self.gc.init_cap = self.gc.init_cap.min(cap_aligned);
        self.gc.nursery_init_cap = self.gc.nursery_init_cap.min(cap_aligned);
        self.state.gc_heap_cap = cap_aligned as u64;
        if self.gc.old_cap > cap_aligned {
            let old_used = (self.gc.old_alloc_ptr - self.gc.active_old_base()) as usize;
            if old_used <= cap_aligned {
                self.gc.old_cap = cap_aligned;
            }
        }
        if self.gc.nursery_cap > cap_aligned {
            let nursery_base = self.gc.nursery_base();
            let nursery_used = (self.state.gc_alloc_ptr - nursery_base) as usize;
            if nursery_used <= cap_aligned {
                self.gc.nursery_cap = cap_aligned;
                self.state.gc_alloc_limit = nursery_base + cap_aligned as u64;
            }
        }
    }

    /// Allocates a new unique `Tag` in this Store.
    pub(crate) fn alloc_tag(&mut self, ty: TagType) -> TagInfo {
        let gid = self.next_global_tag_id;
        self.next_global_tag_id += 1;
        let (wparams, _) = ty.ty().wasm_params_results();
        let param_types: Vec<ValType> = ty.params().collect();
        let mut off = 8i32; // +0: u32 global_tag_id, +4: pad
        let mut fields = Vec::with_capacity(wparams.len());
        let reg = self
            .engine
            .inner
            .types
            .read()
            .unwrap_or_else(|e| e.into_inner());
        for &vt in &wparams {
            let mty = wtime_aot::x64::store_rt::val_type_to_machine_ty(vt);
            let ty_code = wtime_aot::x64::store_rt::ty_to_code(mty);
            let (sz, al) = match mty {
                wtime_aot::x64::ir::Ty::I32 | wtime_aot::x64::ir::Ty::F32 => (4i32, 4i32),
                wtime_aot::x64::ir::Ty::I64 | wtime_aot::x64::ir::Ty::F64 => (8i32, 8i32),
                wtime_aot::x64::ir::Ty::V128 => (16i32, 16i32),
            };
            off = (off + al - 1) & !(al - 1);
            let ref_kind = match vt {
                wtime_types::ValType::Ref(rt) => match rt.heap {
                    wtime_types::HeapType::Any
                    | wtime_types::HeapType::Eq
                    | wtime_types::HeapType::I31
                    | wtime_types::HeapType::Struct
                    | wtime_types::HeapType::Array
                    | wtime_types::HeapType::None => 1u8,
                    wtime_types::HeapType::Extern | wtime_types::HeapType::NoExtern => 2u8,
                    wtime_types::HeapType::Defined(idx)
                    | wtime_types::HeapType::ExactDefined(idx) => {
                        if matches!(
                            reg.infos[idx.0 as usize].composite,
                            crate::types::CanonComposite::Func { .. }
                        ) {
                            0u8
                        } else {
                            1u8
                        }
                    }
                    _ => 0u8,
                },
                _ => 0u8,
            };
            fields.push((ty_code, off, ref_kind));
            off += sz;
        }
        drop(reg);
        let rec_size = ((off + 15) & !15).max(16) as u32;
        let info = TagInfo {
            global_tag_id: gid,
            ty,
            rec_size,
            fields,
            param_types,
        };
        self.tags.push(info.clone());
        info
    }

    pub(crate) fn find_shared_memory_by_host_ptr(
        &self,
        host_ptr: *const u8,
    ) -> Option<Arc<SharedMemoryInner>> {
        let addr = host_ptr as usize;
        for sm in &self.shared_memories {
            let base = sm.desc.base as usize;
            let len = sm.desc.len_bytes.load(Ordering::Acquire) as usize;
            if addr >= base && addr < base.saturating_add(len) {
                return Some(Arc::clone(sm));
            }
        }
        None
    }

    pub(crate) fn find_host_tramp_for_func_type(&self, ft: &FuncType) -> Option<*const u8> {
        for inst in &self.instances {
            if let Some(p) = inst.module.host_tramp_for_func_type(ft) {
                return Some(p);
            }
        }
        None
    }

    pub(crate) fn find_export_tramp_for_func_type(
        &self,
        ft: &FuncType,
    ) -> Option<crate::abi::WtStoreExportTrampFn> {
        for inst in &self.instances {
            if let Some(p) = inst.module.export_tramp_for_func_type(ft) {
                return Some(p);
            }
        }
        None
    }

    /// Patches any pending `HostFuncEntry` whose `code_ptr` was null when created before `module` loaded.
    pub(crate) fn patch_pending_host_tramps_from(&mut self, module: &Module) {
        for hf in &mut self.host_funcs {
            if hf.func_ref.code_ptr.is_null()
                && let Some(p) = module.host_tramp_for_func_type(&hf.ty)
            {
                hf.func_ref.code_ptr = p;
            }
        }
    }
}

/// Computes a safe stack-overflow floor for the current thread (handles both pthreads and custom fibers).
fn current_thread_stack_floor(max_wasm_stack: usize) -> u64 {
    let sp_approx = (&max_wasm_stack as *const usize) as usize;
    let max_span = max_wasm_stack.clamp(64 * 1024, 32 << 20);
    #[cfg(target_os = "linux")]
    {
        thread_local! {
            static OS_STACK_BOUNDS: std::cell::Cell<Option<(usize, usize)>> = const { std::cell::Cell::new(None) };
        }
        let bounds = OS_STACK_BOUNDS.with(|cell| {
            if let Some(b) = cell.get() {
                return Some(b);
            }
            let queried = unsafe {
                let mut attr: libc::pthread_attr_t = std::mem::zeroed();
                if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) == 0 {
                    let mut stack_addr: *mut libc::c_void = std::ptr::null_mut();
                    let mut stack_size: usize = 0;
                    let rc = libc::pthread_attr_getstack(&attr, &mut stack_addr, &mut stack_size);
                    libc::pthread_attr_destroy(&mut attr);
                    if rc == 0 && !stack_addr.is_null() && stack_size > 256 * 1024 {
                        let lo = stack_addr as usize;
                        Some((lo, lo.saturating_add(stack_size)))
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            cell.set(queried);
            queried
        });
        if let Some((lo, hi)) = bounds
            && sp_approx >= lo
            && sp_approx <= hi
        {
            // Running on the OS thread stack: keep 128 KiB headroom above the guard page.
            let os_floor = lo.saturating_add(128 * 1024);
            let cfg_floor = sp_approx.saturating_sub(max_span);
            return os_floor.max(cfg_floor) as u64;
        }
    }
    // Running on a custom fiber stack (e.g. Scriptw's 64 MiB fiber): bound below current RSP.
    sp_approx.saturating_sub(max_span) as u64
}

// =========================================================================
// C-ABI Callbacks Invoked Directly by Compiled `StoreCdylib` Machine Code
// =========================================================================

extern "C" fn store_gc_collect_cb(
    store_ptr: *mut StoreState,
    req_bytes: u64,
    gc_call_ret_rsp: u64,
) -> u32 {
    let store = unsafe { &mut *(store_ptr as *mut StoreInner) };
    match unsafe { collect_store_gc(store, req_bytes, gc_call_ret_rsp, false) } {
        Ok(()) => 0,
        Err(_) => u32::from(TRAP_ALLOC_FAILED),
    }
}

extern "C" fn store_mem_grow_cb(
    store_ptr: *mut StoreState,
    mem_desc: *mut StoreMemDesc,
    delta_pages: u64,
) -> u64 {
    let store = unsafe { &mut *(store_ptr as *mut StoreInner) };
    let flags = unsafe { (*mem_desc).flags };
    if (flags & 1) != 0 {
        // Shared memory: locate matching `Arc<SharedMemoryInner>`.
        let target = mem_desc as usize;
        for sm in &store.shared_memories {
            if (sm.desc.as_ref() as *const StoreMemDesc as usize) == target {
                return sm.grow(delta_pages).unwrap_or(u64::MAX);
            }
        }
        return u64::MAX;
    }
    // Private linear memory: grow with `memory_limit_bytes`.
    let limit = store.memory_limit_bytes;
    let target = mem_desc as usize;
    for mem in &mut store.memories {
        if (mem.desc.as_ref() as *const StoreMemDesc as usize) == target {
            return mem.grow(delta_pages, limit).unwrap_or(u64::MAX);
        }
    }
    u64::MAX
}

extern "C" fn store_atomic_wait_cb(
    store_ptr: *mut StoreState,
    host_ptr: *const u8,
    expected: u64,
    timeout_ns: i64,
    is_64: u32,
) -> i32 {
    let store = unsafe { &mut *(store_ptr as *mut StoreInner) };
    let Some(sm) = store.find_shared_memory_by_host_ptr(host_ptr) else {
        return -1;
    };
    sm.atomic_wait(host_ptr, expected, timeout_ns, is_64 != 0)
}

extern "C" fn store_atomic_notify_cb(
    store_ptr: *mut StoreState,
    host_ptr: *const u8,
    count: u32,
) -> u32 {
    let store = unsafe { &mut *(store_ptr as *mut StoreInner) };
    let Some(sm) = store.find_shared_memory_by_host_ptr(host_ptr) else {
        return 0;
    };
    sm.atomic_notify(host_ptr, count)
}

extern "C" fn store_exn_alloc_cb(
    store_ptr: *mut StoreState,
    global_tag_id: u32,
    size_bytes: u32,
) -> *mut u8 {
    let store = unsafe { &mut *(store_ptr as *mut StoreInner) };
    let sz = (size_bytes as usize).max(16);
    let Ok(layout) = std::alloc::Layout::from_size_align(sz, 16) else {
        return std::ptr::null_mut();
    };
    let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
    if ptr.is_null() {
        return std::ptr::null_mut();
    }
    let traced_fields = store
        .tags
        .iter()
        .find(|t| t.global_tag_id == global_tag_id)
        .map(|t| {
            t.fields
                .iter()
                .filter(|&&(_, _, rk)| rk == 1 || rk == 2)
                .map(|&(_, foff, rk)| (foff, rk == 2))
                .collect()
        })
        .unwrap_or_default();
    let idx = store.gc.exn_objects.len();
    store.gc.exn_by_ptr.insert(ptr as usize, idx);
    store.gc.exn_objects.push(StoreExnBox {
        ptr,
        layout,
        global_tag_id,
        traced_fields,
    });
    ptr
}

extern "C" fn store_table_grow_cb(
    store_ptr: *mut StoreState,
    table_desc: *mut StoreTableDesc,
    init_val: u64,
    delta: u64,
) -> u64 {
    let store = unsafe { &mut *(store_ptr as *mut StoreInner) };
    let limit = store.table_elements_limit;
    let target = table_desc as usize;
    for tbl in &mut store.tables {
        if (tbl.desc.as_ref() as *const StoreTableDesc as usize) == target {
            return tbl.grow(init_val, delta, limit).unwrap_or(u64::MAX);
        }
    }
    u64::MAX
}

extern "C" fn store_host_call_cb(
    store_ptr: *mut StoreState,
    host_func_id: u32,
    args_ptr: *const u64,
    rets_ptr: *mut u64,
) -> u32 {
    let store = unsafe { &mut *(store_ptr as *mut StoreInner) };
    let Some(entry) = store.host_funcs.get(host_func_id as usize) else {
        store.pending_host_error = Some(Error::msg(format!(
            "invalid host function id {host_func_id}"
        )));
        store.state.last_trap_code = TRAP_HOST_STORED_ERROR;
        return TRAP_HOST_STORED_ERROR;
    };
    let cb = Arc::clone(&entry.cb);
    let user_data = store.user_data_ptr;

    // Identify the calling Wasm instance from the saved return PC in `GcStackChainNode`.
    let caller_inst = resolve_caller_instance(store);
    let prev_caller = store.active_caller_instance;
    store.active_caller_instance = caller_inst;

    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cb(store as *mut StoreInner, user_data, args_ptr, rets_ptr)
    }));

    let store = unsafe { &mut *(store_ptr as *mut StoreInner) };
    store.active_caller_instance = prev_caller;

    match res {
        Ok(Ok(())) => 0,
        Ok(Err(err)) => {
            if let Some(sig) = err.downcast_ref::<ThrownExnSignal>() {
                store.state.exn_active_ptr = sig.exn_ptr;
                u32::from(INTERNAL_EXN_SIGNAL)
            } else {
                store.pending_host_error = Some(err);
                store.state.last_trap_code = TRAP_HOST_STORED_ERROR;
                TRAP_HOST_STORED_ERROR
            }
        }
        Err(payload) => {
            store.pending_host_panic = Some(payload);
            store.state.last_trap_code = TRAP_HOST_PANIC;
            TRAP_HOST_PANIC
        }
    }
}

fn resolve_caller_instance(store: &StoreInner) -> Option<usize> {
    let chain_ptr = store.state.gc_stack_chain;
    if chain_ptr != 0 {
        let node = unsafe { *(chain_ptr as *const crate::abi::GcStackChainNode) };
        if node.outer_paused_ret_rsp != 0 {
            let ret_pc = unsafe { *(node.outer_paused_ret_rsp as *const usize) };
            if let Some(entry_idx) = store.active_entry_instance
                && let Some(inst) = store.instances.get(entry_idx)
            {
                let d = &inst.module.inner.so.desc;
                if ret_pc >= d.code_start && ret_pc < d.code_end {
                    return Some(entry_idx);
                }
            }
            for (idx, inst) in store.instances.iter().enumerate().rev() {
                let d = &inst.module.inner.so.desc;
                if ret_pc >= d.code_start && ret_pc < d.code_end {
                    return Some(idx);
                }
            }
        }
    }
    store.active_entry_instance
}

// =========================================================================
// Public `Store<T>`, `StoreContext`, `StoreContextMut`, `AsContext`, `AsContextMut`
// =========================================================================

pub struct Store<T> {
    pub(crate) data: Box<T>,
    pub(crate) inner: Box<StoreInner>,
}

impl<T> Store<T> {
    pub fn new(engine: &Engine, data: T) -> Self {
        let mut boxed_data = Box::new(data);
        let data_ptr = (boxed_data.as_mut() as *mut T).cast::<()>();
        let inner = StoreInner::new(engine, data_ptr).expect("Store::new allocation failed");
        Self {
            data: boxed_data,
            inner,
        }
    }

    pub fn try_new(engine: &Engine, data: T) -> Result<Self> {
        let mut boxed_data = Box::new(data);
        let data_ptr = (boxed_data.as_mut() as *mut T).cast::<()>();
        let inner = StoreInner::new(engine, data_ptr)?;
        Ok(Self {
            data: boxed_data,
            inner,
        })
    }

    pub fn engine(&self) -> &Engine {
        &self.inner.engine
    }

    pub fn data(&self) -> &T {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut T {
        &mut self.data
    }

    pub fn into_data(self) -> T {
        *self.data
    }

    pub fn gc_count(&self) -> u64 {
        self.inner.state.gc_count
    }

    pub fn gc_minor_count(&self) -> u64 {
        self.inner.gc.minor_gc_count
    }

    pub fn gc_major_count(&self) -> u64 {
        self.inner.gc.major_gc_count
    }

    pub fn gc_weak_retire_count(&self) -> u64 {
        self.inner.gc.weak_retire_count
    }

    pub fn limiter(&mut self, mut limiter: impl FnMut(&mut T) -> &mut StoreLimits) {
        let limits = limiter(&mut self.data).clone();
        self.inner.apply_limits(&limits);
    }

    pub fn interrupt_handle(&self) -> InterruptHandle {
        self.inner.interrupt.clone()
    }

    /// Registers a private linear memory with this Store's fault-based `InterruptHandle`.
    pub fn interrupt_on_memory(&mut self, memory: &crate::instance::Memory) -> Result<()> {
        if memory.store_id != self.inner.id {
            return Err(Error::msg("Memory belongs to another Store"));
        }
        let mem = &self.inner.memories[memory.index];
        let base = mem.desc.base as usize;
        let len = mem.desc.len_bytes.load(Ordering::Acquire) as usize;
        self.inner.interrupt.set_region(base, len);
        Ok(())
    }

    pub fn gc(&mut self, _why: Option<&str>) -> Result<()> {
        unsafe { collect_store_gc(&mut self.inner, 0, 0, true).map_err(Error::from) }
    }

    /// Returns physical pages of the warm inactive GC semi-space to the OS without running a collection.
    /// Useful when an isolate transitions to idle so active throughput keeps zero-syscall warm flips
    /// while idle isolates shed inactive semi-space RSS.
    pub fn reclaim_inactive_gc_space(&mut self) {
        let nursery_empty = self.inner.state.gc_alloc_ptr == self.inner.gc.nursery_base();
        self.inner.gc.reclaim_inactive_space(nursery_empty);
        if nursery_empty {
            self.inner.state.gc_alloc_limit =
                self.inner.gc.nursery_base() + self.inner.gc.nursery_cap as u64;
        }
    }

    pub fn gc_heap_capacity(&self) -> usize {
        self.inner.gc.heap_capacity()
    }

    /// Captures a shareable binary snapshot of this Store's compacted GC heap, globals,
    /// linear memories, tables, and weak/ephemeron entries, plus `extra_roots`.
    pub fn capture_snapshot<R: crate::gc::RootedRef<crate::gc::EqRef>>(
        &mut self,
        extra_roots: &[R],
    ) -> Result<crate::snapshot::StoreSnapshot> {
        crate::snapshot::capture_store_snapshot(&mut self.inner, extra_roots)
    }

    /// Captures a shareable binary snapshot like [`Self::capture_snapshot`], invoking
    /// `encode_extern` to serialize each live host `ExternRef` payload into portable bytes.
    pub fn capture_snapshot_with_externs<R, F>(
        &mut self,
        extra_roots: &[R],
        encode_extern: F,
    ) -> Result<crate::snapshot::StoreSnapshot>
    where
        R: crate::gc::RootedRef<crate::gc::EqRef>,
        F: FnMut(&dyn std::any::Any) -> Result<Vec<u8>>,
    {
        crate::snapshot::capture_store_snapshot_with_externs(
            &mut self.inner,
            extra_roots,
            encode_extern,
        )
    }

    /// Restores a previously captured [`StoreSnapshot`](crate::snapshot::StoreSnapshot) into
    /// this freshly instantiated Store, rebasing all GC heap pointers and `StoreFuncRef`s.
    pub fn restore_snapshot(
        &mut self,
        snapshot: &crate::snapshot::StoreSnapshot,
    ) -> Result<Vec<crate::gc::OwnedRooted<crate::gc::EqRef>>> {
        crate::snapshot::restore_store_snapshot(&mut self.inner, snapshot)
    }

    /// Restores a previously captured [`StoreSnapshot`](crate::snapshot::StoreSnapshot),
    /// invoking `decode_extern` to reconstruct each serialized host `ExternRef` payload.
    pub fn restore_snapshot_with_externs<F>(
        &mut self,
        snapshot: &crate::snapshot::StoreSnapshot,
        decode_extern: F,
    ) -> Result<Vec<crate::gc::OwnedRooted<crate::gc::EqRef>>>
    where
        F: FnMut(
            &crate::snapshot::SnapshotRestoreContext,
            &[u8],
        ) -> Result<Box<dyn std::any::Any + Send + Sync>>,
    {
        crate::snapshot::restore_store_snapshot_with_externs(
            &mut self.inner,
            snapshot,
            decode_extern,
        )
    }

    /// Rebinds a [`GcEphemeron`](crate::gc::GcEphemeron) handle captured from a snapshot's
    /// source Store so it addresses the restored slot in this Store.
    pub fn rebind_ephemeron(&self, entry: crate::gc::GcEphemeron) -> crate::gc::GcEphemeron {
        crate::snapshot::rebind_ephemeron(&self.inner, entry)
    }

    /// Rebinds a [`WeakAnyRef`](crate::gc::WeakAnyRef) handle captured from a snapshot's
    /// source Store so it addresses the restored slot in this Store.
    pub fn rebind_weak_ref(&self, weak: crate::gc::WeakAnyRef) -> crate::gc::WeakAnyRef {
        crate::snapshot::rebind_weak_ref(&self.inner, weak)
    }

    /// Reconstructs a [`GcEphemeron`](crate::gc::GcEphemeron) bound to this Store from
    /// serialized `(slot_idx, generation)` parts after [`Self::restore_snapshot`].
    pub fn ephemeron_from_raw_parts(
        &self,
        slot_idx: u32,
        generation: u32,
    ) -> crate::gc::GcEphemeron {
        crate::gc::GcEphemeron {
            store_id: self.inner.id,
            slot_idx,
            generation,
        }
    }

    /// Reconstructs a [`WeakAnyRef`](crate::gc::WeakAnyRef) bound to this Store from
    /// serialized `(slot_idx, generation)` parts after [`Self::restore_snapshot`].
    pub fn weak_ref_from_raw_parts(&self, slot_idx: u32, generation: u32) -> crate::gc::WeakAnyRef {
        crate::gc::WeakAnyRef {
            store_id: self.inner.id,
            slot_idx,
            generation,
        }
    }

    /// Returns the number of Wasm module instances instantiated in this Store.
    pub fn instance_count(&self) -> usize {
        self.inner.instances.len()
    }

    /// Returns a clone of the [`Module`](crate::module::Module) backing instance `index`, if any.
    pub fn instance_module(&self, index: usize) -> Option<crate::module::Module> {
        self.inner.instances.get(index).map(|i| i.module.clone())
    }

    pub fn throw<R>(&mut self, exn: Rooted<ExnRef>) -> Result<R> {
        let ptr = exn.read_abi_word(&self.inner)?;
        Err(Error::new(ThrownExnSignal { exn_ptr: ptr }))
    }

    pub fn take_pending_exception(&mut self) -> Option<Rooted<ExnRef>> {
        let ptr = self.inner.pending_exn_ptr.take()?;
        Some(Rooted::from_abi_word(&mut self.inner, ptr, false, true))
    }

    pub fn set_fuel(&mut self, fuel: u64) -> Result<()> {
        if !self.inner.engine.config().consume_fuel {
            return Err(Error::msg(
                "fuel consumption is not configured on this Store",
            ));
        }
        self.inner.fuel = Some(fuel);
        Ok(())
    }

    pub fn get_fuel(&self) -> Result<u64> {
        if !self.inner.engine.config().consume_fuel {
            return Err(Error::msg(
                "fuel consumption is not configured on this Store",
            ));
        }
        self.inner
            .fuel
            .ok_or_else(|| Error::msg("fuel consumption is not configured on this Store"))
    }
}

pub struct StoreContext<'a, T> {
    pub(crate) inner: &'a StoreInner,
    pub(crate) data: &'a T,
}

impl<T> Clone for StoreContext<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for StoreContext<'_, T> {}

impl<'a, T> StoreContext<'a, T> {
    pub fn engine(&self) -> &Engine {
        &self.inner.engine
    }

    pub fn data(&self) -> &'a T {
        self.data
    }

    pub fn gc_heap_capacity(&self) -> usize {
        self.inner.gc.heap_capacity()
    }
}

pub struct StoreContextMut<'a, T> {
    pub(crate) inner: &'a mut StoreInner,
    pub(crate) data: &'a mut T,
    pub(crate) _marker: PhantomData<&'a mut Store<T>>,
}

impl<'a, T> StoreContextMut<'a, T> {
    pub fn engine(&self) -> &Engine {
        &self.inner.engine
    }

    pub fn data(&self) -> &T {
        self.data
    }

    pub fn data_mut(&mut self) -> &mut T {
        self.data
    }

    pub fn gc(&mut self, _why: Option<&str>) -> Result<()> {
        unsafe { collect_store_gc(self.inner, 0, 0, true).map_err(Error::from) }
    }

    pub fn gc_heap_capacity(&self) -> usize {
        self.inner.gc.heap_capacity()
    }

    pub fn throw<R>(&mut self, exn: Rooted<ExnRef>) -> Result<R> {
        let ptr = exn.read_abi_word(self.inner)?;
        Err(Error::new(ThrownExnSignal { exn_ptr: ptr }))
    }

    pub(crate) fn split_data_and_inner_mut(&mut self) -> (&mut T, &mut StoreInner) {
        (self.data, self.inner)
    }
}

pub trait AsContext {
    type Data;
    fn as_context(&self) -> StoreContext<'_, Self::Data>;
    #[doc(hidden)]
    fn as_store_inner(&self) -> &StoreInner;
}

pub trait AsContextMut: AsContext {
    fn as_context_mut(&mut self) -> StoreContextMut<'_, Self::Data>;
    #[doc(hidden)]
    fn as_store_inner_mut(&mut self) -> &mut StoreInner;
}

impl<T> AsContext for Store<T> {
    type Data = T;
    fn as_context(&self) -> StoreContext<'_, T> {
        StoreContext {
            inner: &self.inner,
            data: &self.data,
        }
    }
    fn as_store_inner(&self) -> &StoreInner {
        &self.inner
    }
}

impl<T> AsContextMut for Store<T> {
    fn as_context_mut(&mut self) -> StoreContextMut<'_, T> {
        StoreContextMut {
            inner: &mut self.inner,
            data: &mut self.data,
            _marker: PhantomData,
        }
    }
    fn as_store_inner_mut(&mut self) -> &mut StoreInner {
        &mut self.inner
    }
}

impl<T> AsContext for StoreContext<'_, T> {
    type Data = T;
    fn as_context(&self) -> StoreContext<'_, T> {
        *self
    }
    fn as_store_inner(&self) -> &StoreInner {
        self.inner
    }
}

impl<T> AsContext for StoreContextMut<'_, T> {
    type Data = T;
    fn as_context(&self) -> StoreContext<'_, T> {
        StoreContext {
            inner: self.inner,
            data: self.data,
        }
    }
    fn as_store_inner(&self) -> &StoreInner {
        self.inner
    }
}

impl<T> AsContextMut for StoreContextMut<'_, T> {
    fn as_context_mut(&mut self) -> StoreContextMut<'_, T> {
        StoreContextMut {
            inner: self.inner,
            data: self.data,
            _marker: PhantomData,
        }
    }
    fn as_store_inner_mut(&mut self) -> &mut StoreInner {
        self.inner
    }
}

impl<S: AsContext + ?Sized> AsContext for &S {
    type Data = S::Data;
    fn as_context(&self) -> StoreContext<'_, Self::Data> {
        (**self).as_context()
    }
    fn as_store_inner(&self) -> &StoreInner {
        (**self).as_store_inner()
    }
}

impl<S: AsContext + ?Sized> AsContext for &mut S {
    type Data = S::Data;
    fn as_context(&self) -> StoreContext<'_, Self::Data> {
        (**self).as_context()
    }
    fn as_store_inner(&self) -> &StoreInner {
        (**self).as_store_inner()
    }
}

impl<S: AsContextMut + ?Sized> AsContextMut for &mut S {
    fn as_context_mut(&mut self) -> StoreContextMut<'_, Self::Data> {
        (**self).as_context_mut()
    }
    fn as_store_inner_mut(&mut self) -> &mut StoreInner {
        (**self).as_store_inner_mut()
    }
}

impl<'a, T> From<&'a Store<T>> for StoreContext<'a, T> {
    fn from(s: &'a Store<T>) -> Self {
        StoreContext {
            inner: &s.inner,
            data: &s.data,
        }
    }
}

impl<'a, T> From<&'a mut Store<T>> for StoreContext<'a, T> {
    fn from(s: &'a mut Store<T>) -> Self {
        StoreContext {
            inner: &s.inner,
            data: &s.data,
        }
    }
}

impl<'a, T> From<&'a mut Store<T>> for StoreContextMut<'a, T> {
    fn from(s: &'a mut Store<T>) -> Self {
        StoreContextMut {
            inner: &mut s.inner,
            data: &mut s.data,
            _marker: PhantomData,
        }
    }
}

impl<'a, T> From<&'a StoreContext<'_, T>> for StoreContext<'a, T> {
    fn from(s: &'a StoreContext<'_, T>) -> Self {
        StoreContext {
            inner: s.inner,
            data: s.data,
        }
    }
}

impl<'a, T> From<&'a StoreContextMut<'_, T>> for StoreContext<'a, T> {
    fn from(s: &'a StoreContextMut<'_, T>) -> Self {
        StoreContext {
            inner: s.inner,
            data: s.data,
        }
    }
}

impl<'a, T> From<&'a mut StoreContextMut<'_, T>> for StoreContext<'a, T> {
    fn from(s: &'a mut StoreContextMut<'_, T>) -> Self {
        StoreContext {
            inner: s.inner,
            data: s.data,
        }
    }
}

impl<'a, S: AsContextMut> From<&'a crate::gc::RootScope<'_, S>> for StoreContext<'a, S::Data> {
    fn from(s: &'a crate::gc::RootScope<'_, S>) -> Self {
        s.store.as_context()
    }
}

impl<'a, S: AsContextMut> From<&'a mut crate::gc::RootScope<'_, S>> for StoreContext<'a, S::Data> {
    fn from(s: &'a mut crate::gc::RootScope<'_, S>) -> Self {
        s.store.as_context()
    }
}

impl<'a, S: AsContextMut> From<&'a mut crate::gc::RootScope<'_, S>>
    for StoreContextMut<'a, S::Data>
{
    fn from(s: &'a mut crate::gc::RootScope<'_, S>) -> Self {
        s.store.as_context_mut()
    }
}
