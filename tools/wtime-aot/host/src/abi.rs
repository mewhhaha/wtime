//! Exact `#[repr(C)]` binary ABI shared with `wtime-aot --backend x64` (`StoreCdylib`).
//!
//! Every field offset is statically verified against the compiler's own `OFF_STORE_*`,
//! `STORE_OFF_*`, and `GC_OFF_*` constants so any drift fails at compile time.

use std::mem::offset_of;

use wtime_aot::x64::gc_rt::{
    GC_OFF_ALLOC_LIMIT, GC_OFF_ALLOC_PTR, GC_OFF_ELEM_BUF_BYTES, GC_OFF_ELEM_BUF_PTR,
    GC_OFF_FROM_BASE, GC_OFF_FROM_CAP, GC_OFF_GC_COUNT, GC_OFF_HEAP_CAP, GC_OFF_INIT_ROOTS_LEN,
    GC_OFF_INIT_ROOTS_PTR, GC_OFF_STACK_CHAIN, GC_OFF_TO_BASE, GC_OFF_TO_CAP, GC_OFF_WALK_STOP_RSP,
    TYPE_DESC_BYTES,
};
use wtime_aot::x64::module::GC_DISPLAY_LEN;
use wtime_aot::x64::{
    STORE_OFF_ATOMIC_NOTIFY_FN, STORE_OFF_ATOMIC_WAIT_FN, STORE_OFF_EXN_ACTIVE,
    STORE_OFF_EXN_ALLOC_FN, STORE_OFF_GC_COLLECT_FN, STORE_OFF_HOST_CALL_FN,
    STORE_OFF_LAST_TRAP_CODE, STORE_OFF_MEM_GROW_FN, STORE_OFF_RET_SPILL_PTR,
    STORE_OFF_TABLE_GROW_FN, STORE_OFF_TYPE_DESCS_LEN, STORE_OFF_TYPE_DESCS_PTR, STORE_STATE_BYTES,
};

pub const STORE_ABI_VERSION: u32 = wtime_aot::x64::store_rt::STORE_ABI_VERSION;

/// Tag placed in the high 32 bits of opaque host `externref` handles (`"WTEX"`).
/// Distinct from `GC_EXT_I31_TAG_HI` (`0x5754_4933_0000_0000`) and non-8-byte aligned
/// (`low bits == 0b10`) so `any.convert_extern` immediately boxes it into `InternalExternBox`.
pub const HOST_EXTERN_TAG_HI: u64 = 0x5754_4558_0000_0000;
pub const HOST_EXTERN_TAG_MASK: u64 = 0xffff_ffff_0000_0000;

/// Helper to pack a 0-based `extern_arena` slot index + generation into a host `externref` word.
#[inline]
pub const fn encode_host_extern_raw(slot_idx: u32) -> u64 {
    HOST_EXTERN_TAG_HI | ((slot_idx as u64) << 3) | 2
}

#[inline]
pub const fn decode_host_extern_slot(raw: u64) -> Option<u32> {
    if (raw & HOST_EXTERN_TAG_MASK) == HOST_EXTERN_TAG_HI && (raw & 7) == 2 {
        Some(((raw & 0xffff_ffff) >> 3) as u32)
    } else {
        None
    }
}

/// Filled by the single ELF export `wt_store_get_desc(out: *mut StoreModuleDesc)` (96 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct StoreModuleDesc {
    pub abi_version: u32,
    pub flags: u32,
    pub ctx_total_bytes: u64,
    pub code_start: usize,
    pub code_end: usize,
    pub safepoint_header: *const SafepointHeader,
    pub rodata_base: *const u8,
    pub metadata_ptr: *const u8,
    pub metadata_len: usize,
    pub init_instance_fn: *const u8,
    pub def_fn_table: *const i32,
    pub host_tramp_table: *const i32,
    pub export_tramp_table: *const i32,
}

const _: () = assert!(std::mem::size_of::<StoreModuleDesc>() == 96);

/// Shared `StoreState` (`208` bytes), pointed to by `Ctx + OFF_STORE_PTR (144)`.
/// `+0..+112` matches `gc_rt::GcState` byte-for-byte.
#[repr(C)]
pub struct StoreState {
    pub gc_alloc_ptr: u64,
    pub gc_alloc_limit: u64,
    pub gc_from_base: u64,
    pub gc_from_cap: u64,
    pub gc_to_base: u64,
    pub gc_to_cap: u64,
    pub gc_heap_cap: u64,
    pub gc_count: u64,
    pub gc_walk_stop_rsp: u64,
    pub gc_stack_chain: u64,
    pub gc_init_roots_ptr: u64,
    pub gc_init_roots_len: u64,
    pub gc_elem_buf_ptr: u64,
    pub gc_elem_buf_bytes: u64,
    pub type_descs_ptr: *const StoreTypeDesc,
    pub type_descs_len: u64,
    pub exn_active_ptr: u64,
    pub gc_collect_fn: extern "C" fn(*mut StoreState, u64, u64) -> u32,
    pub mem_grow_fn: extern "C" fn(*mut StoreState, *mut StoreMemDesc, u64) -> u64,
    pub atomic_wait_fn: extern "C" fn(*mut StoreState, *const u8, u64, i64, u32) -> i32,
    pub atomic_notify_fn: extern "C" fn(*mut StoreState, *const u8, u32) -> u32,
    pub exn_alloc_fn: extern "C" fn(*mut StoreState, u32, u32) -> *mut u8,
    pub table_grow_fn: extern "C" fn(*mut StoreState, *mut StoreTableDesc, u64, u64) -> u64,
    pub host_call_fn: extern "C" fn(*mut StoreState, u32, *const u64, *mut u64) -> u32,
    pub ret_spill_ptr: *mut u8,
    pub last_trap_code: u32,
    pub _pad: u32,
}

