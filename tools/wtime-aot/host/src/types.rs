//! Cross-module iso-recursive `CanonicalTypeRegistry` and public Wasm type descriptors.

use std::collections::HashMap;
use std::sync::Arc;

use wtime_aot::x64::module::{GC_DISPLAY_LEN, GcStorageKind, MAX_GC_SUBTYPE_DEPTH};
use wtime_types::{
    CompositeType, FieldType as WFieldType, FuncType as WFuncType, HeapType as WHeapType,
    RefType as WRefType, StorageType as WStorageType, SubType, TypeGroup, TypeIdx,
    ValType as WValType,
};

use crate::abi::StoreTypeDesc;
use crate::config::{Engine, EngineInner};
use crate::error::{Error, Result};

/// Maximum number of canonical types per `Engine` (4 MiB virtual slab, demand-paged).
const MAX_CANON_TYPES: usize = 65_536;

/// Physical layout metadata for one field of a canonical GC struct.
#[derive(Clone, Debug)]
pub struct CanonStructField {
    pub offset: u32,
    pub storage: GcStorageKind,
    pub mutable: bool,
    pub wasm_storage: WStorageType,
}

/// Complete host-side descriptor for a single canonical Wasm type.
#[derive(Clone, Debug)]
pub enum CanonComposite {
    Func {
        params: Vec<WValType>,
        results: Vec<WValType>,
    },
    Struct {
        fields: Vec<CanonStructField>,
        total_bytes: u32,
        gc_ref_offs: Vec<u32>,
        ext_ref_offs: Vec<u32>,
    },
    Array {
        elem: GcStorageKind,
        mutable: bool,
        wasm_storage: WStorageType,
    },
}

/// Host-side canonical type record aligned 1:1 with `StoreTypeDesc[canon_id]`.
#[derive(Clone, Debug)]
pub struct CanonTypeInfo {
    pub canon_id: u32,
    pub is_final: bool,
    pub supertype: Option<u32>,
    pub depth: u32,
    pub display: [u32; GC_DISPLAY_LEN],
    /// Heap-owned stable slice for `depth >= GC_DISPLAY_LEN` (pointer embedded in `StoreTypeDesc`).
    pub supertypes_all: Box<[u32]>,
    pub composite: CanonComposite,
}

/// Engine-wide canonical type registry shared across all `Module`s and `Store`s.
///
/// `StoreTypeDesc` entries live in an immutable-address demand-paged virtual slab so
/// `StoreState::type_descs_ptr` NEVER reallocates even when new modules are dynamically
/// compiled during active Wasm execution.
pub struct CanonicalTypeRegistry {
    group_to_canon: HashMap<Vec<SubType>, Vec<u32>>,
    pub(crate) infos: Vec<CanonTypeInfo>,
    /// Fixed virtual address slab of `MAX_CANON_TYPES` `StoreTypeDesc` slots (`4 MiB`).
    slab_ptr: *mut StoreTypeDesc,
    slab_bytes: usize,
}

// SAFETY: `slab_ptr` is owned exclusively by `CanonicalTypeRegistry` behind `EngineInner::types` Mutex,
// and published `StoreTypeDesc` entries are write-once immutable once `len` advances.
unsafe impl Send for CanonicalTypeRegistry {}
unsafe impl Sync for CanonicalTypeRegistry {}

impl Drop for CanonicalTypeRegistry {
    fn drop(&mut self) {
        if !self.slab_ptr.is_null() {
            // SAFETY: `slab_ptr` came from `libc::mmap` with length `slab_bytes`.
            unsafe {
                let _ = libc::munmap(self.slab_ptr.cast(), self.slab_bytes);
            }
        }
    }
}

