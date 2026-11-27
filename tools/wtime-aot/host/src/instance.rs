//! `Linker`, `Instance`, `InstancePre`, `Caller`, `Extern`, `Func`, `TypedFunc`,
//! `IntoFunc`, `Memory`, `Global`, `Table`, and `Tag`.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use wtime_aot::x64::module::GcRefFamily;
use wtime_aot::x64::store_rt::{ty_to_code, val_type_to_machine_ty};
use wtime_aot::x64::{
    CTX_HEADER_BYTES, INTERNAL_EXN_SIGNAL, OFF_MEM_BASE, OFF_MEM_RESERVE_BYTES,
    OFF_STORE_CANON_MAP, OFF_STORE_ELEM_BUF_BYTES, OFF_STORE_ELEM_BUF_PTR, OFF_STORE_FUNC_REFS,
    OFF_STORE_INSTANCE_ID, OFF_STORE_MEM_DESCS, OFF_STORE_PTR, OFF_STORE_TABLE_DESCS,
    OFF_STORE_TAG_IDS,
};

use crate::abi::{
    HostFuncCtx, StoreFuncRef, StoreMemDesc, StoreTableDesc, TRAP_HOST_STORED_ERROR,
    WtStoreExportTrampFn, ZERO_MEM_DESC,
};
use crate::config::Engine;
use crate::error::{Error, Result, Trap};
use crate::gc::{AnyRef, ArrayRef, EqRef, ExnRef, ExternRef, Rooted, StructRef, Val, ValRaw};
use crate::memory::{PrivateMemoryData, SharedMemory, WASM_PAGE_BYTES};
use crate::module::Module;
use crate::signal::{ActiveWasmFrame, pop_active_frame, push_active_frame, sigsetjmp};
use crate::store::{
    AsContext, AsContextMut, DynamicTableData, ErasedHostCallback, HostFuncEntry, InstanceData,
    MemRef, StandaloneGlobalData, StoreContext, StoreContextMut, StoreInner, ThrownExnSignal,
};
use crate::types::{
    CanonComposite, ExportKind, FuncType, GlobalType, MemoryType, ModuleExport, Mutability,
    TableType, TagType, ValType,
};

// =========================================================================
// Store-Owned Resource Handles: `Memory`, `Global`, `Table`, `Tag`, `Func`, `Extern`
// =========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Memory {
    pub(crate) store_id: u64,
    pub(crate) index: usize,
}

impl Memory {
    pub fn new(mut store: impl AsContextMut, ty: MemoryType) -> Result<Self> {
        let inner = store.as_store_inner_mut();
        let data = PrivateMemoryData::new(ty, inner.memory_limit_bytes, inner.interrupt.clone())?;
        let index = inner.memories.len();
        inner.memories.push(data);
        Ok(Self {
            store_id: inner.id,
            index,
        })
    }

    pub fn ty(&self, store: impl AsContext) -> MemoryType {
        let inner = store.as_store_inner();
        assert_eq!(inner.id, self.store_id, "Memory belongs to another Store");
        inner.memories[self.index].ty.clone()
    }

    pub fn size(&self, store: impl AsContext) -> u64 {
        let inner = store.as_store_inner();
        assert_eq!(inner.id, self.store_id, "Memory belongs to another Store");
        inner.memories[self.index]
            .desc
            .len_bytes
            .load(Ordering::Acquire)
            / WASM_PAGE_BYTES
    }

    pub fn data_size(&self, store: impl AsContext) -> usize {
        let inner = store.as_store_inner();
        assert_eq!(inner.id, self.store_id, "Memory belongs to another Store");
        inner.memories[self.index]
            .desc
            .len_bytes
            .load(Ordering::Acquire) as usize
    }

    pub fn grow(&self, mut store: impl AsContextMut, delta: u64) -> Result<u64> {
        let inner = store.as_store_inner_mut();
        if inner.id != self.store_id {
            return Err(Error::msg("Memory belongs to another Store"));
        }
        let limit = inner.memory_limit_bytes;
        inner.memories[self.index]
            .grow(delta, limit)
            .ok_or_else(|| Error::msg("failed to grow linear memory"))
    }