const _: () = {
    assert!(std::mem::size_of::<StoreState>() == STORE_STATE_BYTES);
    assert!(offset_of!(StoreState, gc_alloc_ptr) == GC_OFF_ALLOC_PTR as usize);
    assert!(offset_of!(StoreState, gc_alloc_limit) == GC_OFF_ALLOC_LIMIT as usize);
    assert!(offset_of!(StoreState, gc_from_base) == GC_OFF_FROM_BASE as usize);
    assert!(offset_of!(StoreState, gc_from_cap) == GC_OFF_FROM_CAP as usize);
    assert!(offset_of!(StoreState, gc_to_base) == GC_OFF_TO_BASE as usize);
    assert!(offset_of!(StoreState, gc_to_cap) == GC_OFF_TO_CAP as usize);
    assert!(offset_of!(StoreState, gc_heap_cap) == GC_OFF_HEAP_CAP as usize);
    assert!(offset_of!(StoreState, gc_count) == GC_OFF_GC_COUNT as usize);
    assert!(offset_of!(StoreState, gc_walk_stop_rsp) == GC_OFF_WALK_STOP_RSP as usize);
    assert!(offset_of!(StoreState, gc_stack_chain) == GC_OFF_STACK_CHAIN as usize);
    assert!(offset_of!(StoreState, gc_init_roots_ptr) == GC_OFF_INIT_ROOTS_PTR as usize);
    assert!(offset_of!(StoreState, gc_init_roots_len) == GC_OFF_INIT_ROOTS_LEN as usize);
    assert!(offset_of!(StoreState, gc_elem_buf_ptr) == GC_OFF_ELEM_BUF_PTR as usize);
    assert!(offset_of!(StoreState, gc_elem_buf_bytes) == GC_OFF_ELEM_BUF_BYTES as usize);
    assert!(offset_of!(StoreState, type_descs_ptr) == STORE_OFF_TYPE_DESCS_PTR as usize);
    assert!(offset_of!(StoreState, type_descs_len) == STORE_OFF_TYPE_DESCS_LEN as usize);
    assert!(offset_of!(StoreState, exn_active_ptr) == STORE_OFF_EXN_ACTIVE as usize);
    assert!(offset_of!(StoreState, gc_collect_fn) == STORE_OFF_GC_COLLECT_FN as usize);
    assert!(offset_of!(StoreState, mem_grow_fn) == STORE_OFF_MEM_GROW_FN as usize);
    assert!(offset_of!(StoreState, atomic_wait_fn) == STORE_OFF_ATOMIC_WAIT_FN as usize);
    assert!(offset_of!(StoreState, atomic_notify_fn) == STORE_OFF_ATOMIC_NOTIFY_FN as usize);
    assert!(offset_of!(StoreState, exn_alloc_fn) == STORE_OFF_EXN_ALLOC_FN as usize);
    assert!(offset_of!(StoreState, table_grow_fn) == STORE_OFF_TABLE_GROW_FN as usize);
    assert!(offset_of!(StoreState, host_call_fn) == STORE_OFF_HOST_CALL_FN as usize);
    assert!(offset_of!(StoreState, ret_spill_ptr) == STORE_OFF_RET_SPILL_PTR as usize);
    assert!(offset_of!(StoreState, last_trap_code) == STORE_OFF_LAST_TRAP_CODE as usize);
};

/// 64-byte global canonical type descriptor read directly by Wasm `ref.test`/`ref.cast`/`call_indirect`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct StoreTypeDesc {
    /// `0 = Func`, `1 = Struct`, `2 = Array`
    pub kind: u8,
    /// For arrays: `GcRefKind` (`0 = Untraced`, `1 = Gc`, `2 = Extern`).
    pub elem_ref_kind: u8,
    pub is_final: u8,
    pub _pad: u8,
    /// Struct total aligned byte size OR array element size in bytes.
    pub size_or_elem_sz: u32,
    pub depth: u32,
    pub num_traced_fields: u32,
    pub display: [u32; GC_DISPLAY_LEN],
    /// Pointer to full `[u32; depth + 1]` supertype array when `depth >= GC_DISPLAY_LEN`, else null.
    pub supertypes_all_ptr: *const u32,
}