impl CanonicalTypeRegistry {
    pub fn new() -> Self {
        let slab_bytes = MAX_CANON_TYPES * std::mem::size_of::<StoreTypeDesc>();
        // SAFETY: Anonymous private RW mapping; Linux commits physical pages lazily on demand.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                slab_bytes,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        assert!(
            ptr != libc::MAP_FAILED,
            "wtime-aot-host: failed to mmap CanonicalTypeRegistry slab"
        );
        Self {
            group_to_canon: HashMap::new(),
            infos: Vec::new(),
            slab_ptr: ptr.cast::<StoreTypeDesc>(),
            slab_bytes,
        }
    }

    #[inline]
    pub fn descs_ptr(&self) -> *const StoreTypeDesc {
        self.slab_ptr.cast_const()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.infos.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.infos.is_empty()
    }

    #[inline]
    pub fn info(&self, canon_id: u32) -> Option<&CanonTypeInfo> {
        self.infos.get(canon_id as usize)
    }

    /// Returns `true` iff `sub_cid <: sup_cid` according to Wasm GC subtyping.
    #[inline]
    pub fn is_subtype(&self, sub_cid: u32, sup_cid: u32) -> bool {
        if sub_cid == sup_cid {
            return true;
        }
        let (Some(sub), Some(sup)) = (self.info(sub_cid), self.info(sup_cid)) else {
            return false;
        };
        let d = sup.depth as usize;
        if (sub.depth as usize) < d {
            return false;
        }
        if d < GC_DISPLAY_LEN {
            sub.display[d] == sup_cid
        } else {
            sub.supertypes_all.get(d).copied() == Some(sup_cid)
        }
    }

    /// Registers a module's `raw_type_groups` and returns `module_type_idx -> global_canon_id`.
    pub fn register_groups(&mut self, groups: &[TypeGroup]) -> Result<Vec<u32>> {
        let total_types: usize = groups.iter().map(|g| g.types.len()).sum();
        let mut type_canon = Vec::with_capacity(total_types);

        for group in groups {
            let base = type_canon.len();
            let end = base + group.types.len();
            let mut normalized = group.types.clone();
            for st in &mut normalized {
                rewrite_subtype_indices(st, base, end, &type_canon)?;
            }
            if let Some(existing) = self.group_to_canon.get(&normalized) {
                type_canon.extend_from_slice(existing);
                continue;
            }
            let first = self.infos.len() as u32;
            let count = group.types.len();
            if self.infos.len() + count > MAX_CANON_TYPES {
                return Err(Error::msg(
                    "wtime-aot-host: CanonicalTypeRegistry exceeded 65,536 canonical types",
                ));
            }
            let ids: Vec<u32> = (0..count as u32).map(|i| first + i).collect();

            // First determine composite kind of each newly allocated type in this rec group so
            // mutually recursive struct/array fields resolve `GcStorageKind` accurately.
            let mut local_kinds: Vec<u8> = Vec::with_capacity(count);
            for st in &normalized {
                local_kinds.push(match &st.composite {
                    CompositeType::Func(_) => 0,
                    CompositeType::Struct(_) => 1,
                    CompositeType::Array(_) => 2,
                    CompositeType::Cont(_) => {
                        return Err(Error::msg("continuation types are not supported"));
                    }
                });
            }

            let resolve_norm = |idx: TypeIdx| -> u32 {
                if (idx.0 & 0x8000_0000) != 0 {
                    first + (idx.0 & 0x7fff_ffff)
                } else {
                    idx.0
                }
            };

            let kind_of_global = |cid: u32, infos: &[CanonTypeInfo]| -> u8 {
                if cid >= first {
                    local_kinds[(cid - first) as usize]
                } else {
                    match &infos[cid as usize].composite {
                        CanonComposite::Func { .. } => 0,
                        CanonComposite::Struct { .. } => 1,
                        CanonComposite::Array { .. } => 2,
                    }
                }
            };

            let resolve_vt = |vt: WValType| -> WValType {
                match vt {
                    WValType::Ref(rt) => WValType::Ref(WRefType {
                        nullable: rt.nullable,
                        heap: match rt.heap {
                            WHeapType::Defined(idx) | WHeapType::ExactDefined(idx) => {
                                WHeapType::Defined(TypeIdx(resolve_norm(idx)))
                            }
                            other => other,
                        },
                    }),
                    other => other,
                }
            };

            let resolve_storage = |st: WStorageType| -> WStorageType {
                match st {
                    WStorageType::Val(vt) => WStorageType::Val(resolve_vt(vt)),
                    other => other,
                }
            };

            let storage_gc_kind = |st: WStorageType, infos: &[CanonTypeInfo]| -> GcStorageKind {
                match st {
                    WStorageType::I8 => GcStorageKind::I8,
                    WStorageType::I16 => GcStorageKind::I16,
                    WStorageType::Val(vt) => match vt {
                        WValType::I32 => GcStorageKind::I32,
                        WValType::I64 => GcStorageKind::I64,
                        WValType::F32 => GcStorageKind::F32,
                        WValType::F64 => GcStorageKind::F64,
                        WValType::V128 => GcStorageKind::V128,
                        WValType::Ref(rt) => match rt.heap {
                            WHeapType::Any
                            | WHeapType::Eq
                            | WHeapType::I31
                            | WHeapType::Struct
                            | WHeapType::Array
                            | WHeapType::None => GcStorageKind::GcRef,
                            WHeapType::Extern | WHeapType::NoExtern => GcStorageKind::ExternRef,
                            WHeapType::Func
                            | WHeapType::NoFunc
                            | WHeapType::Exn
                            | WHeapType::NoExn
                            | WHeapType::Cont
                            | WHeapType::NoCont => GcStorageKind::OtherRef,
                            WHeapType::Defined(idx) | WHeapType::ExactDefined(idx) => {
                                if kind_of_global(idx.0, infos) == 0 {
                                    GcStorageKind::OtherRef
                                } else {
                                    GcStorageKind::GcRef
                                }
                            }
                        },
                    },
                }
            };

            for (local_i, norm_st) in normalized.iter().enumerate() {
                let cid = first + local_i as u32;
                let super_cid = norm_st.supertype.map(resolve_norm);
                let composite = match &norm_st.composite {
                    CompositeType::Func(ft) => CanonComposite::Func {
                        params: ft.params.iter().copied().map(resolve_vt).collect(),
                        results: ft.results.iter().copied().map(resolve_vt).collect(),
                    },
                    CompositeType::Struct(s) => {
                        let mut off: u32 = 8;
                        let mut fields = Vec::with_capacity(s.fields.len());
                        let mut gc_ref_offs = Vec::new();
                        let mut ext_ref_offs = Vec::new();
                        for f in &s.fields {
                            let wst = resolve_storage(f.storage);
                            let sk = storage_gc_kind(wst, &self.infos);
                            let align = sk.align();
                            off = off
                                .checked_add(align - 1)
                                .map(|v| v & !(align - 1))
                                .ok_or_else(|| Error::msg("GC struct offset overflow"))?;
                            if sk == GcStorageKind::GcRef {
                                gc_ref_offs.push(off);
                            } else if sk == GcStorageKind::ExternRef {
                                ext_ref_offs.push(off);
                            }
                            fields.push(CanonStructField {
                                offset: off,
                                storage: sk,
                                mutable: f.mutable,
                                wasm_storage: wst,
                            });
                            off = off
                                .checked_add(sk.byte_size())
                                .ok_or_else(|| Error::msg("GC struct size overflow"))?;
                        }
                        let total_bytes = off
                            .checked_add(7)
                            .map(|v| (v & !7).max(8))
                            .ok_or_else(|| Error::msg("GC struct size overflow"))?;
                        CanonComposite::Struct {
                            fields,
                            total_bytes,
                            gc_ref_offs,
                            ext_ref_offs,
                        }
                    }
                    CompositeType::Array(a) => {
                        let wst = resolve_storage(a.field.storage);
                        let elem = storage_gc_kind(wst, &self.infos);
                        CanonComposite::Array {
                            elem,
                            mutable: a.field.mutable,
                            wasm_storage: wst,
                        }
                    }
                    CompositeType::Cont(_) => unreachable!(),
                };
                self.infos.push(CanonTypeInfo {
                    canon_id: cid,
                    is_final: norm_st.final_,
                    supertype: super_cid,
                    depth: 0,
                    display: [u32::MAX; GC_DISPLAY_LEN],
                    supertypes_all: Box::new([]),
                    composite,
                });
            }

            // Compute supertype displays and write immutable 64-byte `StoreTypeDesc` into slab.
            for local_i in 0..count {
                let cid = first + local_i as u32;
                let mut rev = Vec::new();
                let mut cur = Some(cid);
                while let Some(id) = cur {
                    if rev.len() > MAX_GC_SUBTYPE_DEPTH {
                        return Err(Error::msg("GC supertype depth exceeds limit"));
                    }
                    rev.push(id);
                    cur = self.infos[id as usize].supertype;
                }
                rev.reverse();
                let depth = (rev.len() - 1) as u32;
                let mut display = [u32::MAX; GC_DISPLAY_LEN];
                for (d, &anc) in rev.iter().enumerate().take(GC_DISPLAY_LEN) {
                    display[d] = anc;
                }
                let supertypes_all: Box<[u32]> = if rev.len() > GC_DISPLAY_LEN {
                    rev.into_boxed_slice()
                } else {
                    Box::new([])
                };
                let supertypes_all_ptr = if supertypes_all.is_empty() {
                    std::ptr::null()
                } else {
                    supertypes_all.as_ptr()
                };
                let info = &mut self.infos[cid as usize];
                info.depth = depth;
                info.display = display;
                info.supertypes_all = supertypes_all;

                let (kind, elem_ref_kind, size_or_elem_sz, num_traced_fields) =
                    match &info.composite {
                        CanonComposite::Func { .. } => (0u8, 0u8, 0u32, 0u32),
                        CanonComposite::Struct {
                            total_bytes,
                            gc_ref_offs,
                            ext_ref_offs,
                            ..
                        } => (
                            1u8,
                            0u8,
                            *total_bytes,
                            (gc_ref_offs.len() + ext_ref_offs.len()) as u32,
                        ),
                        CanonComposite::Array { elem, .. } => {
                            let rk = match elem {
                                GcStorageKind::GcRef => 1u8,
                                GcStorageKind::ExternRef => 2u8,
                                _ => 0u8,
                            };
                            (2u8, rk, elem.byte_size(), 0u32)
                        }
                    };
                let desc = StoreTypeDesc {
                    kind,
                    elem_ref_kind,
                    is_final: u8::from(info.is_final),
                    _pad: 0,
                    size_or_elem_sz,
                    depth,
                    num_traced_fields,
                    display,
                    supertypes_all_ptr,
                };
                // SAFETY: `cid < MAX_CANON_TYPES`; write-once slot in `slab_ptr`.
                unsafe {
                    self.slab_ptr.add(cid as usize).write(desc);
                }
            }

            self.group_to_canon.insert(normalized, ids.clone());
            type_canon.extend_from_slice(&ids);
        }

        Ok(type_canon)
    }

    /// Registers a single standalone non-recursive type (used by `FuncType::new`, `StructType::new`, `ArrayType::new`).
    pub fn register_single_final(&mut self, composite: CompositeType) -> Result<u32> {
        // Standalone host types already reference global canonical IDs directly in `TypeIdx`.
        // We normalize directly against `self.group_to_canon` without shifting indices.
        let st = SubType {
            final_: true,
            supertype: None,
            composite,
            descriptor: None,
            describes: None,
        };
        let key = vec![st.clone()];
        if let Some(existing) = self.group_to_canon.get(&key) {
            return Ok(existing[0]);
        }
        let cid = self.infos.len() as u32;
        if self.infos.len() >= MAX_CANON_TYPES {
            return Err(Error::msg(
                "wtime-aot-host: CanonicalTypeRegistry exceeded 65,536 canonical types",
            ));
        }
        let kind_of_global = |id: u32, infos: &[CanonTypeInfo]| -> u8 {
            match &infos[id as usize].composite {
                CanonComposite::Func { .. } => 0,
                CanonComposite::Struct { .. } => 1,
                CanonComposite::Array { .. } => 2,
            }
        };
        let storage_gc_kind = |st: WStorageType, infos: &[CanonTypeInfo]| -> GcStorageKind {
            match st {
                WStorageType::I8 => GcStorageKind::I8,
                WStorageType::I16 => GcStorageKind::I16,
                WStorageType::Val(vt) => match vt {
                    WValType::I32 => GcStorageKind::I32,
                    WValType::I64 => GcStorageKind::I64,
                    WValType::F32 => GcStorageKind::F32,
                    WValType::F64 => GcStorageKind::F64,
                    WValType::V128 => GcStorageKind::V128,
                    WValType::Ref(rt) => match rt.heap {
                        WHeapType::Any
                        | WHeapType::Eq
                        | WHeapType::I31
                        | WHeapType::Struct
                        | WHeapType::Array
                        | WHeapType::None => GcStorageKind::GcRef,
                        WHeapType::Extern | WHeapType::NoExtern => GcStorageKind::ExternRef,
                        WHeapType::Func
                        | WHeapType::NoFunc
                        | WHeapType::Exn
                        | WHeapType::NoExn
                        | WHeapType::Cont
                        | WHeapType::NoCont => GcStorageKind::OtherRef,
                        WHeapType::Defined(idx) | WHeapType::ExactDefined(idx) => {
                            if kind_of_global(idx.0, infos) == 0 {
                                GcStorageKind::OtherRef
                            } else {
                                GcStorageKind::GcRef
                            }
                        }
                    },
                },
            }
        };
        let composite_info = match &st.composite {
            CompositeType::Func(ft) => CanonComposite::Func {
                params: ft.params.clone(),
                results: ft.results.clone(),
            },
            CompositeType::Struct(s) => {
                let mut off: u32 = 8;
                let mut fields = Vec::with_capacity(s.fields.len());
                let mut gc_ref_offs = Vec::new();
                let mut ext_ref_offs = Vec::new();
                for f in &s.fields {
                    let sk = storage_gc_kind(f.storage, &self.infos);
                    let align = sk.align();
                    off = (off + align - 1) & !(align - 1);
                    if sk == GcStorageKind::GcRef {
                        gc_ref_offs.push(off);
                    } else if sk == GcStorageKind::ExternRef {
                        ext_ref_offs.push(off);
                    }
                    fields.push(CanonStructField {
                        offset: off,
                        storage: sk,
                        mutable: f.mutable,
                        wasm_storage: f.storage,
                    });
                    off += sk.byte_size();
                }
                let total_bytes = ((off + 7) & !7).max(8);
                CanonComposite::Struct {
                    fields,
                    total_bytes,
                    gc_ref_offs,
                    ext_ref_offs,
                }
            }
            CompositeType::Array(a) => {
                let elem = storage_gc_kind(a.field.storage, &self.infos);
                CanonComposite::Array {
                    elem,
                    mutable: a.field.mutable,
                    wasm_storage: a.field.storage,
                }
            }
            CompositeType::Cont(_) => {
                return Err(Error::msg("continuation types are not supported"));
            }
        };
        let mut display = [u32::MAX; GC_DISPLAY_LEN];
        display[0] = cid;
        let (kind, elem_ref_kind, size_or_elem_sz, num_traced_fields) = match &composite_info {
            CanonComposite::Func { .. } => (0u8, 0u8, 0u32, 0u32),
            CanonComposite::Struct {
                total_bytes,
                gc_ref_offs,
                ext_ref_offs,
                ..
            } => (
                1u8,
                0u8,
                *total_bytes,
                (gc_ref_offs.len() + ext_ref_offs.len()) as u32,
            ),
            CanonComposite::Array { elem, .. } => {
                let rk = match elem {
                    GcStorageKind::GcRef => 1u8,
                    GcStorageKind::ExternRef => 2u8,
                    _ => 0u8,
                };
                (2u8, rk, elem.byte_size(), 0u32)
            }
        };
        self.infos.push(CanonTypeInfo {
            canon_id: cid,
            is_final: true,
            supertype: None,
            depth: 0,
            display,
            supertypes_all: Box::new([]),
            composite: composite_info,
        });
        let desc = StoreTypeDesc {
            kind,
            elem_ref_kind,
            is_final: 1,
            _pad: 0,
            size_or_elem_sz,
            depth: 0,
            num_traced_fields,
            display,
            supertypes_all_ptr: std::ptr::null(),
        };
        // SAFETY: `cid < MAX_CANON_TYPES`.
        unsafe {
            self.slab_ptr.add(cid as usize).write(desc);
        }
        self.group_to_canon.insert(key, vec![cid]);
        Ok(cid)
    }
}