    pub fn data<'a, T: 'a>(&self, store: impl Into<StoreContext<'a, T>>) -> &'a [u8] {
        let ctx: StoreContext<'a, T> = store.into();
        assert_eq!(
            ctx.inner.id, self.store_id,
            "Memory belongs to another Store"
        );
        ctx.inner.memories[self.index].data()
    }

    pub fn data_mut<'a, T: 'a>(&self, store: impl Into<StoreContextMut<'a, T>>) -> &'a mut [u8] {
        let ctx: StoreContextMut<'a, T> = store.into();
        assert_eq!(
            ctx.inner.id, self.store_id,
            "Memory belongs to another Store"
        );
        ctx.inner.memories[self.index].data_mut()
    }

    pub fn data_ptr(&self, store: impl AsContext) -> *mut u8 {
        let inner = store.as_store_inner();
        assert_eq!(inner.id, self.store_id, "Memory belongs to another Store");
        inner.memories[self.index].desc.base
    }

    pub fn read(&self, store: impl AsContext, offset: usize, buffer: &mut [u8]) -> Result<()> {
        let inner = store.as_store_inner();
        if inner.id != self.store_id {
            return Err(Error::msg("Memory belongs to another Store"));
        }
        let data = inner.memories[self.index].data();
        let end = offset
            .checked_add(buffer.len())
            .filter(|&e| e <= data.len())
            .ok_or(Trap::MemoryOutOfBounds)?;
        buffer.copy_from_slice(&data[offset..end]);
        Ok(())
    }

    pub fn write(&self, mut store: impl AsContextMut, offset: usize, buffer: &[u8]) -> Result<()> {
        let inner = store.as_store_inner_mut();
        if inner.id != self.store_id {
            return Err(Error::msg("Memory belongs to another Store"));
        }
        let data = inner.memories[self.index].data_mut();
        let end = offset
            .checked_add(buffer.len())
            .filter(|&e| e <= data.len())
            .ok_or(Trap::MemoryOutOfBounds)?;
        data[offset..end].copy_from_slice(buffer);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Global {
    pub(crate) store_id: u64,
    pub(crate) index: usize,
}

impl Global {
    pub fn new(mut store: impl AsContextMut, ty: GlobalType, val: Val) -> Result<Self> {
        let inner = store.as_store_inner_mut();
        let wvt = ty.content().to_wasm();
        let is_traced = is_wasm_val_traced(inner, wvt);
        let mut cell = Box::new([0u64; 2]);
        write_val_to_cell(inner, cell.as_mut_ptr().cast::<u8>(), wvt, &val)?;
        let cell_ptr = cell.as_mut_ptr().cast::<u8>();
        inner.standalone_globals.push(StandaloneGlobalData {
            _ty: ty.clone(),
            cell,
            is_traced,
        });
        let index = inner.global_handles.len();
        inner.global_handles.push((cell_ptr, ty));
        Ok(Self {
            store_id: inner.id,
            index,
        })
    }

    pub fn ty(&self, store: impl AsContext) -> GlobalType {
        let inner = store.as_store_inner();
        assert_eq!(inner.id, self.store_id, "Global belongs to another Store");
        inner.global_handles[self.index].1.clone()
    }

    pub fn get(&self, mut store: impl AsContextMut) -> Val {
        let inner = store.as_store_inner_mut();
        assert_eq!(inner.id, self.store_id, "Global belongs to another Store");
        let (cell_ptr, ref ty) = inner.global_handles[self.index];
        let wvt = ty.content().to_wasm();
        unsafe { read_val_from_cell(inner, cell_ptr, wvt) }.expect("Global::get cell read")
    }

    pub fn set(&self, mut store: impl AsContextMut, val: Val) -> Result<()> {
        let inner = store.as_store_inner_mut();
        if inner.id != self.store_id {
            return Err(Error::msg("Global belongs to another Store"));
        }
        let (cell_ptr, ref ty) = inner.global_handles[self.index];
        if ty.mutability() != Mutability::Var {
            return Err(Error::msg("cannot set immutable global"));
        }
        let wvt = ty.content().to_wasm();
        write_val_to_cell(inner, cell_ptr, wvt, &val)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Table {
    pub(crate) store_id: u64,
    pub(crate) index: usize,
}

impl Table {
    pub fn new(mut store: impl AsContextMut, ty: TableType, init: Val) -> Result<Self> {
        let inner = store.as_store_inner_mut();
        let init_word = init.to_abi_word(inner)?;
        let fam = ref_family_of_reftype(inner, ty.element());
        let data = DynamicTableData::new(ty, init_word, fam)?;
        let index = inner.tables.len();
        inner.tables.push(data);
        Ok(Self {
            store_id: inner.id,
            index,
        })
    }

    pub fn ty(&self, store: impl AsContext) -> TableType {
        let inner = store.as_store_inner();
        assert_eq!(inner.id, self.store_id, "Table belongs to another Store");
        inner.tables[self.index].ty.clone()
    }

    pub fn size(&self, store: impl AsContext) -> u64 {
        let inner = store.as_store_inner();
        assert_eq!(inner.id, self.store_id, "Table belongs to another Store");
        u64::from(inner.tables[self.index].desc.cur_len)
    }

    pub fn grow(&self, mut store: impl AsContextMut, delta: u64, init: Val) -> Result<u64> {
        let inner = store.as_store_inner_mut();
        if inner.id != self.store_id {
            return Err(Error::msg("Table belongs to another Store"));
        }
        let init_word = init.to_abi_word(inner)?;
        let limit = inner.table_elements_limit;
        inner.tables[self.index]
            .grow(init_word, delta, limit)
            .ok_or_else(|| Error::msg("failed to grow table"))
    }

    pub fn get(&self, mut store: impl AsContextMut, index: u64) -> Option<Val> {
        let inner = store.as_store_inner_mut();
        assert_eq!(inner.id, self.store_id, "Table belongs to another Store");
        let tbl = &inner.tables[self.index];
        let cur = u64::from(tbl.desc.cur_len);
        if index >= cur {
            return None;
        }
        let word = unsafe { *tbl.desc.ptr.add(index as usize) };
        let wvt = wtime_types::ValType::Ref(tbl.ty.element().to_wasm());
        Val::from_abi_words(inner, wvt, &[word]).ok()
    }

    pub fn set(&self, mut store: impl AsContextMut, index: u64, val: Val) -> Result<()> {
        let inner = store.as_store_inner_mut();
        if inner.id != self.store_id {
            return Err(Error::msg("Table belongs to another Store"));
        }
        let word = val.to_abi_word(inner)?;
        let tbl = &mut inner.tables[self.index];
        if index >= u64::from(tbl.desc.cur_len) {
            return Err(Trap::TableOutOfBounds.into());
        }
        unsafe { *tbl.desc.ptr.add(index as usize) = word };
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tag {
    pub(crate) store_id: u64,
    pub(crate) index: usize,
}

impl Tag {
    pub fn new(mut store: impl AsContextMut, ty: &TagType) -> Result<Self> {
        let inner = store.as_store_inner_mut();
        let index = inner.tags.len();
        inner.alloc_tag(ty.clone());
        Ok(Self {
            store_id: inner.id,
            index,
        })
    }

    pub fn ty(&self, store: impl AsContext) -> TagType {
        let inner = store.as_store_inner();
        assert_eq!(inner.id, self.store_id, "Tag belongs to another Store");
        inner.tags[self.index].ty.clone()
    }

    #[allow(clippy::should_implement_trait)]
    pub fn eq(a: &Self, b: &Self, _store: impl AsContext) -> bool {
        a.store_id == b.store_id && a.index == b.index
    }
}

/// Unified handle to either a compiled Wasm function or a Store-registered host function.
#[derive(Clone, Copy, Debug)]
pub struct Func {
    pub(crate) store_id: u64,
    pub(crate) func_ref_ptr: *const StoreFuncRef,
}

// SAFETY: Dereferenced exclusively under a borrow of the owning `Store` (`store.id == self.store_id`).
unsafe impl Send for Func {}
unsafe impl Sync for Func {}

impl Func {
    pub(crate) fn from_raw_ptr(store_id: u64, func_ref_ptr: *const StoreFuncRef) -> Self {
        Self {
            store_id,
            func_ref_ptr,
        }
    }

    pub fn ty(&self, store: impl AsContext) -> FuncType {
        let inner = store.as_store_inner();
        assert_eq!(inner.id, self.store_id, "Func belongs to another Store");
        let cid = unsafe { (*self.func_ref_ptr).canon_type_id };
        FuncType::from_canon(Arc::clone(&inner.engine.inner), cid)
    }

    pub fn new<T: 'static>(
        mut store: impl AsContextMut<Data = T>,
        ty: FuncType,
        func: impl Fn(Caller<'_, T>, &[Val], &mut [Val]) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        Self::try_new(&mut store, ty, func).expect("Func::new failed")
    }

    pub fn try_new<T: 'static>(
        mut store: impl AsContextMut<Data = T>,
        ty: FuncType,
        func: impl Fn(Caller<'_, T>, &[Val], &mut [Val]) -> Result<()> + Send + Sync + 'static,
    ) -> Result<Self> {
        let inner = store.as_store_inner_mut();
        let cb = make_dynamic_host_callback::<T>(&ty, func);
        Ok(register_host_func_in_store(inner, ty, cb))
    }

    /// # Safety
    /// The caller guarantees `func` reads/writes `ValRaw` elements matching `ty`.
    pub unsafe fn new_unchecked<T: 'static>(
        mut store: impl AsContextMut<Data = T>,
        ty: FuncType,
        func: impl Fn(Caller<'_, T>, &mut [MaybeUninit<ValRaw>]) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        let inner = store.as_store_inner_mut();
        let cb = make_unchecked_host_callback::<T>(&ty, func);
        register_host_func_in_store(inner, ty, cb)
    }

    pub fn wrap<T: 'static, Params, Results>(
        mut store: impl AsContextMut<Data = T>,
        func: impl IntoFunc<T, Params, Results>,
    ) -> Self {
        let engine = store.as_context().engine().clone();
        let (ty, cb) = func.into_erased(&engine);
        let inner = store.as_store_inner_mut();
        register_host_func_in_store(inner, ty, cb)
    }

    pub fn call(
        &self,
        mut store: impl AsContextMut,
        params: &[Val],
        results: &mut [Val],
    ) -> Result<()> {
        let inner = store.as_store_inner_mut();
        if inner.id != self.store_id {
            return Err(Error::msg("Func::call called on wrong Store"));
        }
        let fr = unsafe { *self.func_ref_ptr };
        let ft = FuncType::from_canon(Arc::clone(&inner.engine.inner), fr.canon_type_id);
        let (wparams, wresults) = ft.wasm_params_results();
        if params.len() != wparams.len() || results.len() != wresults.len() {
            return Err(Error::msg("Func::call argument/result arity mismatch"));
        }

        // Pack flat u64 args buffer.
        let mut args_buf = Vec::with_capacity(wparams.len() * 2);
        for (val, &vt) in params.iter().zip(&wparams) {
            pack_val_flat(inner, val, vt, &mut args_buf)?;
        }
        let ret_words: usize = wresults
            .iter()
            .map(|&vt| {
                if vt == wtime_types::ValType::V128 {
                    2
                } else {
                    1
                }
            })
            .sum();
        let mut rets_buf = vec![0u64; ret_words.max(1)];

        invoke_store_func_raw(inner, fr, &ft, &args_buf, &mut rets_buf)?;

        // Unpack flat u64 results buffer.
        let mut cursor = 0usize;
        for (slot, &vt) in results.iter_mut().zip(&wresults) {
            let n = if vt == wtime_types::ValType::V128 {
                2
            } else {
                1
            };
            *slot = Val::from_abi_words(inner, vt, &rets_buf[cursor..cursor + n])?;
            cursor += n;
        }
        Ok(())
    }

    pub fn typed<Params: WasmParams, Results: WasmResults>(
        &self,
        store: impl AsContext,
    ) -> Result<TypedFunc<Params, Results>> {
        let inner = store.as_store_inner();
        if inner.id != self.store_id {
            return Err(Error::msg("Func::typed called on wrong Store"));
        }
        let ft = self.ty(store);
        let (wparams, wresults) = ft.wasm_params_results();
        Params::check_types(&wparams)?;
        Results::check_types(&wresults)?;
        Ok(TypedFunc {
            func: *self,
            _marker: PhantomData,
        })
    }
}

/// Statically typed function handle (`TypedFunc<Params, Results>`).
pub struct TypedFunc<Params, Results> {
    func: Func,
    _marker: PhantomData<fn(Params) -> Results>,
}

impl<P, R> Clone for TypedFunc<P, R> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<P, R> Copy for TypedFunc<P, R> {}

impl<Params: WasmParams, Results: WasmResults> TypedFunc<Params, Results> {
    pub fn func(&self) -> &Func {
        &self.func
    }

    pub fn call(&self, mut store: impl AsContextMut, params: Params) -> Result<Results> {
        let inner = store.as_store_inner_mut();
        if inner.id != self.func.store_id {
            return Err(Error::msg("TypedFunc::call called on wrong Store"));
        }
        let fr = unsafe { *self.func.func_ref_ptr };
        let ft = FuncType::from_canon(Arc::clone(&inner.engine.inner), fr.canon_type_id);
        let mut args_buf = [0u64; 24];
        let mut rets_buf = [0u64; 24];
        let mut argc = 0usize;
        params.write_flat(inner, &mut args_buf, &mut argc)?;
        invoke_store_func_raw(inner, fr, &ft, &args_buf[..argc], &mut rets_buf)?;
        let mut rc = 0usize;
        Results::read_flat(inner, &rets_buf, &mut rc)
    }
}

/// Core low-level Wasm/Host invocation helper with `ActiveWasmFrame` signal guard & reentrancy restoration.
fn invoke_store_func_raw(
    inner: &mut StoreInner,
    fr: StoreFuncRef,
    ft: &FuncType,
    args_buf: &[u64],
    rets_buf: &mut [u64],
) -> Result<()> {
    // Direct Rust-to-host fast path (no machine trampoline needed!).
    if fr.host_func_id != u32::MAX {
        let entry = inner
            .host_funcs
            .get(fr.host_func_id as usize)
            .ok_or_else(|| Error::msg("invalid host_func_id"))?;
        let cb = Arc::clone(&entry.cb);
        let user_data = inner.user_data_ptr;
        let prev_caller = inner.active_caller_instance;
        inner.active_caller_instance = None;
        let res = cb(
            inner as *mut StoreInner,
            user_data,
            args_buf.as_ptr(),
            rets_buf.as_mut_ptr(),
        );
        inner.active_caller_instance = prev_caller;
        return res;
    }

    inner.consume_entry_fuel()?;

    let tramp: WtStoreExportTrampFn = inner
        .find_export_tramp_for_func_type(ft)
        .ok_or_else(|| Error::msg("no export-entry trampoline for function signature"))?;

    inner.sync_type_descs();
    inner.stamp_stack_floors();
    inner.state.last_trap_code = 0;

    let entry_inst_id = unsafe {
        *(fr.ctx_ptr
            .offset(OFF_STORE_INSTANCE_ID as isize)
            .cast::<u64>())
    } as usize;
    let prev_entry = inner.active_entry_instance;
    inner.active_entry_instance = Some(entry_inst_id);

    let saved_stop = inner.state.gc_walk_stop_rsp;
    let saved_chain = inner.state.gc_stack_chain;
    let saved_init_ptr = inner.state.gc_init_roots_ptr;
    let saved_init_len = inner.state.gc_init_roots_len;
    let store_state_ptr = std::ptr::addr_of_mut!(inner.state);

    let mut frame = ActiveWasmFrame {
        prev: std::ptr::null_mut(),
        jmp_buf: crate::signal::SigJmpBuf::default(),
        store_ptr: store_state_ptr,
        trap_code: 0,
    };

    let status: u32 = unsafe {
        push_active_frame(&mut frame);
        let rc = if sigsetjmp(std::ptr::addr_of_mut!(frame.jmp_buf), 0) == 0 {
            tramp(
                fr.ctx_ptr,
                fr.code_ptr,
                args_buf.as_ptr(),
                rets_buf.as_mut_ptr(),
            )
        } else {
            // Unwound from signal handler (Interrupt / Guard-page / SIGFPE / SIGILL).
            inner.state.gc_walk_stop_rsp = saved_stop;
            inner.state.gc_stack_chain = saved_chain;
            inner.state.gc_init_roots_ptr = saved_init_ptr;
            inner.state.gc_init_roots_len = saved_init_len;
            inner.interrupt.restore_all_rw();
            frame.trap_code
        };
        pop_active_frame(&mut frame);
        rc
    };
    inner.active_entry_instance = prev_entry;

    interpret_wasm_status(inner, status)
}

fn interpret_wasm_status(inner: &mut StoreInner, status: u32) -> Result<()> {
    if status == 0 {
        return Ok(());
    }
    if status == crate::abi::TRAP_HOST_PANIC {
        if let Some(payload) = inner.pending_host_panic.take() {
            std::panic::resume_unwind(payload);
        }
        panic!("host callback panicked across Wasm boundary");
    }
    if status == TRAP_HOST_STORED_ERROR {
        return Err(inner
            .pending_host_error
            .take()
            .unwrap_or_else(|| Error::msg("host callback failed")));
    }
    if status == u32::from(INTERNAL_EXN_SIGNAL) {
        let exn_ptr = inner.state.exn_active_ptr;
        inner.state.exn_active_ptr = 0;
        inner.pending_exn_ptr = Some(exn_ptr);
        return Err(Error::new(ThrownExnSignal { exn_ptr }));
    }
    Err(Trap::from_x64_code(status).into())
}

#[derive(Clone, Debug)]
pub enum Extern {
    Func(Func),
    Global(Global),
    Table(Table),
    Memory(Memory),
    SharedMemory(SharedMemory),
    Tag(Tag),
}

impl Extern {
    pub fn ty(&self, store: impl AsContext) -> crate::types::ExternType {
        match self {
            Self::Func(f) => crate::types::ExternType::Func(f.ty(store)),
            Self::Global(g) => crate::types::ExternType::Global(g.ty(store)),
            Self::Table(t) => crate::types::ExternType::Table(t.ty(store)),
            Self::Memory(m) => crate::types::ExternType::Memory(m.ty(store)),
            Self::SharedMemory(sm) => crate::types::ExternType::Memory(sm.ty()),
            Self::Tag(t) => crate::types::ExternType::Tag(t.ty(store)),
        }
    }

    pub fn into_func(self) -> Option<Func> {
        match self {
            Self::Func(v) => Some(v),
            _ => None,
        }
    }
    pub fn into_global(self) -> Option<Global> {
        match self {
            Self::Global(v) => Some(v),
            _ => None,
        }
    }
    pub fn into_table(self) -> Option<Table> {
        match self {
            Self::Table(v) => Some(v),
            _ => None,
        }
    }
    pub fn into_memory(self) -> Option<Memory> {
        match self {
            Self::Memory(v) => Some(v),
            _ => None,
        }
    }
    pub fn into_shared_memory(self) -> Option<SharedMemory> {
        match self {
            Self::SharedMemory(v) => Some(v),
            _ => None,
        }
    }
    pub fn into_tag(self) -> Option<Tag> {
        match self {
            Self::Tag(v) => Some(v),
            _ => None,
        }
    }
}

impl From<Func> for Extern {
    fn from(v: Func) -> Self {
        Self::Func(v)
    }
}
impl From<Global> for Extern {
    fn from(v: Global) -> Self {
        Self::Global(v)
    }
}
impl From<Table> for Extern {
    fn from(v: Table) -> Self {
        Self::Table(v)
    }
}
impl From<Memory> for Extern {
    fn from(v: Memory) -> Self {
        Self::Memory(v)
    }
}
impl From<SharedMemory> for Extern {
    fn from(v: SharedMemory) -> Self {
        Self::SharedMemory(v)
    }
}
impl From<Tag> for Extern {
    fn from(v: Tag) -> Self {
        Self::Tag(v)
    }
}

// =========================================================================
// `Caller<'a, T>` Passed into Host Callbacks
// =========================================================================

pub struct Caller<'a, T> {
    pub(crate) inner: &'a mut StoreInner,
    pub(crate) data: &'a mut T,
}

impl<T> Caller<'_, T> {
    pub fn data(&self) -> &T {
        self.data
    }

    pub fn data_mut(&mut self) -> &mut T {
        self.data
    }

    pub fn engine(&self) -> &Engine {
        &self.inner.engine
    }

    pub fn gc(&mut self, _why: Option<&str>) -> Result<()> {
        unsafe { crate::gc::collect_store_gc(self.inner, 0, 0).map_err(Error::from) }
    }

    pub fn gc_heap_capacity(&self) -> usize {
        self.inner.gc.from_cap
    }

    pub fn throw<R>(&mut self, exn: Rooted<ExnRef>) -> Result<R> {
        let ptr = exn.read_abi_word(self.inner)?;
        Err(Error::new(ThrownExnSignal { exn_ptr: ptr }))
    }

    pub fn get_export(&mut self, name: &str) -> Option<Extern> {
        let inst_idx = self.inner.active_caller_instance?;
        let inst = Instance {
            store_id: self.inner.id,
            index: inst_idx,
        };
        inst.get_export(&mut *self, name)
    }

    pub fn get_module_export(&mut self, export: &ModuleExport) -> Option<Extern> {
        let inst_idx = self.inner.active_caller_instance?;
        let inst = Instance {
            store_id: self.inner.id,
            index: inst_idx,
        };
        inst.get_module_export(&mut *self, export)
    }
}

impl<T> AsContext for Caller<'_, T> {
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

impl<T> AsContextMut for Caller<'_, T> {
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

impl<'a, T> From<&'a Caller<'_, T>> for StoreContext<'a, T> {
    fn from(c: &'a Caller<'_, T>) -> Self {
        StoreContext {
            inner: c.inner,
            data: c.data,
        }
    }
}

impl<'a, T> From<&'a mut Caller<'_, T>> for StoreContext<'a, T> {
    fn from(c: &'a mut Caller<'_, T>) -> Self {
        StoreContext {
            inner: c.inner,
            data: c.data,
        }
    }
}

impl<'a, T> From<&'a mut Caller<'_, T>> for StoreContextMut<'a, T> {
    fn from(c: &'a mut Caller<'_, T>) -> Self {
        StoreContextMut {
            inner: c.inner,
            data: c.data,
            _marker: PhantomData,
        }
    }
}

// =========================================================================
// `Instance` (Instantiation & Exports)
// =========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instance {
    pub(crate) store_id: u64,
    pub(crate) index: usize,
}

impl Instance {
    /// Instantiates `module` inside `store` with positional `imports` matching `module.imports()`.
    pub fn new(mut store: impl AsContextMut, module: &Module, imports: &[Extern]) -> Result<Self> {
        let inner = store.as_store_inner_mut();
        if !Engine::same(&inner.engine, module.engine()) {
            return Err(Error::msg("Instance::new module belongs to another Engine"));
        }
        if inner._pool_guard.is_none() {
            inner._pool_guard = inner.engine.acquire_store_slot()?;
        }
        let meta = &module.inner.meta;
        if imports.len() != meta.import_order.len() {
            return Err(Error::msg(format!(
                "import count mismatch: expected {}, got {}",
                meta.import_order.len(),
                imports.len()
            )));
        }

        // Patch any previously created host functions that didn't have a trampoline yet.
        inner.patch_pending_host_tramps_from(module);

        // 1. Partition positional `imports` into kind-indexed arrays.
        let mut imp_funcs: Vec<Option<Func>> = vec![None; meta.imported_funcs.len()];
        let mut imp_tables: Vec<Option<Table>> = vec![None; meta.imported_tables.len()];
        let mut imp_mems: Vec<Option<MemRef>> = vec![None; meta.imported_mems.len()];
        let mut imp_globals: Vec<Option<Global>> = vec![None; meta.imported_globals.len()];
        let mut imp_tags: Vec<Option<Tag>> = vec![None; meta.imported_tags.len()];

        for (pos, (&(kind, idx), ext)) in meta.import_order.iter().zip(imports.iter()).enumerate() {
            let idx = idx as usize;
            match (kind, ext) {
                (0, Extern::Func(f)) => {
                    if f.store_id != inner.id {
                        return Err(Error::msg("imported Func belongs to another Store"));
                    }
                    // Ensure host function has a compiled host trampoline pointer!
                    let fr = unsafe { &mut *(f.func_ref_ptr as *mut StoreFuncRef) };
                    let expected_ti = meta.imported_funcs[idx].2;
                    let expected_gcid =
                        module.inner.local_type_to_global_canon[expected_ti as usize];
                    {
                        let reg = inner
                            .engine
                            .inner
                            .types
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        if !reg.is_subtype(fr.canon_type_id, expected_gcid) {
                            return Err(Error::msg(format!(
                                "incompatible function import #{} (`{}::{}`)",
                                pos, meta.imported_funcs[idx].0, meta.imported_funcs[idx].1
                            )));
                        }
                    }
                    if fr.host_func_id != u32::MAX && fr.code_ptr.is_null() {
                        let ft =
                            FuncType::from_canon(Arc::clone(&inner.engine.inner), expected_gcid);
                        fr.code_ptr = module.host_tramp_for_func_type(&ft).ok_or_else(|| {
                            Error::msg("missing host trampoline for imported function signature")
                        })?;
                    }
                    imp_funcs[idx] = Some(*f);
                }
                (1, Extern::Table(t)) => {
                    if t.store_id != inner.id {
                        return Err(Error::msg("imported Table belongs to another Store"));
                    }
                    imp_tables[idx] = Some(*t);
                }
                (2, Extern::Memory(m)) => {
                    if m.store_id != inner.id {
                        return Err(Error::msg("imported Memory belongs to another Store"));
                    }
                    let (_, _, min_p, _, dmax, is64, shared) = &meta.imported_mems[idx];
                    let expected = MemoryType::from_raw(*min_p, *dmax, *is64, *shared);
                    if !inner.memories[m.index].ty.matches(&expected) {
                        return Err(Error::msg(format!(
                            "incompatible memory import `{}`::`{}`",
                            meta.imported_mems[idx].0, meta.imported_mems[idx].1
                        )));
                    }
                    imp_mems[idx] = Some(MemRef::Private(m.index));
                }
                (2, Extern::SharedMemory(sm)) => {
                    let (_, _, min_p, _, dmax, is64, shared) = &meta.imported_mems[idx];
                    let expected = MemoryType::from_raw(*min_p, *dmax, *is64, *shared);
                    if !sm.inner.ty.matches(&expected) {
                        return Err(Error::msg(format!(
                            "incompatible shared memory import `{}`::`{}`",
                            meta.imported_mems[idx].0, meta.imported_mems[idx].1
                        )));
                    }
                    if !inner
                        .shared_memories
                        .iter()
                        .any(|s| Arc::ptr_eq(s, &sm.inner))
                    {
                        inner.shared_memories.push(Arc::clone(&sm.inner));
                    }
                    imp_mems[idx] = Some(MemRef::Shared(sm.clone()));
                }
                (3, Extern::Global(g)) => {
                    if g.store_id != inner.id {
                        return Err(Error::msg("imported Global belongs to another Store"));
                    }
                    imp_globals[idx] = Some(*g);
                }
                (4, Extern::Tag(t)) => {
                    if t.store_id != inner.id {
                        return Err(Error::msg("imported Tag belongs to another Store"));
                    }
                    imp_tags[idx] = Some(*t);
                }
                _ => {
                    return Err(Error::msg(format!(
                        "import #{pos} has wrong extern kind (expected kind {kind})"
                    )));
                }
            }
        }

        // 2. Allocate instance `Ctx` buffer (zeroed, 16-byte aligned via Vec<u128> -> Box<[u8]>?
        //    Use `alloc_zeroed` or `Vec<u8>`? Wait: `Box<[u8]>` might only be 1-byte aligned!
        //    Use over-allocated or 16-byte aligned `Ctx`! Wait: `Vec<u128>` guarantees 16B alignment
        //    and converts cleanly, OR `alloc_zeroed`? Actually `Box<[u128]>` is 16B-aligned!)
        let ctx_bytes = (meta.ctx_total_bytes as usize).max(CTX_HEADER_BYTES);
        let ctx_words = ctx_bytes.div_ceil(16);
        let mut ctx_aligned: Vec<u128> = vec![0u128; ctx_words];
        let ctx_raw_ptr = ctx_aligned.as_mut_ptr().cast::<u8>();
        // Keep ownership as `Box<[u8]>`? No — `Box<[u128]>` would keep 16B alignment on drop!
        // Let's convert cleanly: wait, `InstanceData::ctx_buf` is `Box<[u8]>` so let's allocate
        // exact size via `Vec<u8>`? On glibc x86_64 malloc, any allocation >= 16 bytes is 16-byte aligned,
        // and `ctx_bytes >= 256`! Assert `(ptr as usize) & 15 == 0` to be 100% certain!
        drop(ctx_aligned);
        let _ = ctx_raw_ptr;
        let mut ctx_buf: Box<[u8]> = vec![0u8; ctx_bytes].into_boxed_slice();
        assert_eq!(
            (ctx_buf.as_ptr() as usize) & 15,
            0,
            "Ctx buffer must be 16-byte aligned for V128 globals"
        );
        let ctx_ptr = ctx_buf.as_mut_ptr();

        // 3. Allocate optional passive GC element buffer (`OFF_STORE_ELEM_BUF_PTR`).
        let (elem_buf, elem_buf_bytes) = if meta.gc_elem_buf_bytes > 0 {
            let sz = meta.gc_elem_buf_bytes as usize;
            let p = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    sz,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                    -1,
                    0,
                )
            };
            if p == libc::MAP_FAILED {
                return Err(Error::msg("failed to allocate passive GC elem buffer"));
            }
            (p.cast::<u8>(), sz)
        } else {
            (std::ptr::null_mut(), 0usize)
        };

        // 4. Create defined memories & assemble `mem_desc_ptrs` (`0..total_mems`).
        let mut mem_handles: Vec<MemRef> =
            Vec::with_capacity(meta.imported_mems.len() + meta.defined_mems.len());
        let mut mem_desc_ptrs: Vec<*mut StoreMemDesc> =
            Vec::with_capacity(meta.imported_mems.len() + meta.defined_mems.len());
        for m in imp_mems {
            let mr = m.expect("validated imported memory");
            let desc_ptr = match &mr {
                MemRef::Private(i) => inner.memories[*i].desc.as_mut() as *mut StoreMemDesc,
                MemRef::Shared(sm) => {
                    sm.inner.desc.as_ref() as *const StoreMemDesc as *mut StoreMemDesc
                }
            };
            mem_handles.push(mr);
            mem_desc_ptrs.push(desc_ptr);
        }
        for &(_mi, min_p, _eff_max, dmax, is64, shared) in &meta.defined_mems {
            let ty = MemoryType::from_raw(min_p, dmax, is64, shared);
            if shared {
                let sm = SharedMemory::new(&inner.engine, ty)?;
                let desc_ptr = sm.inner.desc.as_ref() as *const StoreMemDesc as *mut StoreMemDesc;
                inner.shared_memories.push(Arc::clone(&sm.inner));
                mem_handles.push(MemRef::Shared(sm));
                mem_desc_ptrs.push(desc_ptr);
            } else {
                let data =
                    PrivateMemoryData::new(ty, inner.memory_limit_bytes, inner.interrupt.clone())?;
                let idx = inner.memories.len();
                inner.memories.push(data);
                let desc_ptr = inner.memories[idx].desc.as_mut() as *mut StoreMemDesc;
                mem_handles.push(MemRef::Private(idx));
                mem_desc_ptrs.push(desc_ptr);
            }
        }
        let mem_desc_ptrs = mem_desc_ptrs.into_boxed_slice();

        // 5. Create defined tables & assemble `table_desc_ptrs` (`0..total_tables`).
        let mut table_indices: Vec<usize> =
            Vec::with_capacity(meta.imported_tables.len() + meta.defined_tables.len());
        let mut table_desc_ptrs: Vec<*mut StoreTableDesc> =
            Vec::with_capacity(meta.imported_tables.len() + meta.defined_tables.len());
        for t in imp_tables {
            let tbl = t.expect("validated imported table");
            table_indices.push(tbl.index);
            table_desc_ptrs.push(inner.tables[tbl.index].desc.as_mut() as *mut StoreTableDesc);
        }
        for &(_ti, _min_l, _eff_max, _is64, _is_func, fam_code, raw_tt) in &meta.defined_tables {
            let tt = TableType::from_module_wasm(
                &inner.engine.inner,
                raw_tt,
                &module.inner.local_type_to_global_canon,
            );
            let fam = match fam_code {
                0 => GcRefFamily::Func,
                1 => GcRefFamily::Extern,
                2 => GcRefFamily::Any,
                _ => GcRefFamily::Exn,
            };
            let data = DynamicTableData::new(tt, 0, fam)?;
            let idx = inner.tables.len();
            inner.tables.push(data);
            table_indices.push(idx);
            table_desc_ptrs.push(inner.tables[idx].desc.as_mut() as *mut StoreTableDesc);
        }
        let table_desc_ptrs = table_desc_ptrs.into_boxed_slice();

        // 6. Create defined tags & assemble `tag_ids` (`0..total_tags`).
        let mut tag_indices: Vec<usize> =
            Vec::with_capacity(meta.imported_tags.len() + meta.defined_tags.len());
        let mut tag_ids: Vec<u32> =
            Vec::with_capacity(meta.imported_tags.len() + meta.defined_tags.len());
        for t in imp_tags {
            let tag = t.expect("validated imported tag");
            tag_indices.push(tag.index);
            tag_ids.push(inner.tags[tag.index].global_tag_id);
        }
        for &(_tag_i, type_idx) in &meta.defined_tags {
            let gcid = module.inner.local_type_to_global_canon[type_idx as usize];
            let ft = FuncType::from_canon(Arc::clone(&inner.engine.inner), gcid);
            let idx = inner.tags.len();
            let info = inner.alloc_tag(TagType::new(ft));
            tag_indices.push(idx);
            tag_ids.push(info.global_tag_id);
        }
        let tag_ids = tag_ids.into_boxed_slice();

        // 7. Build defined `StoreFuncRef` table (`0..n_defs`) & unified `func_ref_ptrs` (`0..n_imports + n_defs`).
        let n_imp_f = meta.n_imports as usize;
        let n_def_f = meta.n_defs as usize;
        let mut def_func_refs_vec = Vec::with_capacity(n_def_f);
        for def_i in 0..n_def_f {
            let fi = n_imp_f + def_i;
            let ti = meta.func_type_idx[fi];
            let gcid = module.inner.local_type_to_global_canon[ti as usize];
            def_func_refs_vec.push(StoreFuncRef {
                code_ptr: module.def_fn_ptr(def_i),
                ctx_ptr,
                canon_type_id: gcid,
                host_func_id: u32::MAX,
            });
        }
        let def_func_refs = def_func_refs_vec.into_boxed_slice();
        let mut func_ref_ptrs_vec = Vec::with_capacity(n_imp_f + n_def_f);
        for f in imp_funcs {
            func_ref_ptrs_vec.push(f.expect("validated imported func").func_ref_ptr);
        }
        for i in 0..n_def_f {
            func_ref_ptrs_vec.push(unsafe { def_func_refs.as_ptr().add(i) });
        }
        let func_ref_ptrs = func_ref_ptrs_vec.into_boxed_slice();

        // 8. Wire globals (`0..total_globals`): imported globals store `*mut u8` cell ptr in `Ctx`;
        //    defined globals live inline in `Ctx` at `off`.
        let mut global_ptrs: Vec<(*mut u8, GlobalType)> =
            Vec::with_capacity(meta.imported_globals.len() + meta.defined_globals.len());
        for ((_, _, _, _, off), g_opt) in meta.imported_globals.iter().zip(imp_globals) {
            let g = g_opt.expect("validated imported global");
            let (cell_ptr, gty) = inner.global_handles[g.index].clone();
            unsafe {
                *(ctx_ptr.offset(*off as isize).cast::<*mut u8>()) = cell_ptr;
            }
            global_ptrs.push((cell_ptr, gty));
        }
        let mut traced_defined_global_offs = Vec::new();
        for &(_gi, vt, mutable, off, is_traced) in &meta.defined_globals {
            let content = ValType::from_module_wasm(
                &inner.engine.inner,
                vt,
                &module.inner.local_type_to_global_canon,
            );
            let gt = GlobalType::new(
                content,
                if mutable {
                    Mutability::Var
                } else {
                    Mutability::Const
                },
            );
            let cell_ptr = if off >= 0 {
                if is_traced {
                    traced_defined_global_offs.push(off);
                }
                unsafe { ctx_ptr.offset(off as isize) }
            } else {
                std::ptr::null_mut()
            };
            global_ptrs.push((cell_ptr, gt));
        }

        // 9. Populate `Ctx` header (`OFF_MEM_BASE`, `OFF_MEM_RESERVE_BYTES`, `OFF_STACK_FLOOR`, `OFF_STORE_*`).
        let inst_id = inner.instances.len();
        let (mem0_base, mem0_desc_ptr) = if let Some(&p) = mem_desc_ptrs.first() {
            (unsafe { (*p).base as u64 }, p)
        } else {
            (0u64, (&ZERO_MEM_DESC as *const StoreMemDesc).cast_mut())
        };
        let store_state_ptr = std::ptr::addr_of_mut!(inner.state);
        unsafe {
            *(ctx_ptr.offset(OFF_MEM_BASE as isize).cast::<u64>()) = mem0_base;
            *(ctx_ptr
                .offset(OFF_MEM_RESERVE_BYTES as isize)
                .cast::<*mut StoreMemDesc>()) = mem0_desc_ptr;
            *(ctx_ptr
                .offset(OFF_STORE_PTR as isize)
                .cast::<*mut crate::abi::StoreState>()) = store_state_ptr;
            *(ctx_ptr
                .offset(OFF_STORE_CANON_MAP as isize)
                .cast::<*const u32>()) = module.inner.local_canon_to_global.as_ptr();
            *(ctx_ptr
                .offset(OFF_STORE_FUNC_REFS as isize)
                .cast::<*const *const StoreFuncRef>()) = func_ref_ptrs.as_ptr();
            *(ctx_ptr
                .offset(OFF_STORE_MEM_DESCS as isize)
                .cast::<*const *mut StoreMemDesc>()) = mem_desc_ptrs.as_ptr();
            *(ctx_ptr
                .offset(OFF_STORE_TAG_IDS as isize)
                .cast::<*const u32>()) = tag_ids.as_ptr();
            *(ctx_ptr
                .offset(OFF_STORE_ELEM_BUF_PTR as isize)
                .cast::<*mut u8>()) = elem_buf;
            *(ctx_ptr
                .offset(OFF_STORE_ELEM_BUF_BYTES as isize)
                .cast::<u64>()) = elem_buf_bytes as u64;
            *(ctx_ptr
                .offset(OFF_STORE_TABLE_DESCS as isize)
                .cast::<*const *mut StoreTableDesc>()) = table_desc_ptrs.as_ptr();
            *(ctx_ptr.offset(OFF_STORE_INSTANCE_ID as isize).cast::<u64>()) = inst_id as u64;
        }

        inner.instances.push(InstanceData {
            module: module.clone(),
            ctx_buf,
            elem_buf,
            elem_buf_bytes,
            _def_func_refs: def_func_refs,
            func_ref_ptrs,
            _mem_desc_ptrs: mem_desc_ptrs,
            _table_desc_ptrs: table_desc_ptrs,
            _tag_ids: tag_ids,
            mem_handles,
            table_indices,
            tag_indices,
            traced_defined_global_offs,
            global_ptrs,
        });

        // 10. Sync type descriptors & stack floors, then run `init_instance_fn(ctx_ptr)` inside `ActiveWasmFrame`!
        inner.consume_entry_fuel()?;
        inner.sync_type_descs();
        inner.stamp_stack_floors();
        inner.state.last_trap_code = 0;

        let init_fn = module.init_instance_fn();
        let prev_entry = inner.active_entry_instance;
        inner.active_entry_instance = Some(inst_id);
        let saved_stop = inner.state.gc_walk_stop_rsp;
        let saved_chain = inner.state.gc_stack_chain;
        let saved_init_ptr = inner.state.gc_init_roots_ptr;
        let saved_init_len = inner.state.gc_init_roots_len;
        let mut frame = ActiveWasmFrame {
            prev: std::ptr::null_mut(),
            jmp_buf: crate::signal::SigJmpBuf::default(),
            store_ptr: store_state_ptr,
            trap_code: 0,
        };
        let status: u32 = unsafe {
            push_active_frame(&mut frame);
            let rc = if sigsetjmp(std::ptr::addr_of_mut!(frame.jmp_buf), 0) == 0 {
                init_fn(ctx_ptr)
            } else {
                inner.state.gc_walk_stop_rsp = saved_stop;
                inner.state.gc_stack_chain = saved_chain;
                inner.state.gc_init_roots_ptr = saved_init_ptr;
                inner.state.gc_init_roots_len = saved_init_len;
                inner.interrupt.restore_all_rw();
                frame.trap_code
            };
            pop_active_frame(&mut frame);
            rc
        };
        inner.active_entry_instance = prev_entry;
        interpret_wasm_status(inner, status)?;

        Ok(Instance {
            store_id: inner.id,
            index: inst_id,
        })
    }

    pub fn get_export(&self, mut store: impl AsContextMut, name: &str) -> Option<Extern> {
        let inner = store.as_store_inner_mut();
        assert_eq!(inner.id, self.store_id, "Instance belongs to another Store");
        let inst = &inner.instances[self.index];
        let &(kind, idx) = inst.module.inner.export_index.get(name)?;
        self.extract_export(inner, kind, idx)
    }

    pub fn get_module_export(
        &self,
        mut store: impl AsContextMut,
        export: &ModuleExport,
    ) -> Option<Extern> {
        let inner = store.as_store_inner_mut();
        assert_eq!(inner.id, self.store_id, "Instance belongs to another Store");
        let inst = &inner.instances[self.index];
        if inst.module.inner.id != export.module_id {
            return None;
        }
        self.extract_export(inner, export.kind, export.index)
    }

    fn extract_export(
        &self,
        inner: &mut StoreInner,
        kind: ExportKind,
        idx: usize,
    ) -> Option<Extern> {
        let inst = &inner.instances[self.index];
        let meta = &inst.module.inner.meta;
        match kind {
            ExportKind::Func => {
                let &(_, fi, _) = meta.exported_funcs.get(idx)?;
                let ptr = inst.func_ref_ptrs[fi as usize];
                Some(Extern::Func(Func {
                    store_id: inner.id,
                    func_ref_ptr: ptr,
                }))
            }
            ExportKind::Global => {
                let &(_, gi, _, _, _) = meta.exported_globals.get(idx)?;
                let (cell_ptr, ty) = inst.global_ptrs[gi as usize].clone();
                let handle_idx = inner.global_handles.len();
                inner.global_handles.push((cell_ptr, ty));
                Some(Extern::Global(Global {
                    store_id: inner.id,
                    index: handle_idx,
                }))
            }
            ExportKind::Memory => {
                let &(_, mi) = meta.exported_mems.get(idx)?;
                match &inst.mem_handles[mi as usize] {
                    MemRef::Private(i) => Some(Extern::Memory(Memory {
                        store_id: inner.id,
                        index: *i,
                    })),
                    MemRef::Shared(sm) => Some(Extern::SharedMemory(sm.clone())),
                }
            }
            ExportKind::Table => {
                let &(_, ti) = meta.exported_tables.get(idx)?;
                let tbl_idx = inst.table_indices[ti as usize];
                Some(Extern::Table(Table {
                    store_id: inner.id,
                    index: tbl_idx,
                }))
            }
            ExportKind::Tag => {
                let &(_, tag_i) = meta.exported_tags.get(idx)?;
                let idx = inst.tag_indices[tag_i as usize];
                Some(Extern::Tag(Tag {
                    store_id: inner.id,
                    index: idx,
                }))
            }
        }
    }

    pub fn get_func(&self, store: impl AsContextMut, name: &str) -> Option<Func> {
        self.get_export(store, name).and_then(Extern::into_func)
    }

    pub fn get_typed_func<Params: WasmParams, Results: WasmResults>(
        &self,
        mut store: impl AsContextMut,
        name: &str,
    ) -> Result<TypedFunc<Params, Results>> {
        let func = self
            .get_func(&mut store, name)
            .ok_or_else(|| Error::msg(format!("failed to find function export `{name}`")))?;
        func.typed(&store)
    }

    pub fn get_memory(&self, store: impl AsContextMut, name: &str) -> Option<Memory> {
        self.get_export(store, name).and_then(Extern::into_memory)
    }

    pub fn get_shared_memory(&self, store: impl AsContextMut, name: &str) -> Option<SharedMemory> {
        self.get_export(store, name)
            .and_then(Extern::into_shared_memory)
    }

    pub fn get_global(&self, store: impl AsContextMut, name: &str) -> Option<Global> {
        self.get_export(store, name).and_then(Extern::into_global)
    }

    pub fn get_table(&self, store: impl AsContextMut, name: &str) -> Option<Table> {
        self.get_export(store, name).and_then(Extern::into_table)
    }

    pub fn get_tag(&self, store: impl AsContextMut, name: &str) -> Option<Tag> {
        self.get_export(store, name).and_then(Extern::into_tag)
    }
}

// =========================================================================
// `Linker<T>` and `InstancePre<T>`
// =========================================================================

static NEXT_LINKER_DEF_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Clone)]
struct LinkerEntry {
    def_id: u64,
    def: LinkerDef,
}