const _: () = {
    assert!(std::mem::size_of::<StoreTypeDesc>() == TYPE_DESC_BYTES);
    assert!(offset_of!(StoreTypeDesc, depth) == 8);
    assert!(offset_of!(StoreTypeDesc, display) == 16);
    assert!(offset_of!(StoreTypeDesc, supertypes_all_ptr) == 56);
};

// SAFETY: `StoreTypeDesc` points only to immutable heap-owned supertype arrays kept alive by the registry.
unsafe impl Send for StoreTypeDesc {}
unsafe impl Sync for StoreTypeDesc {}

/// 32-byte linear memory descriptor shared across modules/threads.
#[repr(C)]
#[derive(Debug)]
pub struct StoreMemDesc {
    pub base: *mut u8,
    pub len_bytes: std::sync::atomic::AtomicU64,
    pub max_pages: u64,
    /// Bit 0 = `is_shared`, Bit 1 = `is_mem64`.
    pub flags: u64,
}

const _: () = assert!(std::mem::size_of::<StoreMemDesc>() == 32);

// SAFETY: `StoreMemDesc` base pointer is fixed for the mapping lifetime and `len_bytes` is atomic.
unsafe impl Send for StoreMemDesc {}
unsafe impl Sync for StoreMemDesc {}

/// Sentinel zero-length `StoreMemDesc` used by `HostFuncCtx` and zero-memory modules so
/// `[R15 + OFF_MEM_RESERVE_BYTES]` is ALWAYS a valid readable pointer.
pub static ZERO_MEM_DESC: StoreMemDesc = StoreMemDesc {
    base: std::ptr::null_mut(),
    len_bytes: std::sync::atomic::AtomicU64::new(0),
    max_pages: 0,
    flags: 0,
};

/// 24-byte dynamic table descriptor shared across modules.
#[repr(C)]
#[derive(Debug)]
pub struct StoreTableDesc {
    pub ptr: *mut u64,
    pub cur_len: u32,
    pub max_len: u32,
    pub cap_slots: u64,
}

const _: () = assert!(std::mem::size_of::<StoreTableDesc>() == 24);

/// 24-byte unified cross-module function reference (`funcref` representation in `StoreCdylib`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct StoreFuncRef {
    pub code_ptr: *const u8,
    pub ctx_ptr: *mut u8,
    pub canon_type_id: u32,
    /// `u32::MAX` for Wasm functions; host callback slot index for host functions.
    pub host_func_id: u32,
}

const _: () = assert!(std::mem::size_of::<StoreFuncRef>() == 24);

/// 32-byte synthetic `Ctx` for host function trampolines (`StoreFuncRef::ctx_ptr` when `host_func_id != u32::MAX`).
#[repr(C)]
pub struct HostFuncCtx {
    pub mem0_base: u64,
    pub host_func_id: u64,
    pub store_ptr: *mut StoreState,
    pub mem0_desc_ptr: *const StoreMemDesc,
}

const _: () = assert!(std::mem::size_of::<HostFuncCtx>() == 32);

/// 24-byte stack-allocated reentrancy chain node pushed by `emit_store_host_trampoline`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GcStackChainNode {
    pub prev_chain: u64,
    pub outer_stop_rsp: u64,
    pub outer_paused_ret_rsp: u64,
}

const _: () = assert!(std::mem::size_of::<GcStackChainNode>() == 24);

/// 16-byte header of `.rodata` sorted return-address safepoint table.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SafepointHeader {
    pub num_entries: u32,
    pub _pad: u32,
    pub entries_rel_off: i32,
    pub slots_rel_off: i32,
}

/// 16-byte sorted safepoint entry inside `.rodata`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SafepointEntry {
    pub ret_pc_rel: u32,
    pub frame_caller_ret_off: u32,
    pub first_slot_idx: u32,
    pub num_slots: u32,
}

/// Type of the `wt_store_get_desc` ELF symbol.
pub type WtStoreGetDescFn = unsafe extern "C" fn(*mut StoreModuleDesc);

/// Type of the `HELPER_INITIALIZE` (`init_instance_fn`) entry point.
pub type WtStoreInitInstanceFn = unsafe extern "C" fn(*mut u8) -> u32;

/// Type of every `export_tramp_table[idx]` trampoline.
pub type WtStoreExportTrampFn =
    unsafe extern "C" fn(*mut u8, *const u8, *const u64, *mut u64) -> u32;

/// Custom host-side trap sentinel written to `StoreState::last_trap_code` when a host callback
/// returns `Err(e)` (so the outer Rust boundary recovers the exact `Error` without losing its type).
pub const TRAP_HOST_STORED_ERROR: u32 = 0x7fff_0001;
/// Custom trap sentinel for `Trap::Interrupt`.
pub const TRAP_INTERRUPT_CODE: u32 = 0x7fff_0002;
/// Custom host-side trap sentinel when a Rust host callback panics across the `extern "C"` Wasm boundary;
/// `invoke_store_func_raw` / `Instance::new` takes `StoreInner::pending_host_panic` and calls `resume_unwind`.
pub const TRAP_HOST_PANIC: u32 = 0x7fff_0003;