impl Default for CanonicalTypeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn rewrite_type_idx(idx: TypeIdx, base: usize, end: usize, canon: &[u32]) -> Result<TypeIdx> {
    let raw = idx.0 as usize;
    if raw >= base && raw < end {
        Ok(TypeIdx(0x8000_0000 | ((raw - base) as u32)))
    } else if raw < base {
        Ok(TypeIdx(canon[raw]))
    } else {
        Err(Error::msg(format!(
            "forward type reference {raw} outside rec group [{base}..{end})"
        )))
    }
}

fn rewrite_val_type(vt: &mut WValType, base: usize, end: usize, canon: &[u32]) -> Result<()> {
    if let WValType::Ref(rt) = vt {
        match &mut rt.heap {
            WHeapType::Defined(idx) | WHeapType::ExactDefined(idx) => {
                *idx = rewrite_type_idx(*idx, base, end, canon)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn rewrite_storage_type(
    st: &mut WStorageType,
    base: usize,
    end: usize,
    canon: &[u32],
) -> Result<()> {
    if let WStorageType::Val(vt) = st {
        rewrite_val_type(vt, base, end, canon)?;
    }
    Ok(())
}

fn rewrite_subtype_indices(st: &mut SubType, base: usize, end: usize, canon: &[u32]) -> Result<()> {
    if let Some(sup) = &mut st.supertype {
        *sup = rewrite_type_idx(*sup, base, end, canon)?;
    }
    match &mut st.composite {
        CompositeType::Func(ft) => {
            for p in &mut ft.params {
                rewrite_val_type(p, base, end, canon)?;
            }
            for r in &mut ft.results {
                rewrite_val_type(r, base, end, canon)?;
            }
        }
        CompositeType::Struct(s) => {
            for f in &mut s.fields {
                rewrite_storage_type(&mut f.storage, base, end, canon)?;
            }
        }
        CompositeType::Array(a) => {
            rewrite_storage_type(&mut a.field.storage, base, end, canon)?;
        }
        CompositeType::Cont(c) => {
            c.function = rewrite_type_idx(c.function, base, end, canon)?;
        }
    }
    Ok(())
}

// =========================================================================
// Public Wasmtime-compatible Type Surface
// =========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mutability {
    Const,
    Var,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageType {
    I8,
    I16,
    ValType(ValType),
}

impl StorageType {
    pub fn as_val_type(&self) -> Option<&ValType> {
        match self {
            Self::ValType(vt) => Some(vt),
            _ => None,
        }
    }

    pub fn unwrap_val_type(&self) -> &ValType {
        self.as_val_type()
            .expect("StorageType::unwrap_val_type on packed integer type")
    }

    pub(crate) fn to_wasm(&self) -> WStorageType {
        match self {
            Self::I8 => WStorageType::I8,
            Self::I16 => WStorageType::I16,
            Self::ValType(vt) => WStorageType::Val(vt.to_wasm()),
        }
    }

    pub(crate) fn from_wasm_canon(engine: &Arc<EngineInner>, st: WStorageType) -> Self {
        match st {
            WStorageType::I8 => Self::I8,
            WStorageType::I16 => Self::I16,
            WStorageType::Val(vt) => Self::ValType(ValType::from_wasm_canon(engine, vt)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldType {
    mutability: Mutability,
    element_type: StorageType,
}

impl FieldType {
    pub fn new(mutability: Mutability, element_type: StorageType) -> Self {
        Self {
            mutability,
            element_type,
        }
    }

    pub fn mutability(&self) -> Mutability {
        self.mutability
    }

    pub fn element_type(&self) -> &StorageType {
        &self.element_type
    }
}

#[derive(Clone, Debug)]
pub struct FuncType {
    pub(crate) engine: Arc<EngineInner>,
    pub(crate) canon_id: u32,
}

impl PartialEq for FuncType {
    fn eq(&self, other: &Self) -> bool {
        self.engine.id == other.engine.id && self.canon_id == other.canon_id
    }
}
impl Eq for FuncType {}

impl FuncType {
    pub fn new(
        engine: &Engine,
        params: impl IntoIterator<Item = ValType>,
        results: impl IntoIterator<Item = ValType>,
    ) -> Self {
        let wparams: Vec<WValType> = params.into_iter().map(|v| v.to_wasm()).collect();
        let wresults: Vec<WValType> = results.into_iter().map(|v| v.to_wasm()).collect();
        let mut reg = engine
            .inner
            .types
            .write()
            .unwrap_or_else(|e| e.into_inner());
        let cid = reg
            .register_single_final(CompositeType::Func(WFuncType {
                params: wparams,
                results: wresults,
            }))
            .expect("FuncType::new canonical registration");
        engine.inner.publish_types_snapshot(&reg);
        Self {
            engine: Arc::clone(&engine.inner),
            canon_id: cid,
        }
    }

    pub(crate) fn from_canon(engine: Arc<EngineInner>, canon_id: u32) -> Self {
        Self { engine, canon_id }
    }

    pub fn canon_id(&self) -> u32 {
        self.canon_id
    }

    #[allow(clippy::should_implement_trait)]
    pub fn eq(a: &Self, b: &Self) -> bool {
        a == b
    }

    pub fn matches(&self, other: &Self) -> bool {
        if self.engine.id != other.engine.id {
            return false;
        }
        let reg = self.engine.types.read().unwrap_or_else(|e| e.into_inner());
        reg.is_subtype(self.canon_id, other.canon_id)
    }

    pub fn params(&self) -> std::vec::IntoIter<ValType> {
        let reg = self.engine.types.read().unwrap_or_else(|e| e.into_inner());
        let vts = match &reg.infos[self.canon_id as usize].composite {
            CanonComposite::Func { params, .. } => params.clone(),
            _ => Vec::new(),
        };
        drop(reg);
        vts.into_iter()
            .map(|v| ValType::from_wasm_canon(&self.engine, v))
            .collect::<Vec<_>>()
            .into_iter()
    }

    pub fn results(&self) -> std::vec::IntoIter<ValType> {
        let reg = self.engine.types.read().unwrap_or_else(|e| e.into_inner());
        let vts = match &reg.infos[self.canon_id as usize].composite {
            CanonComposite::Func { results, .. } => results.clone(),
            _ => Vec::new(),
        };
        drop(reg);
        vts.into_iter()
            .map(|v| ValType::from_wasm_canon(&self.engine, v))
            .collect::<Vec<_>>()
            .into_iter()
    }

    pub(crate) fn wasm_params_results(&self) -> (Vec<WValType>, Vec<WValType>) {
        let reg = self.engine.types.read().unwrap_or_else(|e| e.into_inner());
        match &reg.infos[self.canon_id as usize].composite {
            CanonComposite::Func { params, results } => (params.clone(), results.clone()),
            _ => (Vec::new(), Vec::new()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct StructType {
    pub(crate) engine: Arc<EngineInner>,
    pub(crate) canon_id: u32,
}

impl PartialEq for StructType {
    fn eq(&self, other: &Self) -> bool {
        self.engine.id == other.engine.id && self.canon_id == other.canon_id
    }
}
impl Eq for StructType {}

impl StructType {
    pub fn new(engine: &Engine, fields: impl IntoIterator<Item = FieldType>) -> Result<Self> {
        let wfields: Vec<WFieldType> = fields
            .into_iter()
            .map(|f| WFieldType {
                storage: f.element_type.to_wasm(),
                mutable: f.mutability == Mutability::Var,
            })
            .collect();
        let mut reg = engine
            .inner
            .types
            .write()
            .unwrap_or_else(|e| e.into_inner());
        let cid = reg.register_single_final(CompositeType::Struct(wtime_types::StructType {
            fields: wfields,
        }))?;
        engine.inner.publish_types_snapshot(&reg);
        Ok(Self {
            engine: Arc::clone(&engine.inner),
            canon_id: cid,
        })
    }

    pub(crate) fn from_canon(engine: Arc<EngineInner>, canon_id: u32) -> Self {
        Self { engine, canon_id }
    }

    pub fn canon_id(&self) -> u32 {
        self.canon_id
    }

    #[allow(clippy::should_implement_trait)]
    pub fn eq(a: &Self, b: &Self) -> bool {
        a == b
    }

    pub fn matches(&self, other: &Self) -> bool {
        if self.engine.id != other.engine.id {
            return false;
        }
        let reg = self.engine.types.read().unwrap_or_else(|e| e.into_inner());
        reg.is_subtype(self.canon_id, other.canon_id)
    }

    pub fn fields(&self) -> std::vec::IntoIter<FieldType> {
        let reg = self.engine.types.read().unwrap_or_else(|e| e.into_inner());
        let raw: Vec<(bool, WStorageType)> = match &reg.infos[self.canon_id as usize].composite {
            CanonComposite::Struct { fields, .. } => {
                fields.iter().map(|f| (f.mutable, f.wasm_storage)).collect()
            }
            _ => Vec::new(),
        };
        drop(reg);
        raw.into_iter()
            .map(|(m, s)| {
                FieldType::new(
                    if m {
                        Mutability::Var
                    } else {
                        Mutability::Const
                    },
                    StorageType::from_wasm_canon(&self.engine, s),
                )
            })
            .collect::<Vec<_>>()
            .into_iter()
    }

    pub fn field(&self, index: usize) -> Option<FieldType> {
        let reg = self.engine.types.read().unwrap_or_else(|e| e.into_inner());
        let entry = match &reg.infos[self.canon_id as usize].composite {
            CanonComposite::Struct { fields, .. } => {
                fields.get(index).map(|f| (f.mutable, f.wasm_storage))
            }
            _ => None,
        };
        drop(reg);
        entry.map(|(m, s)| {
            FieldType::new(
                if m {
                    Mutability::Var
                } else {
                    Mutability::Const
                },
                StorageType::from_wasm_canon(&self.engine, s),
            )
        })
    }
}

#[derive(Clone, Debug)]
pub struct ArrayType {
    pub(crate) engine: Arc<EngineInner>,
    pub(crate) canon_id: u32,
}

impl PartialEq for ArrayType {
    fn eq(&self, other: &Self) -> bool {
        self.engine.id == other.engine.id && self.canon_id == other.canon_id
    }
}
impl Eq for ArrayType {}

impl ArrayType {
    pub fn new(engine: &Engine, field: FieldType) -> Self {
        let wf = WFieldType {
            storage: field.element_type.to_wasm(),
            mutable: field.mutability == Mutability::Var,
        };
        let mut reg = engine
            .inner
            .types
            .write()
            .unwrap_or_else(|e| e.into_inner());
        let cid = reg
            .register_single_final(CompositeType::Array(wtime_types::ArrayType { field: wf }))
            .expect("ArrayType::new canonical registration");
        engine.inner.publish_types_snapshot(&reg);
        Self {
            engine: Arc::clone(&engine.inner),
            canon_id: cid,
        }
    }

    pub(crate) fn from_canon(engine: Arc<EngineInner>, canon_id: u32) -> Self {
        Self { engine, canon_id }
    }

    pub fn canon_id(&self) -> u32 {
        self.canon_id
    }

    #[allow(clippy::should_implement_trait)]
    pub fn eq(a: &Self, b: &Self) -> bool {
        a == b
    }

    pub fn matches(&self, other: &Self) -> bool {
        if self.engine.id != other.engine.id {
            return false;
        }
        let reg = self.engine.types.read().unwrap_or_else(|e| e.into_inner());
        reg.is_subtype(self.canon_id, other.canon_id)
    }

    pub fn mutability(&self) -> Mutability {
        let reg = self.engine.types.read().unwrap_or_else(|e| e.into_inner());
        match &reg.infos[self.canon_id as usize].composite {
            CanonComposite::Array { mutable, .. } => {
                if *mutable {
                    Mutability::Var
                } else {
                    Mutability::Const
                }
            }
            _ => Mutability::Const,
        }
    }

    pub fn element_type(&self) -> StorageType {
        let reg = self.engine.types.read().unwrap_or_else(|e| e.into_inner());
        let st = match &reg.infos[self.canon_id as usize].composite {
            CanonComposite::Array { wasm_storage, .. } => *wasm_storage,
            _ => WStorageType::I8,
        };
        drop(reg);
        StorageType::from_wasm_canon(&self.engine, st)
    }

    pub fn field_type(&self) -> FieldType {
        FieldType::new(self.mutability(), self.element_type())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeapType {
    Func,
    Extern,
    Any,
    Eq,
    I31,
    Struct,
    Array,
    Exn,
    None,
    NoFunc,
    NoExtern,
    NoExn,
    ConcreteFunc(FuncType),
    ConcreteStruct(StructType),
    ConcreteArray(ArrayType),
}

impl HeapType {
    #[allow(clippy::should_implement_trait)]
    pub fn eq(a: &Self, b: &Self) -> bool {
        a == b
    }

    pub fn as_concrete_func(&self) -> Option<&FuncType> {
        match self {
            Self::ConcreteFunc(f) => Some(f),
            _ => None,
        }
    }

    pub fn as_concrete_struct(&self) -> Option<&StructType> {
        match self {
            Self::ConcreteStruct(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_concrete_array(&self) -> Option<&ArrayType> {
        match self {
            Self::ConcreteArray(a) => Some(a),
            _ => None,
        }
    }

    pub fn unwrap_concrete_func(&self) -> &FuncType {
        self.as_concrete_func()
            .expect("HeapType::unwrap_concrete_func")
    }

    pub fn unwrap_concrete_struct(&self) -> &StructType {
        self.as_concrete_struct()
            .expect("HeapType::unwrap_concrete_struct")
    }

    pub fn unwrap_concrete_array(&self) -> &ArrayType {
        self.as_concrete_array()
            .expect("HeapType::unwrap_concrete_array")
    }

    pub fn matches(&self, other: &Self) -> bool {
        if self == other {
            return true;
        }
        match (self, other) {
            (Self::None, Self::Any | Self::Eq | Self::I31 | Self::Struct | Self::Array)
            | (Self::None, Self::ConcreteStruct(_) | Self::ConcreteArray(_)) => true,
            (Self::NoFunc, Self::Func | Self::ConcreteFunc(_)) => true,
            (Self::NoExtern, Self::Extern) => true,
            (Self::NoExn, Self::Exn) => true,
            (Self::Eq | Self::I31 | Self::Struct | Self::Array, Self::Any) => true,
            (Self::ConcreteStruct(_) | Self::ConcreteArray(_), Self::Any | Self::Eq) => true,
            (Self::ConcreteStruct(_), Self::Struct) => true,
            (Self::ConcreteArray(_), Self::Array) => true,
            (Self::I31 | Self::Struct | Self::Array, Self::Eq) => true,
            (Self::ConcreteFunc(_), Self::Func) => true,
            (Self::ConcreteFunc(a), Self::ConcreteFunc(b)) => a.matches(b),
            (Self::ConcreteStruct(a), Self::ConcreteStruct(b)) => a.matches(b),
            (Self::ConcreteArray(a), Self::ConcreteArray(b)) => a.matches(b),
            _ => false,
        }
    }

    pub(crate) fn to_wasm(&self) -> WHeapType {
        match self {
            Self::Func => WHeapType::Func,
            Self::Extern => WHeapType::Extern,
            Self::Any => WHeapType::Any,
            Self::Eq => WHeapType::Eq,
            Self::I31 => WHeapType::I31,
            Self::Struct => WHeapType::Struct,
            Self::Array => WHeapType::Array,
            Self::Exn => WHeapType::Exn,
            Self::None => WHeapType::None,
            Self::NoFunc => WHeapType::NoFunc,
            Self::NoExtern => WHeapType::NoExtern,
            Self::NoExn => WHeapType::NoExn,
            Self::ConcreteFunc(f) => WHeapType::Defined(TypeIdx(f.canon_id)),
            Self::ConcreteStruct(s) => WHeapType::Defined(TypeIdx(s.canon_id)),
            Self::ConcreteArray(a) => WHeapType::Defined(TypeIdx(a.canon_id)),
        }
    }

    pub(crate) fn from_wasm_canon(engine: &Arc<EngineInner>, h: WHeapType) -> Self {
        match h {
            WHeapType::Func => Self::Func,
            WHeapType::Extern => Self::Extern,
            WHeapType::Any => Self::Any,
            WHeapType::Eq => Self::Eq,
            WHeapType::I31 => Self::I31,
            WHeapType::Struct => Self::Struct,
            WHeapType::Array => Self::Array,
            WHeapType::Exn => Self::Exn,
            WHeapType::None => Self::None,
            WHeapType::NoFunc => Self::NoFunc,
            WHeapType::NoExtern => Self::NoExtern,
            WHeapType::NoExn => Self::NoExn,
            WHeapType::Cont | WHeapType::NoCont => Self::Func,
            WHeapType::Defined(idx) | WHeapType::ExactDefined(idx) => {
                let reg = engine.types.read().unwrap_or_else(|e| e.into_inner());
                let kind = match &reg.infos[idx.0 as usize].composite {
                    CanonComposite::Func { .. } => 0,
                    CanonComposite::Struct { .. } => 1,
                    CanonComposite::Array { .. } => 2,
                };
                drop(reg);
                match kind {
                    0 => Self::ConcreteFunc(FuncType::from_canon(Arc::clone(engine), idx.0)),
                    1 => Self::ConcreteStruct(StructType::from_canon(Arc::clone(engine), idx.0)),
                    _ => Self::ConcreteArray(ArrayType::from_canon(Arc::clone(engine), idx.0)),
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefType {
    nullable: bool,
    heap_type: HeapType,
}

impl RefType {
    pub const FUNCREF: Self = Self {
        nullable: true,
        heap_type: HeapType::Func,
    };
    pub const EXTERNREF: Self = Self {
        nullable: true,
        heap_type: HeapType::Extern,
    };
    pub const ANYREF: Self = Self {
        nullable: true,
        heap_type: HeapType::Any,
    };
    pub const EQREF: Self = Self {
        nullable: true,
        heap_type: HeapType::Eq,
    };
    pub const EXNREF: Self = Self {
        nullable: true,
        heap_type: HeapType::Exn,
    };

    pub const fn new(nullable: bool, heap_type: HeapType) -> Self {
        Self {
            nullable,
            heap_type,
        }
    }

    pub const fn is_nullable(&self) -> bool {
        self.nullable
    }

    pub const fn heap_type(&self) -> &HeapType {
        &self.heap_type
    }

    pub fn matches(&self, other: &Self) -> bool {
        (!self.nullable || other.nullable) && self.heap_type.matches(&other.heap_type)
    }

    pub(crate) fn to_wasm(&self) -> WRefType {
        WRefType {
            nullable: self.nullable,
            heap: self.heap_type.to_wasm(),
        }
    }

    pub(crate) fn from_wasm_canon(engine: &Arc<EngineInner>, rt: WRefType) -> Self {
        Self {
            nullable: rt.nullable,
            heap_type: HeapType::from_wasm_canon(engine, rt.heap),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValType {
    I32,
    I64,
    F32,
    F64,
    V128,
    Ref(RefType),
}

impl ValType {
    pub const FUNCREF: Self = Self::Ref(RefType::FUNCREF);
    pub const EXTERNREF: Self = Self::Ref(RefType::EXTERNREF);
    pub const ANYREF: Self = Self::Ref(RefType::ANYREF);
    pub const EQREF: Self = Self::Ref(RefType::EQREF);
    pub const EXNREF: Self = Self::Ref(RefType::EXNREF);

    pub fn as_ref(&self) -> Option<&RefType> {
        match self {
            Self::Ref(r) => Some(r),
            _ => None,
        }
    }

    pub fn unwrap_ref(&self) -> &RefType {
        self.as_ref()
            .expect("ValType::unwrap_ref on non-reference type")
    }

    pub fn is_ref(&self) -> bool {
        matches!(self, Self::Ref(_))
    }

    pub fn matches(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Ref(a), Self::Ref(b)) => a.matches(b),
            _ => self == other,
        }
    }

    pub(crate) fn to_wasm(&self) -> WValType {
        match self {
            Self::I32 => WValType::I32,
            Self::I64 => WValType::I64,
            Self::F32 => WValType::F32,
            Self::F64 => WValType::F64,
            Self::V128 => WValType::V128,
            Self::Ref(r) => WValType::Ref(r.to_wasm()),
        }
    }

    pub(crate) fn from_wasm_canon(engine: &Arc<EngineInner>, vt: WValType) -> Self {
        match vt {
            WValType::I32 => Self::I32,
            WValType::I64 => Self::I64,
            WValType::F32 => Self::F32,
            WValType::F64 => Self::F64,
            WValType::V128 => Self::V128,
            WValType::Ref(r) => Self::Ref(RefType::from_wasm_canon(engine, r)),
        }
    }

    /// Converts a module-local `ValType` using `local_type_to_global_canon`.
    pub(crate) fn from_module_wasm(
        engine: &Arc<EngineInner>,
        vt: WValType,
        type_to_global_canon: &[u32],
    ) -> Self {
        let resolved = match vt {
            WValType::Ref(rt) => WValType::Ref(WRefType {
                nullable: rt.nullable,
                heap: match rt.heap {
                    WHeapType::Defined(idx) | WHeapType::ExactDefined(idx) => {
                        WHeapType::Defined(TypeIdx(type_to_global_canon[idx.0 as usize]))
                    }
                    other => other,
                },
            }),
            other => other,
        };
        Self::from_wasm_canon(engine, resolved)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryType {
    min_pages: u64,
    max_pages: Option<u64>,
    is_64: bool,
    shared: bool,
}

impl MemoryType {
    pub fn new(min: u32, max: Option<u32>) -> Self {
        Self {
            min_pages: u64::from(min),
            max_pages: max.map(u64::from),
            is_64: false,
            shared: false,
        }
    }

    pub fn new64(min: u64, max: Option<u64>) -> Self {
        Self {
            min_pages: min,
            max_pages: max,
            is_64: true,
            shared: false,
        }
    }

    pub fn shared(min: u32, max: u32) -> Self {
        Self {
            min_pages: u64::from(min),
            max_pages: Some(u64::from(max)),
            is_64: false,
            shared: true,
        }
    }

    pub(crate) fn from_raw(
        min_pages: u64,
        max_pages: Option<u64>,
        is_64: bool,
        shared: bool,
    ) -> Self {
        Self {
            min_pages,
            max_pages,
            is_64,
            shared,
        }
    }

    pub fn minimum(&self) -> u64 {
        self.min_pages
    }

    pub fn maximum(&self) -> Option<u64> {
        self.max_pages
    }

    pub fn is_64(&self) -> bool {
        self.is_64
    }

    pub fn is_shared(&self) -> bool {
        self.shared
    }

    pub fn page_size(&self) -> u64 {
        65_536
    }

    /// Wasm import-matching check (`self` = actual memory type, `expected` = import requirement).
    pub fn matches(&self, expected: &Self) -> bool {
        if self.is_64 != expected.is_64 || self.shared != expected.shared {
            return false;
        }
        if self.min_pages < expected.min_pages {
            return false;
        }
        match (self.max_pages, expected.max_pages) {
            (_, None) => true,
            (Some(actual_max), Some(exp_max)) => actual_max <= exp_max,
            (None, Some(_)) => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalType {
    content: ValType,
    mutability: Mutability,
}

impl GlobalType {
    pub fn new(content: ValType, mutability: Mutability) -> Self {
        Self {
            content,
            mutability,
        }
    }

    pub fn content(&self) -> &ValType {
        &self.content
    }

    pub fn mutability(&self) -> Mutability {
        self.mutability
    }

    pub fn matches(&self, expected: &Self) -> bool {
        if self.mutability != expected.mutability {
            return false;
        }
        match self.mutability {
            Mutability::Var => self.content == expected.content,
            Mutability::Const => self.content.matches(&expected.content),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableType {
    element: RefType,
    min: u64,
    max: Option<u64>,
    is_64: bool,
}

impl TableType {
    pub fn new(element: RefType, min: u32, max: Option<u32>) -> Self {
        Self {
            element,
            min: u64::from(min),
            max: max.map(u64::from),
            is_64: false,
        }
    }

    pub fn new64(element: RefType, min: u64, max: Option<u64>) -> Self {
        Self {
            element,
            min,
            max,
            is_64: true,
        }
    }

    pub(crate) fn from_module_wasm(
        engine: &Arc<EngineInner>,
        tt: wtime_types::TableType,
        type_to_global_canon: &[u32],
    ) -> Self {
        let rt = match ValType::from_module_wasm(
            engine,
            WValType::Ref(tt.element),
            type_to_global_canon,
        ) {
            ValType::Ref(r) => r,
            _ => unreachable!(),
        };
        Self {
            element: rt,
            min: tt.limits.min,
            max: tt.limits.max,
            is_64: tt.index_type == wtime_types::IndexType::I64,
        }
    }

    pub fn element(&self) -> &RefType {
        &self.element
    }

    pub fn minimum(&self) -> u64 {
        self.min
    }

    pub fn maximum(&self) -> Option<u64> {
        self.max
    }

    pub fn is_table64(&self) -> bool {
        self.is_64
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagType {
    func_ty: FuncType,
}

impl TagType {
    pub fn new(func_ty: FuncType) -> Self {
        Self { func_ty }
    }

    pub fn ty(&self) -> &FuncType {
        &self.func_ty
    }

    pub fn params(&self) -> std::vec::IntoIter<ValType> {
        self.func_ty.params()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExnType {
    tag_ty: TagType,
}

impl ExnType {
    pub fn new(engine: &Engine, fields: impl IntoIterator<Item = ValType>) -> Result<Self> {
        let ft = FuncType::new(engine, fields, []);
        Ok(Self {
            tag_ty: TagType::new(ft),
        })
    }

    pub fn from_tag_type(tag_ty: &TagType) -> Result<Self> {
        Ok(Self {
            tag_ty: tag_ty.clone(),
        })
    }

    pub fn fields(&self) -> std::vec::IntoIter<FieldType> {
        self.tag_ty
            .params()
            .map(|vt| FieldType::new(Mutability::Const, StorageType::ValType(vt)))
            .collect::<Vec<_>>()
            .into_iter()
    }

    pub fn tag_type(&self) -> TagType {
        self.tag_ty.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternType {
    Func(FuncType),
    Global(GlobalType),
    Table(TableType),
    Memory(MemoryType),
    Tag(TagType),
}

impl ExternType {
    pub fn unwrap_func(&self) -> &FuncType {
        match self {
            Self::Func(f) => f,
            _ => panic!("ExternType is not Func"),
        }
    }

    pub fn unwrap_global(&self) -> &GlobalType {
        match self {
            Self::Global(g) => g,
            _ => panic!("ExternType is not Global"),
        }
    }

    pub fn unwrap_table(&self) -> &TableType {
        match self {
            Self::Table(t) => t,
            _ => panic!("ExternType is not Table"),
        }
    }

    pub fn unwrap_memory(&self) -> &MemoryType {
        match self {
            Self::Memory(m) => m,
            _ => panic!("ExternType is not Memory"),
        }
    }

    pub fn unwrap_tag(&self) -> &TagType {
        match self {
            Self::Tag(t) => t,
            _ => panic!("ExternType is not Tag"),
        }
    }
}

/// Borrowed descriptor for a single module import.
#[derive(Clone, Debug)]
pub struct ImportType<'a> {
    pub(crate) module: &'a str,
    pub(crate) name: &'a str,
    pub(crate) ty: ExternType,
}

impl<'a> ImportType<'a> {
    pub fn module(&self) -> &'a str {
        self.module
    }

    pub fn name(&self) -> &'a str {
        self.name
    }

    pub fn ty(&self) -> ExternType {
        self.ty.clone()
    }
}

/// Borrowed descriptor for a single module export.
#[derive(Clone, Debug)]
pub struct ExportType<'a> {
    pub(crate) name: &'a str,
    pub(crate) ty: ExternType,
}

impl<'a> ExportType<'a> {
    pub fn name(&self) -> &'a str {
        self.name
    }

    pub fn ty(&self) -> ExternType {
        self.ty.clone()
    }
}

/// Precomputed export lookup handle returned by `Module::get_export_index`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModuleExport {
    pub(crate) module_id: u64,
    pub(crate) kind: ExportKind,
    pub(crate) index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ExportKind {
    Func,
    Global,
    Memory,
    Table,
    Tag,
}