#[derive(Clone)]
enum LinkerDef {
    HostFunc {
        ty: FuncType,
        cb: ErasedHostCallback,
    },
    Extern(Extern),
}

pub struct Linker<T> {
    engine: Engine,
    allow_shadowing: bool,
    defs: HashMap<(String, String), LinkerEntry>,
    modules: HashMap<String, usize>,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Clone for Linker<T> {
    fn clone(&self) -> Self {
        Self {
            engine: self.engine.clone(),
            allow_shadowing: self.allow_shadowing,
            defs: self.defs.clone(),
            modules: self.modules.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T: 'static> Linker<T> {
    pub fn new(engine: &Engine) -> Self {
        Self {
            engine: engine.clone(),
            allow_shadowing: false,
            defs: HashMap::new(),
            modules: HashMap::new(),
            _marker: PhantomData,
        }
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn allow_shadowing(&mut self, allow: bool) -> &mut Self {
        self.allow_shadowing = allow;
        self
    }

    fn insert_def(&mut self, module: &str, name: &str, def: LinkerDef) -> Result<&mut Self> {
        let key = (module.to_string(), name.to_string());
        if !self.allow_shadowing && self.defs.contains_key(&key) {
            return Err(Error::msg(format!(
                "import of `{module}::{name}` defined twice"
            )));
        }
        let def_id = NEXT_LINKER_DEF_ID.fetch_add(1, Ordering::Relaxed);
        if self.defs.insert(key, LinkerEntry { def_id, def }).is_none() {
            *self.modules.entry(module.to_string()).or_insert(0) += 1;
        }
        Ok(self)
    }

    pub fn has_unshadowed_shared_definitions(&self, other: &Self) -> bool {
        if !Engine::same(&self.engine, &other.engine) {
            return false;
        }
        other.defs.iter().all(|(k, other_entry)| {
            self.defs
                .get(k)
                .is_some_and(|e| e.def_id == other_entry.def_id)
        })
    }

    pub fn try_extend_from_shared(&mut self, other: &Self) -> bool {
        if !Engine::same(&self.engine, &other.engine) {
            return false;
        }
        if !self.allow_shadowing && other.defs.keys().any(|k| self.defs.contains_key(k)) {
            return false;
        }
        for (k, v) in &other.defs {
            if self.defs.insert(k.clone(), v.clone()).is_none() {
                *self.modules.entry(k.0.clone()).or_insert(0) += 1;
            }
        }
        true
    }

    pub fn define(
        &mut self,
        _store: impl AsContext<Data = T>,
        module: &str,
        name: &str,
        item: impl Into<Extern>,
    ) -> Result<&mut Self> {
        self.insert_def(module, name, LinkerDef::Extern(item.into()))
    }

    pub fn define_shared_memory(
        &mut self,
        module: &str,
        name: &str,
        mem: SharedMemory,
    ) -> Result<&mut Self> {
        self.insert_def(module, name, LinkerDef::Extern(Extern::SharedMemory(mem)))
    }

    pub fn instance(
        &mut self,
        mut store: impl AsContextMut<Data = T>,
        module: &str,
        instance: Instance,
    ) -> Result<&mut Self> {
        let export_names: Vec<String> = {
            let inner = store.as_store_inner();
            if inner.id != instance.store_id {
                return Err(Error::msg("Instance belongs to another Store"));
            }
            inner.instances[instance.index]
                .module
                .inner
                .exports_cache
                .iter()
                .map(|(n, _)| n.clone())
                .collect()
        };
        for name in export_names {
            if let Some(ext) = instance.get_export(&mut store, &name) {
                self.insert_def(module, &name, LinkerDef::Extern(ext))?;
            }
        }
        Ok(self)
    }

    pub fn func_wrap<Params, Results>(
        &mut self,
        module: &str,
        name: &str,
        func: impl IntoFunc<T, Params, Results>,
    ) -> Result<&mut Self> {
        let (ty, cb) = func.into_erased(&self.engine);
        self.insert_def(module, name, LinkerDef::HostFunc { ty, cb })
    }

    pub fn func_new(
        &mut self,
        module: &str,
        name: &str,
        ty: FuncType,
        func: impl Fn(Caller<'_, T>, &[Val], &mut [Val]) -> Result<()> + Send + Sync + 'static,
    ) -> Result<&mut Self> {
        let cb = make_dynamic_host_callback::<T>(&ty, func);
        self.insert_def(module, name, LinkerDef::HostFunc { ty, cb })
    }

    /// # Safety
    /// The caller guarantees `func` reads/writes `ValRaw` elements matching `ty`.
    pub unsafe fn func_new_unchecked(
        &mut self,
        module: &str,
        name: &str,
        ty: FuncType,
        func: impl Fn(Caller<'_, T>, &mut [MaybeUninit<ValRaw>]) -> Result<()> + Send + Sync + 'static,
    ) -> Result<&mut Self> {
        let cb = make_unchecked_host_callback::<T>(&ty, func);
        self.insert_def(module, name, LinkerDef::HostFunc { ty, cb })
    }

    pub fn has_module(&self, module: &str) -> bool {
        self.modules.get(module).copied().unwrap_or(0) > 0
    }

    pub fn has_definition(&self, module: &str, name: &str) -> bool {
        self.defs
            .contains_key(&(module.to_string(), name.to_string()))
    }

    pub fn get(
        &self,
        mut store: impl AsContextMut<Data = T>,
        module: &str,
        name: &str,
    ) -> Result<Extern> {
        let Some(entry) = self.defs.get(&(module.to_string(), name.to_string())) else {
            return Err(Error::msg(format!("unknown import `{module}::{name}`")));
        };
        let inner = store.as_store_inner_mut();
        match &entry.def {
            LinkerDef::HostFunc { ty, cb } => {
                let f = register_host_func_in_store(inner, ty.clone(), Arc::clone(cb));
                Ok(Extern::Func(f))
            }
            LinkerDef::Extern(ext) => Ok(ext.clone()),
        }
    }

    pub fn instantiate(
        &self,
        mut store: impl AsContextMut<Data = T>,
        module: &Module,
    ) -> Result<Instance> {
        let mut resolved = Vec::with_capacity(module.inner.imports_cache.len());
        for (m, n, _ty) in &module.inner.imports_cache {
            resolved.push(self.get(&mut store, m, n)?);
        }
        Instance::new(store, module, &resolved)
    }

    pub fn instantiate_pre(&self, module: &Module) -> Result<InstancePre<T>> {
        let mut plan = Vec::with_capacity(module.inner.imports_cache.len());
        for (m, n, _ty) in &module.inner.imports_cache {
            let def = self
                .defs
                .get(&(m.clone(), n.clone()))
                .ok_or_else(|| Error::msg(format!("unknown import `{m}::{n}`")))?
                .def
                .clone();
            plan.push(PreImport::Linker(def));
        }
        Ok(InstancePre {
            module: module.clone(),
            plan,
            _marker: PhantomData,
        })
    }

    pub fn instantiate_pre_with_shared_memory(
        &self,
        module: &Module,
        shared_ns: &str,
        shared_name: &str,
    ) -> Result<InstancePre<T>> {
        let mut plan = Vec::with_capacity(module.inner.imports_cache.len());
        for (m, n, _ty) in &module.inner.imports_cache {
            if m == shared_ns && n == shared_name {
                plan.push(PreImport::SharedMemorySlot);
            } else {
                let def = self
                    .defs
                    .get(&(m.clone(), n.clone()))
                    .ok_or_else(|| Error::msg(format!("unknown import `{m}::{n}`")))?
                    .def
                    .clone();
                plan.push(PreImport::Linker(def));
            }
        }
        Ok(InstancePre {
            module: module.clone(),
            plan,
            _marker: PhantomData,
        })
    }

    pub fn instantiate_pre_with_module_exports(
        &self,
        program: &Module,
        _runtime: &Module,
        exports: &[Option<ModuleExport>],
        shared_import: Option<(&str, &str)>,
    ) -> Result<InstancePre<T>> {
        if exports.len() != program.inner.imports_cache.len() {
            return Err(Error::msg("module export import plan length mismatch"));
        }
        let mut plan = Vec::with_capacity(exports.len());
        for ((m, n, _ty), exp) in program.inner.imports_cache.iter().zip(exports.iter()) {
            if let Some(exp) = exp {
                plan.push(PreImport::RuntimeExport(*exp));
            } else if shared_import == Some((m.as_str(), n.as_str())) {
                plan.push(PreImport::SharedMemorySlot);
            } else {
                let def = self
                    .defs
                    .get(&(m.clone(), n.clone()))
                    .ok_or_else(|| Error::msg(format!("unknown import `{m}::{n}`")))?
                    .def
                    .clone();
                plan.push(PreImport::Linker(def));
            }
        }
        Ok(InstancePre {
            module: program.clone(),
            plan,
            _marker: PhantomData,
        })
    }
}

#[derive(Clone)]
enum PreImport {
    Linker(LinkerDef),
    SharedMemorySlot,
    RuntimeExport(ModuleExport),
}

pub struct InstancePre<T> {
    module: Module,
    plan: Vec<PreImport>,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Clone for InstancePre<T> {
    fn clone(&self) -> Self {
        Self {
            module: self.module.clone(),
            plan: self.plan.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T: 'static> InstancePre<T> {
    pub fn module(&self) -> &Module {
        &self.module
    }

    pub fn instantiate(&self, store: impl AsContextMut<Data = T>) -> Result<Instance> {
        self.instantiate_inner(store, None, None)
    }

    pub fn instantiate_with_shared_memory(
        &self,
        store: impl AsContextMut<Data = T>,
        shared: &SharedMemory,
    ) -> Result<Instance> {
        self.instantiate_inner(store, None, Some(shared))
    }

    pub fn instantiate_with_module_exports(
        &self,
        store: impl AsContextMut<Data = T>,
        runtime: Instance,
        shared: Option<&SharedMemory>,
    ) -> Result<Instance> {
        self.instantiate_inner(store, Some(runtime), shared)
    }

    fn instantiate_inner(
        &self,
        mut store: impl AsContextMut<Data = T>,
        runtime: Option<Instance>,
        shared: Option<&SharedMemory>,
    ) -> Result<Instance> {
        let mut resolved = Vec::with_capacity(self.plan.len());
        for entry in &self.plan {
            match entry {
                PreImport::Linker(def) => {
                    let inner = store.as_store_inner_mut();
                    match def {
                        LinkerDef::HostFunc { ty, cb } => {
                            let f = register_host_func_in_store(inner, ty.clone(), Arc::clone(cb));
                            resolved.push(Extern::Func(f));
                        }
                        LinkerDef::Extern(ext) => resolved.push(ext.clone()),
                    }
                }
                PreImport::SharedMemorySlot => {
                    let sm = shared
                        .ok_or_else(|| Error::msg("missing SharedMemory for prepared instance"))?;
                    resolved.push(Extern::SharedMemory(sm.clone()));
                }
                PreImport::RuntimeExport(exp) => {
                    let rt = runtime.ok_or_else(|| {
                        Error::msg("missing runtime Instance for prepared program")
                    })?;
                    let ext = rt
                        .get_module_export(&mut store, exp)
                        .ok_or_else(|| Error::msg("runtime export missing for prepared program"))?;
                    resolved.push(ext);
                }
            }
        }
        Instance::new(store, &self.module, &resolved)
    }
}

// =========================================================================
// Host Function Registration & Marshalling Helpers
// =========================================================================

fn register_host_func_in_store(
    inner: &mut StoreInner,
    ty: FuncType,
    cb: ErasedHostCallback,
) -> Func {
    let host_func_id = inner.host_funcs.len() as u32;
    let store_state_ptr = std::ptr::addr_of_mut!(inner.state);
    let mut host_ctx = Box::new(HostFuncCtx {
        mem0_base: 0,
        host_func_id: u64::from(host_func_id),
        store_ptr: store_state_ptr,
        mem0_desc_ptr: &ZERO_MEM_DESC as *const StoreMemDesc,
    });
    let ctx_ptr = (host_ctx.as_mut() as *mut HostFuncCtx).cast::<u8>();
    let code_ptr = inner
        .find_host_tramp_for_func_type(&ty)
        .unwrap_or(std::ptr::null());
    let func_ref = Box::new(StoreFuncRef {
        code_ptr,
        ctx_ptr,
        canon_type_id: ty.canon_id,
        host_func_id,
    });
    let func_ref_ptr = func_ref.as_ref() as *const StoreFuncRef;
    inner.host_funcs.push(HostFuncEntry {
        ty,
        cb,
        _host_ctx: host_ctx,
        func_ref,
    });
    Func {
        store_id: inner.id,
        func_ref_ptr,
    }
}

fn make_dynamic_host_callback<T: 'static>(
    ty: &FuncType,
    func: impl Fn(Caller<'_, T>, &[Val], &mut [Val]) -> Result<()> + Send + Sync + 'static,
) -> ErasedHostCallback {
    let (wparams, wresults) = ty.wasm_params_results();
    Arc::new(move |store_ptr, user_data, args_ptr, rets_ptr| {
        let inner = unsafe { &mut *store_ptr };
        let data = unsafe { &mut *(user_data as *mut T) };
        inner.gc.scope_stack.push(Vec::new());

        let mut in_vals = Vec::with_capacity(wparams.len());
        let mut cursor = 0usize;
        for &vt in &wparams {
            let n = if vt == wtime_types::ValType::V128 {
                2
            } else {
                1
            };
            let words = unsafe { std::slice::from_raw_parts(args_ptr.add(cursor), n) };
            cursor += n;
            in_vals.push(Val::from_abi_words(inner, vt, words)?);
        }
        let mut out_vals: Vec<Val> = wresults.iter().map(|&vt| default_val_for(vt)).collect();
        let caller = Caller { inner, data };
        let res = func(caller, &in_vals, &mut out_vals);

        let inner = unsafe { &mut *store_ptr };
        let write_res = if res.is_ok() {
            let mut rcur = 0usize;
            let mut r_ok = Ok(());
            for (val, &vt) in out_vals.iter().zip(&wresults) {
                let mut flat = Vec::with_capacity(2);
                if let Err(e) = pack_val_flat(inner, val, vt, &mut flat) {
                    r_ok = Err(e);
                    break;
                }
                for w in flat {
                    unsafe { *rets_ptr.add(rcur) = w };
                    rcur += 1;
                }
            }
            r_ok
        } else {
            Ok(())
        };

        if let Some(frame) = inner.gc.scope_stack.pop() {
            for (idx, gen_id) in frame {
                inner.gc.free_host_root(idx, gen_id);
            }
        }
        res?;
        write_res
    })
}

fn make_unchecked_host_callback<T: 'static>(
    ty: &FuncType,
    func: impl Fn(Caller<'_, T>, &mut [MaybeUninit<ValRaw>]) -> Result<()> + Send + Sync + 'static,
) -> ErasedHostCallback {
    let (wparams, wresults) = ty.wasm_params_results();
    let n_slots = wparams.len().max(wresults.len());
    Arc::new(move |store_ptr, user_data, args_ptr, rets_ptr| {
        let inner = unsafe { &mut *store_ptr };
        let data = unsafe { &mut *(user_data as *mut T) };
        let mut raw_buf: Vec<MaybeUninit<ValRaw>> = vec![MaybeUninit::uninit(); n_slots];
        let mut cursor = 0usize;
        for (i, &vt) in wparams.iter().enumerate() {
            let val_raw = match vt {
                wtime_types::ValType::I32 => {
                    let w = unsafe { *args_ptr.add(cursor) };
                    cursor += 1;
                    ValRaw::i32(w as i32)
                }
                wtime_types::ValType::I64 => {
                    let w = unsafe { *args_ptr.add(cursor) };
                    cursor += 1;
                    ValRaw::i64(w as i64)
                }
                wtime_types::ValType::F32 => {
                    let w = unsafe { *args_ptr.add(cursor) };
                    cursor += 1;
                    ValRaw::f32(w as u32)
                }
                wtime_types::ValType::F64 => {
                    let w = unsafe { *args_ptr.add(cursor) };
                    cursor += 1;
                    ValRaw::f64(w)
                }
                wtime_types::ValType::V128 => {
                    let lo = unsafe { *args_ptr.add(cursor) };
                    let hi = unsafe { *args_ptr.add(cursor + 1) };
                    cursor += 2;
                    ValRaw::v128(u128::from(lo) | (u128::from(hi) << 64))
                }
                wtime_types::ValType::Ref(_) => {
                    let w = unsafe { *args_ptr.add(cursor) };
                    cursor += 1;
                    ValRaw::anyref(inner.gc.abi_word_to_raw_u32(w))
                }
            };
            raw_buf[i].write(val_raw);
        }
        let caller = Caller { inner, data };
        func(caller, &mut raw_buf)?;

        let inner = unsafe { &mut *store_ptr };
        let mut rcur = 0usize;
        for (i, &vt) in wresults.iter().enumerate() {
            let rv = unsafe { raw_buf[i].assume_init() };
            match vt {
                wtime_types::ValType::I32 => {
                    unsafe { *rets_ptr.add(rcur) = u64::from(rv.get_i32() as u32) };
                    rcur += 1;
                }
                wtime_types::ValType::I64 => {
                    unsafe { *rets_ptr.add(rcur) = rv.get_i64() as u64 };
                    rcur += 1;
                }
                wtime_types::ValType::F32 => {
                    unsafe { *rets_ptr.add(rcur) = u64::from(rv.get_f32()) };
                    rcur += 1;
                }
                wtime_types::ValType::F64 => {
                    unsafe { *rets_ptr.add(rcur) = rv.get_f64() };
                    rcur += 1;
                }
                wtime_types::ValType::V128 => {
                    let v = rv.get_v128();
                    unsafe {
                        *rets_ptr.add(rcur) = v as u64;
                        *rets_ptr.add(rcur + 1) = (v >> 64) as u64;
                    }
                    rcur += 2;
                }
                wtime_types::ValType::Ref(_) => {
                    let w = inner.gc.raw_u32_to_abi_word(rv.get_anyref());
                    unsafe { *rets_ptr.add(rcur) = w };
                    rcur += 1;
                }
            }
        }
        Ok(())
    })
}

fn default_val_for(vt: wtime_types::ValType) -> Val {
    match vt {
        wtime_types::ValType::I32 => Val::I32(0),
        wtime_types::ValType::I64 => Val::I64(0),
        wtime_types::ValType::F32 => Val::F32(0),
        wtime_types::ValType::F64 => Val::F64(0),
        wtime_types::ValType::V128 => Val::V128(0),
        wtime_types::ValType::Ref(rt) => match rt.heap {
            wtime_types::HeapType::Extern | wtime_types::HeapType::NoExtern => Val::ExternRef(None),
            wtime_types::HeapType::Func | wtime_types::HeapType::NoFunc => Val::FuncRef(None),
            wtime_types::HeapType::Exn | wtime_types::HeapType::NoExn => Val::ExnRef(None),
            _ => Val::AnyRef(None),
        },
    }
}

fn pack_val_flat(
    inner: &StoreInner,
    val: &Val,
    vt: wtime_types::ValType,
    out: &mut Vec<u64>,
) -> Result<()> {
    if vt == wtime_types::ValType::V128 {
        let Val::V128(v) = *val else {
            return Err(Error::msg("expected V128 argument"));
        };
        out.push(v as u64);
        out.push((v >> 64) as u64);
    } else {
        out.push(val.to_abi_word(inner)?);
    }
    Ok(())
}

fn is_wasm_val_traced(inner: &StoreInner, vt: wtime_types::ValType) -> bool {
    match vt {
        wtime_types::ValType::Ref(rt) => match rt.heap {
            wtime_types::HeapType::Any
            | wtime_types::HeapType::Eq
            | wtime_types::HeapType::I31
            | wtime_types::HeapType::Struct
            | wtime_types::HeapType::Array
            | wtime_types::HeapType::None
            | wtime_types::HeapType::Extern
            | wtime_types::HeapType::NoExtern => true,
            wtime_types::HeapType::Defined(idx) | wtime_types::HeapType::ExactDefined(idx) => {
                let reg = inner
                    .engine
                    .inner
                    .types
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                !matches!(
                    reg.infos[idx.0 as usize].composite,
                    CanonComposite::Func { .. }
                )
            }
            _ => false,
        },
        _ => false,
    }
}

fn ref_family_of_reftype(inner: &StoreInner, rt: &crate::types::RefType) -> GcRefFamily {
    match rt.heap_type() {
        crate::types::HeapType::Func
        | crate::types::HeapType::NoFunc
        | crate::types::HeapType::ConcreteFunc(_) => GcRefFamily::Func,
        crate::types::HeapType::Extern | crate::types::HeapType::NoExtern => GcRefFamily::Extern,
        crate::types::HeapType::Exn | crate::types::HeapType::NoExn => GcRefFamily::Exn,
        _ => {
            let _ = inner;
            GcRefFamily::Any
        }
    }
}

fn write_val_to_cell(
    inner: &StoreInner,
    cell: *mut u8,
    vt: wtime_types::ValType,
    val: &Val,
) -> Result<()> {
    unsafe {
        match vt {
            wtime_types::ValType::I32 => {
                let Val::I32(v) = *val else {
                    return Err(Error::msg("global type mismatch: expected i32"));
                };
                *(cell as *mut i32) = v;
            }
            wtime_types::ValType::I64 => {
                let Val::I64(v) = *val else {
                    return Err(Error::msg("global type mismatch: expected i64"));
                };
                *(cell as *mut i64) = v;
            }
            wtime_types::ValType::F32 => {
                let Val::F32(b) = *val else {
                    return Err(Error::msg("global type mismatch: expected f32"));
                };
                *(cell as *mut u32) = b;
            }
            wtime_types::ValType::F64 => {
                let Val::F64(b) = *val else {
                    return Err(Error::msg("global type mismatch: expected f64"));
                };
                *(cell as *mut u64) = b;
            }
            wtime_types::ValType::V128 => {
                let Val::V128(v) = *val else {
                    return Err(Error::msg("global type mismatch: expected v128"));
                };
                *(cell as *mut u128) = v;
            }
            wtime_types::ValType::Ref(_) => {
                *(cell as *mut u64) = val.to_abi_word(inner)?;
            }
        }
    }
    Ok(())
}

unsafe fn read_val_from_cell(
    inner: &mut StoreInner,
    cell: *const u8,
    vt: wtime_types::ValType,
) -> Result<Val> {
    unsafe {
        if vt == wtime_types::ValType::V128 {
            let v = *(cell as *const u128);
            Ok(Val::V128(v))
        } else if matches!(vt, wtime_types::ValType::I32 | wtime_types::ValType::F32) {
            let w = u64::from(*(cell as *const u32));
            Val::from_abi_words(inner, vt, &[w])
        } else {
            let w = *(cell as *const u64);
            Val::from_abi_words(inner, vt, &[w])
        }
    }
}

// =========================================================================
// Typed Function Traits (`WasmTy`, `WasmParams`, `WasmResults`, `IntoFunc`)
// =========================================================================

pub trait WasmTy: Sized {
    fn val_type() -> ValType;
    fn check_wasm_type(vt: wtime_types::ValType) -> Result<()>;
    fn write_word(self, inner: &StoreInner, dst: &mut [u64], cursor: &mut usize) -> Result<()>;
    fn read_word(inner: &mut StoreInner, src: &[u64], cursor: &mut usize) -> Result<Self>;
}

impl WasmTy for i32 {
    fn val_type() -> ValType {
        ValType::I32
    }
    fn check_wasm_type(vt: wtime_types::ValType) -> Result<()> {
        if vt == wtime_types::ValType::I32 {
            Ok(())
        } else {
            Err(Error::msg("type mismatch: expected i32"))
        }
    }
    fn write_word(self, _inner: &StoreInner, dst: &mut [u64], cursor: &mut usize) -> Result<()> {
        dst[*cursor] = u64::from(self as u32);
        *cursor += 1;
        Ok(())
    }
    fn read_word(_inner: &mut StoreInner, src: &[u64], cursor: &mut usize) -> Result<Self> {
        let v = src[*cursor] as i32;
        *cursor += 1;
        Ok(v)
    }
}

impl WasmTy for u32 {
    fn val_type() -> ValType {
        ValType::I32
    }
    fn check_wasm_type(vt: wtime_types::ValType) -> Result<()> {
        i32::check_wasm_type(vt)
    }
    fn write_word(self, _inner: &StoreInner, dst: &mut [u64], cursor: &mut usize) -> Result<()> {
        dst[*cursor] = u64::from(self);
        *cursor += 1;
        Ok(())
    }
    fn read_word(_inner: &mut StoreInner, src: &[u64], cursor: &mut usize) -> Result<Self> {
        let v = src[*cursor] as u32;
        *cursor += 1;
        Ok(v)
    }
}

impl WasmTy for i64 {
    fn val_type() -> ValType {
        ValType::I64
    }
    fn check_wasm_type(vt: wtime_types::ValType) -> Result<()> {
        if vt == wtime_types::ValType::I64 {
            Ok(())
        } else {
            Err(Error::msg("type mismatch: expected i64"))
        }
    }
    fn write_word(self, _inner: &StoreInner, dst: &mut [u64], cursor: &mut usize) -> Result<()> {
        dst[*cursor] = self as u64;
        *cursor += 1;
        Ok(())
    }
    fn read_word(_inner: &mut StoreInner, src: &[u64], cursor: &mut usize) -> Result<Self> {
        let v = src[*cursor] as i64;
        *cursor += 1;
        Ok(v)
    }
}

impl WasmTy for u64 {
    fn val_type() -> ValType {
        ValType::I64
    }
    fn check_wasm_type(vt: wtime_types::ValType) -> Result<()> {
        i64::check_wasm_type(vt)
    }
    fn write_word(self, _inner: &StoreInner, dst: &mut [u64], cursor: &mut usize) -> Result<()> {
        dst[*cursor] = self;
        *cursor += 1;
        Ok(())
    }
    fn read_word(_inner: &mut StoreInner, src: &[u64], cursor: &mut usize) -> Result<Self> {
        let v = src[*cursor];
        *cursor += 1;
        Ok(v)
    }
}

impl WasmTy for f32 {
    fn val_type() -> ValType {
        ValType::F32
    }
    fn check_wasm_type(vt: wtime_types::ValType) -> Result<()> {
        if vt == wtime_types::ValType::F32 {
            Ok(())
        } else {
            Err(Error::msg("type mismatch: expected f32"))
        }
    }
    fn write_word(self, _inner: &StoreInner, dst: &mut [u64], cursor: &mut usize) -> Result<()> {
        dst[*cursor] = u64::from(self.to_bits());
        *cursor += 1;
        Ok(())
    }
    fn read_word(_inner: &mut StoreInner, src: &[u64], cursor: &mut usize) -> Result<Self> {
        let v = f32::from_bits(src[*cursor] as u32);
        *cursor += 1;
        Ok(v)
    }
}

impl WasmTy for f64 {
    fn val_type() -> ValType {
        ValType::F64
    }
    fn check_wasm_type(vt: wtime_types::ValType) -> Result<()> {
        if vt == wtime_types::ValType::F64 {
            Ok(())
        } else {
            Err(Error::msg("type mismatch: expected f64"))
        }
    }
    fn write_word(self, _inner: &StoreInner, dst: &mut [u64], cursor: &mut usize) -> Result<()> {
        dst[*cursor] = self.to_bits();
        *cursor += 1;
        Ok(())
    }
    fn read_word(_inner: &mut StoreInner, src: &[u64], cursor: &mut usize) -> Result<Self> {
        let v = f64::from_bits(src[*cursor]);
        *cursor += 1;
        Ok(v)
    }
}

macro_rules! impl_wasm_ty_rooted {
    ($ref_ty:ty, $val_ty:expr, $non_null_val_ty:expr, $is_ext:expr, $is_exn:expr) => {
        impl WasmTy for Option<Rooted<$ref_ty>> {
            fn val_type() -> ValType {
                $val_ty
            }
            fn check_wasm_type(vt: wtime_types::ValType) -> Result<()> {
                if matches!(vt, wtime_types::ValType::Ref(_)) {
                    Ok(())
                } else {
                    Err(Error::msg("type mismatch: expected nullable reference"))
                }
            }
            fn write_word(
                self,
                inner: &StoreInner,
                dst: &mut [u64],
                cursor: &mut usize,
            ) -> Result<()> {
                dst[*cursor] = match self {
                    Some(r) => r.read_abi_word(inner)?,
                    None => 0,
                };
                *cursor += 1;
                Ok(())
            }
            fn read_word(inner: &mut StoreInner, src: &[u64], cursor: &mut usize) -> Result<Self> {
                let w = src[*cursor];
                *cursor += 1;
                Ok(if w == 0 {
                    None
                } else {
                    Some(Rooted::<$ref_ty>::from_abi_word(inner, w, $is_ext, $is_exn))
                })
            }
        }

        impl WasmTy for Rooted<$ref_ty> {
            fn val_type() -> ValType {
                $non_null_val_ty
            }
            fn check_wasm_type(vt: wtime_types::ValType) -> Result<()> {
                if matches!(vt, wtime_types::ValType::Ref(_)) {
                    Ok(())
                } else {
                    Err(Error::msg("type mismatch: expected non-null reference"))
                }
            }
            fn write_word(
                self,
                inner: &StoreInner,
                dst: &mut [u64],
                cursor: &mut usize,
            ) -> Result<()> {
                let w = self.read_abi_word(inner)?;
                if w == 0 {
                    return Err(Error::msg("unexpected null for non-null reference"));
                }
                dst[*cursor] = w;
                *cursor += 1;
                Ok(())
            }
            fn read_word(inner: &mut StoreInner, src: &[u64], cursor: &mut usize) -> Result<Self> {
                let w = src[*cursor];
                *cursor += 1;
                if w == 0 {
                    return Err(Error::msg("unexpected null for non-null reference"));
                }
                Ok(Rooted::<$ref_ty>::from_abi_word(inner, w, $is_ext, $is_exn))
            }
        }
    };
}

impl_wasm_ty_rooted!(
    AnyRef,
    ValType::ANYREF,
    ValType::Ref(crate::types::RefType::new(
        false,
        crate::types::HeapType::Any
    )),
    false,
    false
);
impl_wasm_ty_rooted!(
    EqRef,
    ValType::EQREF,
    ValType::Ref(crate::types::RefType::new(
        false,
        crate::types::HeapType::Eq
    )),
    false,
    false
);
impl_wasm_ty_rooted!(
    StructRef,
    ValType::Ref(crate::types::RefType::new(
        true,
        crate::types::HeapType::Struct
    )),
    ValType::Ref(crate::types::RefType::new(
        false,
        crate::types::HeapType::Struct
    )),
    false,
    false
);
impl_wasm_ty_rooted!(
    ArrayRef,
    ValType::Ref(crate::types::RefType::new(
        true,
        crate::types::HeapType::Array
    )),
    ValType::Ref(crate::types::RefType::new(
        false,
        crate::types::HeapType::Array
    )),
    false,
    false
);
impl_wasm_ty_rooted!(
    ExternRef,
    ValType::EXTERNREF,
    ValType::Ref(crate::types::RefType::new(
        false,
        crate::types::HeapType::Extern
    )),
    true,
    false
);
impl_wasm_ty_rooted!(
    ExnRef,
    ValType::EXNREF,
    ValType::Ref(crate::types::RefType::new(
        false,
        crate::types::HeapType::Exn
    )),
    false,
    true
);

impl WasmTy for Option<Func> {
    fn val_type() -> ValType {
        ValType::FUNCREF
    }
    fn check_wasm_type(vt: wtime_types::ValType) -> Result<()> {
        if matches!(vt, wtime_types::ValType::Ref(_)) {
            Ok(())
        } else {
            Err(Error::msg("type mismatch: expected funcref"))
        }
    }
    fn write_word(self, inner: &StoreInner, dst: &mut [u64], cursor: &mut usize) -> Result<()> {
        dst[*cursor] = match self {
            Some(f) => {
                if f.store_id != inner.id {
                    return Err(Error::msg("Func belongs to another Store"));
                }
                f.func_ref_ptr as u64
            }
            None => 0,
        };
        *cursor += 1;
        Ok(())
    }
    fn read_word(inner: &mut StoreInner, src: &[u64], cursor: &mut usize) -> Result<Self> {
        let w = src[*cursor];
        *cursor += 1;
        Ok(if w == 0 {
            None
        } else {
            Some(Func::from_raw_ptr(inner.id, w as *const StoreFuncRef))
        })
    }
}

pub trait WasmParams: Sized {
    fn val_types() -> Vec<ValType>;
    fn check_types(tys: &[wtime_types::ValType]) -> Result<()>;
    fn write_flat(self, inner: &StoreInner, dst: &mut [u64], cursor: &mut usize) -> Result<()>;
    fn read_flat(inner: &mut StoreInner, src: &[u64], cursor: &mut usize) -> Result<Self>;
}

pub trait WasmResults: Sized {
    fn val_types() -> Vec<ValType>;
    fn check_types(tys: &[wtime_types::ValType]) -> Result<()>;
    fn write_flat(self, inner: &StoreInner, dst: &mut [u64], cursor: &mut usize) -> Result<()>;
    fn read_flat(inner: &mut StoreInner, src: &[u64], cursor: &mut usize) -> Result<Self>;
}

impl WasmParams for () {
    fn val_types() -> Vec<ValType> {
        Vec::new()
    }
    fn check_types(tys: &[wtime_types::ValType]) -> Result<()> {
        if tys.is_empty() {
            Ok(())
        } else {
            Err(Error::msg("expected 0 parameters"))
        }
    }
    fn write_flat(self, _inner: &StoreInner, _dst: &mut [u64], _cursor: &mut usize) -> Result<()> {
        Ok(())
    }
    fn read_flat(_inner: &mut StoreInner, _src: &[u64], _cursor: &mut usize) -> Result<Self> {
        Ok(())
    }
}

impl WasmResults for () {
    fn val_types() -> Vec<ValType> {
        Vec::new()
    }
    fn check_types(tys: &[wtime_types::ValType]) -> Result<()> {
        if tys.is_empty() {
            Ok(())
        } else {
            Err(Error::msg("expected 0 results"))
        }
    }
    fn write_flat(self, _inner: &StoreInner, _dst: &mut [u64], _cursor: &mut usize) -> Result<()> {
        Ok(())
    }
    fn read_flat(_inner: &mut StoreInner, _src: &[u64], _cursor: &mut usize) -> Result<Self> {
        Ok(())
    }
}

impl<T: WasmTy> WasmParams for T {
    fn val_types() -> Vec<ValType> {
        vec![T::val_type()]
    }
    fn check_types(tys: &[wtime_types::ValType]) -> Result<()> {
        if tys.len() != 1 {
            return Err(Error::msg("expected 1 parameter"));
        }
        T::check_wasm_type(tys[0])
    }
    fn write_flat(self, inner: &StoreInner, dst: &mut [u64], cursor: &mut usize) -> Result<()> {
        self.write_word(inner, dst, cursor)
    }
    fn read_flat(inner: &mut StoreInner, src: &[u64], cursor: &mut usize) -> Result<Self> {
        T::read_word(inner, src, cursor)
    }
}

impl<T: WasmTy> WasmResults for T {
    fn val_types() -> Vec<ValType> {
        vec![T::val_type()]
    }
    fn check_types(tys: &[wtime_types::ValType]) -> Result<()> {
        if tys.len() != 1 {
            return Err(Error::msg("expected 1 result"));
        }
        T::check_wasm_type(tys[0])
    }
    fn write_flat(self, inner: &StoreInner, dst: &mut [u64], cursor: &mut usize) -> Result<()> {
        self.write_word(inner, dst, cursor)
    }
    fn read_flat(inner: &mut StoreInner, src: &[u64], cursor: &mut usize) -> Result<Self> {
        T::read_word(inner, src, cursor)
    }
}

macro_rules! impl_wasm_tuples {
    ($($t:ident : $idx:tt),+) => {
        impl<$($t: WasmTy),+> WasmParams for ($($t,)+) {
            fn val_types() -> Vec<ValType> {
                vec![$($t::val_type()),+]
            }
            fn check_types(tys: &[wtime_types::ValType]) -> Result<()> {
                let expected = [$($idx),+].len();
                if tys.len() != expected {
                    return Err(Error::msg(format!("expected {expected} parameters, got {}", tys.len())));
                }
                $($t::check_wasm_type(tys[$idx])?;)+
                Ok(())
            }
            fn write_flat(self, inner: &StoreInner, dst: &mut [u64], cursor: &mut usize) -> Result<()> {
                $(self.$idx.write_word(inner, dst, cursor)?;)+
                Ok(())
            }
            fn read_flat(inner: &mut StoreInner, src: &[u64], cursor: &mut usize) -> Result<Self> {
                Ok(($($t::read_word(inner, src, cursor)?,)+))
            }
        }

        impl<$($t: WasmTy),+> WasmResults for ($($t,)+) {
            fn val_types() -> Vec<ValType> {
                vec![$($t::val_type()),+]
            }
            fn check_types(tys: &[wtime_types::ValType]) -> Result<()> {
                let expected = [$($idx),+].len();
                if tys.len() != expected {
                    return Err(Error::msg(format!("expected {expected} results, got {}", tys.len())));
                }
                $($t::check_wasm_type(tys[$idx])?;)+
                Ok(())
            }
            fn write_flat(self, inner: &StoreInner, dst: &mut [u64], cursor: &mut usize) -> Result<()> {
                $(self.$idx.write_word(inner, dst, cursor)?;)+
                Ok(())
            }
            fn read_flat(inner: &mut StoreInner, src: &[u64], cursor: &mut usize) -> Result<Self> {
                Ok(($($t::read_word(inner, src, cursor)?,)+))
            }
        }
    };
}

impl_wasm_tuples!(A: 0);
impl_wasm_tuples!(A: 0, B: 1);
impl_wasm_tuples!(A: 0, B: 1, C: 2);
impl_wasm_tuples!(A: 0, B: 1, C: 2, D: 3);
impl_wasm_tuples!(A: 0, B: 1, C: 2, D: 3, E: 4);
impl_wasm_tuples!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5);
impl_wasm_tuples!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6);
impl_wasm_tuples!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7);
impl_wasm_tuples!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8);
impl_wasm_tuples!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9);

/// Conversion trait for host callback return values (`R` or `Result<R>`).
pub trait WasmRet {
    type Results: WasmResults;
    fn into_results(self) -> Result<Self::Results>;
}

impl<R: WasmResults> WasmRet for R {
    type Results = R;
    fn into_results(self) -> Result<R> {
        Ok(self)
    }
}

impl<R: WasmResults> WasmRet for Result<R> {
    type Results = R;
    fn into_results(self) -> Result<R> {
        self
    }
}

/// Trait implemented by Rust closures accepted by `Linker::func_wrap` and `Func::wrap`.
pub trait IntoFunc<T, Params, Results>: Send + Sync + 'static {
    fn into_erased(self, engine: &Engine) -> (FuncType, ErasedHostCallback);
}

macro_rules! impl_into_func {
    ($($p:ident),*) => {
        // Without `Caller<'_, T>`:
        impl<T, F, $($p,)* R> IntoFunc<T, ($($p,)*), R> for F
        where
            T: 'static,
            F: Fn($($p),*) -> R + Send + Sync + 'static,
            $($p: WasmTy,)*
            R: WasmRet,
        {
            #[allow(non_snake_case, unused_variables, unused_mut)]
            fn into_erased(self, engine: &Engine) -> (FuncType, ErasedHostCallback) {
                let ft = FuncType::new(
                    engine,
                    vec![$($p::val_type()),*],
                    R::Results::val_types(),
                );
                let n_params = ft.params().len();
                let n_results = ft.results().len();
                let cb: ErasedHostCallback = Arc::new(move |store_ptr, _user_data, args_ptr, rets_ptr| {
                    let inner = unsafe { &mut *store_ptr };
                    inner.gc.scope_stack.push(Vec::new());
                    let args_slice = if n_params == 0 {
                        &[]
                    } else {
                        unsafe { std::slice::from_raw_parts(args_ptr, n_params) }
                    };
                    let mut ac = 0usize;
                    $(let $p = $p::read_word(inner, args_slice, &mut ac)?;)*
                    let res = (self)($($p),*).into_results();
                    let inner = unsafe { &mut *store_ptr };
                    let write_res = match res {
                        Ok(ret_val) => {
                            let rets_slice = if n_results == 0 {
                                &mut []
                            } else {
                                unsafe { std::slice::from_raw_parts_mut(rets_ptr, n_results) }
                            };
                            let mut rc = 0usize;
                            ret_val.write_flat(inner, rets_slice, &mut rc)
                        }
                        Err(e) => Err(e),
                    };
                    if let Some(frame) = inner.gc.scope_stack.pop() {
                        for (idx, gen_id) in frame {
                            inner.gc.free_host_root(idx, gen_id);
                        }
                    }
                    write_res
                });
                (ft, cb)
            }
        }

        // With `Caller<'_, T>` first argument:
        impl<T, F, $($p,)* R> IntoFunc<T, (Caller<'_, T>, $($p,)*), R> for F
        where
            T: 'static,
            F: Fn(Caller<'_, T>, $($p),*) -> R + Send + Sync + 'static,
            $($p: WasmTy,)*
            R: WasmRet,
        {
            #[allow(non_snake_case, unused_variables, unused_mut)]
            fn into_erased(self, engine: &Engine) -> (FuncType, ErasedHostCallback) {
                let ft = FuncType::new(
                    engine,
                    vec![$($p::val_type()),*],
                    R::Results::val_types(),
                );
                let n_params = ft.params().len();
                let n_results = ft.results().len();
                let cb: ErasedHostCallback = Arc::new(move |store_ptr, user_data, args_ptr, rets_ptr| {
                    let inner = unsafe { &mut *store_ptr };
                    let data = unsafe { &mut *(user_data as *mut T) };
                    inner.gc.scope_stack.push(Vec::new());
                    let args_slice = if n_params == 0 {
                        &[]
                    } else {
                        unsafe { std::slice::from_raw_parts(args_ptr, n_params) }
                    };
                    let mut ac = 0usize;
                    $(let $p = $p::read_word(inner, args_slice, &mut ac)?;)*
                    let caller = Caller { inner, data };
                    let res = (self)(caller, $($p),*).into_results();
                    let inner = unsafe { &mut *store_ptr };
                    let write_res = match res {
                        Ok(ret_val) => {
                            let rets_slice = if n_results == 0 {
                                &mut []
                            } else {
                                unsafe { std::slice::from_raw_parts_mut(rets_ptr, n_results) }
                            };
                            let mut rc = 0usize;
                            ret_val.write_flat(inner, rets_slice, &mut rc)
                        }
                        Err(e) => Err(e),
                    };
                    if let Some(frame) = inner.gc.scope_stack.pop() {
                        for (idx, gen_id) in frame {
                            inner.gc.free_host_root(idx, gen_id);
                        }
                    }
                    write_res
                });
                (ft, cb)
            }
        }
    };
}

impl_into_func!();
impl_into_func!(P0);
impl_into_func!(P0, P1);
impl_into_func!(P0, P1, P2);
impl_into_func!(P0, P1, P2, P3);
impl_into_func!(P0, P1, P2, P3, P4);
impl_into_func!(P0, P1, P2, P3, P4, P5);
impl_into_func!(P0, P1, P2, P3, P4, P5, P6);
impl_into_func!(P0, P1, P2, P3, P4, P5, P6, P7);
impl_into_func!(P0, P1, P2, P3, P4, P5, P6, P7, P8);
impl_into_func!(P0, P1, P2, P3, P4, P5, P6, P7, P8, P9);

// Keep `ty_to_code` / `val_type_to_machine_ty` import clean if unused in release builds.
const _: fn(wtime_types::ValType) -> u8 = |v| ty_to_code(val_type_to_machine_ty(v));
