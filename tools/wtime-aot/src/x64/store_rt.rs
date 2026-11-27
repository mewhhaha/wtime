//! Multi-module `X64Target::StoreCdylib` runtime helpers, host-import trampolines,
//! export-entry trampolines, instance initializer, single ELF descriptor (`wt_store_get_desc`),
//! and binary metadata encoder/decoder (`WTST0001`).
//!
//! All code here is 100% safe Rust (`#![forbid(unsafe_code)]`) with zero external dependencies.

use std::collections::BTreeMap;

use wtime_types::{
    ArrayType, CompositeType, ContType, FieldType, FuncType, HeapType, IndexType, Limits, RefType,
    StorageType, StructType, SubType, TableType, TypeGroup, TypeIdx, ValType,
};

use super::asm::{Asm, Cond, Gp, Mem, RelocTarget, Xmm};
use super::gc_rt::{self, GC_OFF_STACK_CHAIN, GC_OFF_WALK_STOP_RSP};
use super::ir::{TruncOp, Ty};
use super::module::{GcRefFamily, X64ConstVal, X64Module};
use super::regalloc::{AbiSlot, layout_args, layout_rets};
use super::runtime::{RuntimeTables, emit_trunc_helper};
use super::{
    CtxLayout, HELPER_ATOMIC_NOTIFY, HELPER_ATOMIC_WAIT, HELPER_EXTRA_MEM_COPY,
    HELPER_EXTRA_MEM_FILL, HELPER_EXTRA_MEM_GROW, HELPER_GC_ARRAY_COPY, HELPER_GC_ARRAY_FILL,
    HELPER_GC_ARRAY_FILL_RAW, HELPER_GC_ARRAY_INIT_DATA, HELPER_GC_ARRAY_INIT_ELEM,
    HELPER_GC_COLLECT, HELPER_INITIALIZE, HELPER_MEM_COPY, HELPER_MEM_FILL, HELPER_MEM_GROW,
    HELPER_MEM_INIT, HELPER_STORE_EXN_ALLOC, HELPER_TABLE_COPY, HELPER_TABLE_FILL,
    HELPER_TABLE_GROW, HELPER_TABLE_INIT, HELPER_TRUNC_BASE, NUM_HELPERS, OFF_MEM_BASE,
    OFF_MEM_RESERVE_BYTES, OFF_STORE_ELEM_BUF_PTR, OFF_STORE_FUNC_REFS, OFF_STORE_MEM_DESCS,
    OFF_STORE_PTR, OFF_STORE_TABLE_DESCS, RodataConsts, STORE_OFF_ATOMIC_NOTIFY_FN,
    STORE_OFF_ATOMIC_WAIT_FN, STORE_OFF_EXN_ALLOC_FN, STORE_OFF_GC_COLLECT_FN,
    STORE_OFF_HOST_CALL_FN, STORE_OFF_MEM_GROW_FN, STORE_OFF_RET_SPILL_PTR,
    STORE_OFF_TABLE_GROW_FN, TRAP_ALLOC_FAILED, TRAP_OOB, TRAP_TABLE_OOB,
};

/// Current `StoreCdylib` binary ABI version verified by `wtime-aot-host`.
pub const STORE_ABI_VERSION: u32 = 1;

/// Single exported ELF symbol for every `X64Target::StoreCdylib` `.so` image.
pub const STORE_EXPORT_NAMES: [&str; 1] = ["wt_store_get_desc"];

/// Binary metadata magic header (`WTST0001`).
pub const STORE_META_MAGIC: [u8; 8] = *b"WTST0001";

/// Machine-ABI value type code stored in metadata trampoline signature tables.
pub const TY_I32: u8 = 0;
pub const TY_I64: u8 = 1;
pub const TY_F32: u8 = 2;
pub const TY_F64: u8 = 3;
pub const TY_V128: u8 = 4;

#[inline]
pub fn ty_to_code(ty: Ty) -> u8 {
    match ty {
        Ty::I32 => TY_I32,
        Ty::I64 => TY_I64,
        Ty::F32 => TY_F32,
        Ty::F64 => TY_F64,
        Ty::V128 => TY_V128,
    }
}

#[inline]
pub fn code_to_ty(c: u8) -> Result<Ty, String> {
    match c {
        TY_I32 => Ok(Ty::I32),
        TY_I64 => Ok(Ty::I64),
        TY_F32 => Ok(Ty::F32),
        TY_F64 => Ok(Ty::F64),
        TY_V128 => Ok(Ty::V128),
        _ => Err(format!("store_meta: invalid Ty code {c}")),
    }
}

#[inline]
pub fn val_type_to_machine_ty(vt: ValType) -> Ty {
    Ty::from_val(vt)
}

/// `.rodata` offsets and deduplicated trampoline signatures built for `X64Target::StoreCdylib`.
#[derive(Clone, Debug)]
pub struct StoreRodataInfo {
    pub meta_ro_off: u32,
    pub meta_len: u64,
    pub def_fn_rel_ro_off: u32,
    pub host_tramp_rel_ro_off: u32,
    pub export_tramp_rel_ro_off: u32,
    pub host_tramp_sigs: Vec<(Vec<Ty>, Vec<Ty>)>,
    pub export_tramp_sigs: Vec<(Vec<Ty>, Vec<Ty>)>,
}

/// Exception tag payload layout `(record_size_bytes, [(ty_code, field_offset, ref_kind_u8)])`.
pub type StoreTagLayout = (u32, Vec<(u8, i32, u8)>);

/// Complete decoded metadata embedded in a `StoreCdylib` `.rodata` section, consumed by `wtime-aot-host`.
#[derive(Clone, Debug)]
pub struct StoreModuleMeta {
    pub abi_version: u32,
    pub uses_gc: bool,
    pub uses_exceptions: bool,
    pub uses_threads: bool,
    pub ctx_total_bytes: u64,
    pub gc_elem_buf_bytes: u64,
    pub n_imports: u32,
    pub n_defs: u32,
    pub n_local_canon: u32,
    pub start_func: Option<u32>,
    pub raw_type_groups: Vec<TypeGroup>,
    pub type_canon: Vec<u32>,
    pub func_type_idx: Vec<u32>,
    /// Original Wasm import declaration order `(kind, kind_local_idx)` where
    /// `0=Func, 1=Table, 2=Mem, 3=Global, 4=Tag`.
    pub import_order: Vec<(u8, u32)>,
    pub imported_funcs: Vec<(String, String, u32)>,
    pub imported_globals: Vec<(String, String, ValType, bool, i32)>,
    /// `(module, name, min_pages, eff_max_pages, declared_max_pages, is64, shared)`
    pub imported_mems: Vec<super::module::ImportedMemMeta>,
    pub imported_tables: Vec<(String, String, TableType)>,
    pub imported_tags: Vec<(String, String, u32)>,
    /// `(global_idx, val_type, mutable, ctx_off, is_traced)`
    pub defined_globals: Vec<(u32, ValType, bool, i32, bool)>,
    /// `(mem_idx, min_pages, eff_max_pages, declared_max_pages, is64, shared)`
    pub defined_mems: Vec<(u32, u64, u64, Option<u64>, bool, bool)>,
    /// `(table_idx, min_len, eff_max_len, is64, is_funcref, ref_family_code, table_type)`
    pub defined_tables: Vec<(u32, u32, u32, bool, bool, u8, TableType)>,
    /// `(tag_idx, type_idx)`
    pub defined_tags: Vec<(u32, u32)>,
    /// For every local tag `0..tags.len()`: `(rec_size, [(ty_code, field_off, ref_kind_u8)])`.
    pub all_tag_layouts: Vec<StoreTagLayout>,
    /// `(name, func_idx, export_tramp_idx)`
    pub exported_funcs: Vec<(String, u32, u32)>,
    /// `(name, global_idx, val_type, mutable, ctx_off)`
    pub exported_globals: Vec<(String, u32, ValType, bool, i32)>,
    pub exported_mems: Vec<(String, u32)>,
    pub exported_tables: Vec<(String, u32)>,
    pub exported_tags: Vec<(String, u32)>,
    /// Traced passive GC element segments: `(elem_seg_idx, byte_off_in_elem_buf, elem_count)`.
    pub passive_gc_elem_ranges: Vec<(u32, i32, u32)>,
    pub elem_dropped_off: Option<i32>,
    pub host_tramp_sigs: Vec<(Vec<u8>, Vec<u8>)>,
    pub export_tramp_sigs: Vec<(Vec<u8>, Vec<u8>)>,
}

// ---- Compact binary metadata codec (shared by compiler and host loader) ----

struct Enc {
    buf: Vec<u8>,
}

impl Enc {
    fn new() -> Self {
        Self {
            buf: Vec::with_capacity(4096),
        }
    }
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.buf.extend_from_slice(s.as_bytes());
    }
    fn heap_type(&mut self, h: HeapType) {
        match h {
            HeapType::Func => self.u8(0),
            HeapType::Extern => self.u8(1),
            HeapType::Exn => self.u8(2),
            HeapType::Any => self.u8(3),
            HeapType::Eq => self.u8(4),
            HeapType::I31 => self.u8(5),
            HeapType::Struct => self.u8(6),
            HeapType::Array => self.u8(7),
            HeapType::None => self.u8(8),
            HeapType::NoFunc => self.u8(9),
            HeapType::NoExtern => self.u8(10),
            HeapType::NoExn => self.u8(11),
            HeapType::Cont => self.u8(12),
            HeapType::NoCont => self.u8(13),
            HeapType::Defined(ti) => {
                self.u8(14);
                self.u32(ti.0);
            }
            HeapType::ExactDefined(ti) => {
                self.u8(15);
                self.u32(ti.0);
            }
        }
    }
    fn ref_type(&mut self, rt: RefType) {
        self.bool(rt.nullable);
        self.heap_type(rt.heap);
    }
    fn val_type(&mut self, vt: ValType) {
        match vt {
            ValType::I32 => self.u8(0),
            ValType::I64 => self.u8(1),
            ValType::F32 => self.u8(2),
            ValType::F64 => self.u8(3),
            ValType::V128 => self.u8(4),
            ValType::Ref(rt) => {
                self.u8(5);
                self.ref_type(rt);
            }
        }
    }
    fn storage_type(&mut self, st: StorageType) {
        match st {
            StorageType::I8 => self.u8(0),
            StorageType::I16 => self.u8(1),
            StorageType::Val(vt) => {
                self.u8(2);
                self.val_type(vt);
            }
        }
    }
    fn field_type(&mut self, ft: FieldType) {
        self.storage_type(ft.storage);
        self.bool(ft.mutable);
    }
    fn limits(&mut self, lim: Limits) {
        self.u64(lim.min);
        match lim.max {
            Some(m) => {
                self.u8(1);
                self.u64(m);
            }
            None => self.u8(0),
        }
    }
    fn table_type(&mut self, tt: TableType) {
        self.ref_type(tt.element);
        self.limits(tt.limits);
        self.bool(tt.index_type == IndexType::I64);
    }
    fn type_group(&mut self, tg: &TypeGroup) {
        self.bool(tg.explicit);
        self.u32(tg.types.len() as u32);
        for st in &tg.types {
            self.bool(st.final_);
            match st.supertype {
                Some(sup) => {
                    self.u8(1);
                    self.u32(sup.0);
                }
                None => self.u8(0),
            }
            match &st.composite {
                CompositeType::Func(ft) => {
                    self.u8(0);
                    self.u32(ft.params.len() as u32);
                    for &vt in &ft.params {
                        self.val_type(vt);
                    }
                    self.u32(ft.results.len() as u32);
                    for &vt in &ft.results {
                        self.val_type(vt);
                    }
                }
                CompositeType::Struct(s) => {
                    self.u8(1);
                    self.u32(s.fields.len() as u32);
                    for &fd in &s.fields {
                        self.field_type(fd);
                    }
                }
                CompositeType::Array(a) => {
                    self.u8(2);
                    self.field_type(a.field);
                }
                CompositeType::Cont(ct) => {
                    self.u8(3);
                    self.u32(ct.function.0);
                }
            }
        }
    }
}

struct Dec<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Dec<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or("store_meta: read offset overflow")?;
        if end > self.buf.len() {
            return Err("store_meta: unexpected EOF".into());
        }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn bool(&mut self) -> Result<bool, String> {
        Ok(self.u8()? != 0)
    }
    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(self.u32()? as i32)
    }
    fn u64(&mut self) -> Result<u64, String> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }
    fn str(&mut self) -> Result<String, String> {
        let len = self.u32()? as usize;
        let b = self.take(len)?;
        std::str::from_utf8(b)
            .map(str::to_owned)
            .map_err(|_| "store_meta: invalid UTF-8".into())
    }
    fn heap_type(&mut self) -> Result<HeapType, String> {
        match self.u8()? {
            0 => Ok(HeapType::Func),
            1 => Ok(HeapType::Extern),
            2 => Ok(HeapType::Exn),
            3 => Ok(HeapType::Any),
            4 => Ok(HeapType::Eq),
            5 => Ok(HeapType::I31),
            6 => Ok(HeapType::Struct),
            7 => Ok(HeapType::Array),
            8 => Ok(HeapType::None),
            9 => Ok(HeapType::NoFunc),
            10 => Ok(HeapType::NoExtern),
            11 => Ok(HeapType::NoExn),
            12 => Ok(HeapType::Cont),
            13 => Ok(HeapType::NoCont),
            14 => Ok(HeapType::Defined(TypeIdx(self.u32()?))),
            15 => Ok(HeapType::ExactDefined(TypeIdx(self.u32()?))),
            other => Err(format!("store_meta: unknown HeapType tag {other}")),
        }
    }
    fn ref_type(&mut self) -> Result<RefType, String> {
        let nullable = self.bool()?;
        let heap = self.heap_type()?;
        Ok(RefType { nullable, heap })
    }
    fn val_type(&mut self) -> Result<ValType, String> {
        match self.u8()? {
            0 => Ok(ValType::I32),
            1 => Ok(ValType::I64),
            2 => Ok(ValType::F32),
            3 => Ok(ValType::F64),
            4 => Ok(ValType::V128),
            5 => Ok(ValType::Ref(self.ref_type()?)),
            other => Err(format!("store_meta: unknown ValType tag {other}")),
        }
    }
    fn storage_type(&mut self) -> Result<StorageType, String> {
        match self.u8()? {
            0 => Ok(StorageType::I8),
            1 => Ok(StorageType::I16),
            2 => Ok(StorageType::Val(self.val_type()?)),
            other => Err(format!("store_meta: unknown StorageType tag {other}")),
        }
    }
    fn field_type(&mut self) -> Result<FieldType, String> {
        let storage = self.storage_type()?;
        let mutable = self.bool()?;
        Ok(FieldType { storage, mutable })
    }
    fn limits(&mut self) -> Result<Limits, String> {
        let min = self.u64()?;
        let has_max = self.bool()?;
        let max = if has_max { Some(self.u64()?) } else { None };
        Ok(Limits { min, max })
    }
    fn table_type(&mut self) -> Result<TableType, String> {
        let element = self.ref_type()?;
        let limits = self.limits()?;
        let index_type = if self.bool()? {
            IndexType::I64
        } else {
            IndexType::I32
        };
        Ok(TableType {
            element,
            limits,
            index_type,
        })
    }
    fn type_group(&mut self) -> Result<TypeGroup, String> {
        let explicit = self.bool()?;
        let n = self.u32()? as usize;
        let mut types = Vec::with_capacity(n);
        for _ in 0..n {
            let final_ = self.bool()?;
            let supertype = if self.bool()? {
                Some(TypeIdx(self.u32()?))
            } else {
                None
            };
            let composite = match self.u8()? {
                0 => {
                    let np = self.u32()? as usize;
                    let mut params = Vec::with_capacity(np);
                    for _ in 0..np {
                        params.push(self.val_type()?);
                    }
                    let nr = self.u32()? as usize;
                    let mut results = Vec::with_capacity(nr);
                    for _ in 0..nr {
                        results.push(self.val_type()?);
                    }
                    CompositeType::Func(FuncType { params, results })
                }
                1 => {
                    let nf = self.u32()? as usize;
                    let mut fields = Vec::with_capacity(nf);
                    for _ in 0..nf {
                        fields.push(self.field_type()?);
                    }
                    CompositeType::Struct(StructType { fields })
                }
                2 => CompositeType::Array(ArrayType {
                    field: self.field_type()?,
                }),
                3 => CompositeType::Cont(ContType {
                    function: TypeIdx(self.u32()?),
                }),
                other => return Err(format!("store_meta: unknown CompositeType tag {other}")),
            };
            types.push(SubType {
                final_,
                supertype,
                composite,
                descriptor: None,
                describes: None,
            });
        }
        Ok(TypeGroup { explicit, types })
    }
}

/// Encodes `StoreModuleMeta` into a self-contained binary blob (`WTST0001`).
pub fn encode_store_metadata(m: &StoreModuleMeta) -> Vec<u8> {
    let mut e = Enc::new();
    e.buf.extend_from_slice(&STORE_META_MAGIC);
    e.u32(m.abi_version);
    e.bool(m.uses_gc);
    e.bool(m.uses_exceptions);
    e.bool(m.uses_threads);
    e.u64(m.ctx_total_bytes);
    e.u64(m.gc_elem_buf_bytes);
    e.u32(m.n_imports);
    e.u32(m.n_defs);
    e.u32(m.n_local_canon);
    match m.start_func {
        Some(f) => {
            e.u8(1);
            e.u32(f);
        }
        None => e.u8(0),
    }
    e.u32(m.raw_type_groups.len() as u32);
    for tg in &m.raw_type_groups {
        e.type_group(tg);
    }
    e.u32(m.type_canon.len() as u32);
    for &cid in &m.type_canon {
        e.u32(cid);
    }
    e.u32(m.func_type_idx.len() as u32);
    for &ti in &m.func_type_idx {
        e.u32(ti);
    }
    e.u32(m.import_order.len() as u32);
    for &(kind, idx) in &m.import_order {
        e.u8(kind);
        e.u32(idx);
    }
    e.u32(m.imported_funcs.len() as u32);
    for (mo, na, ti) in &m.imported_funcs {
        e.str(mo);
        e.str(na);
        e.u32(*ti);
    }
    e.u32(m.imported_globals.len() as u32);
    for (mo, na, vt, mut_, off) in &m.imported_globals {
        e.str(mo);
        e.str(na);
        e.val_type(*vt);
        e.bool(*mut_);
        e.i32(*off);
    }
    e.u32(m.imported_mems.len() as u32);
    for (mo, na, min, max, dmax, is64, sh) in &m.imported_mems {
        e.str(mo);
        e.str(na);
        e.u64(*min);
        e.u64(*max);
        match dmax {
            Some(v) => {
                e.u8(1);
                e.u64(*v);
            }
            None => e.u8(0),
        }
        e.bool(*is64);
        e.bool(*sh);
    }
    e.u32(m.imported_tables.len() as u32);
    for (mo, na, tt) in &m.imported_tables {
        e.str(mo);
        e.str(na);
        e.table_type(*tt);
    }
    e.u32(m.imported_tags.len() as u32);
    for (mo, na, ti) in &m.imported_tags {
        e.str(mo);
        e.str(na);
        e.u32(*ti);
    }
    e.u32(m.defined_globals.len() as u32);
    for &(gi, vt, mut_, off, traced) in &m.defined_globals {
        e.u32(gi);
        e.val_type(vt);
        e.bool(mut_);
        e.i32(off);
        e.bool(traced);
    }
    e.u32(m.defined_mems.len() as u32);
    for &(mi, min, max, dmax, is64, sh) in &m.defined_mems {
        e.u32(mi);
        e.u64(min);
        e.u64(max);
        match dmax {
            Some(v) => {
                e.u8(1);
                e.u64(v);
            }
            None => e.u8(0),
        }
        e.bool(is64);
        e.bool(sh);
    }
    e.u32(m.defined_tables.len() as u32);
    for &(ti, min_l, max_l, is64, is_f, fam, tt) in &m.defined_tables {
        e.u32(ti);
        e.u32(min_l);
        e.u32(max_l);
        e.bool(is64);
        e.bool(is_f);
        e.u8(fam);
        e.table_type(tt);
    }
    e.u32(m.defined_tags.len() as u32);
    for &(tag_i, ti) in &m.defined_tags {
        e.u32(tag_i);
        e.u32(ti);
    }
    e.u32(m.all_tag_layouts.len() as u32);
    for (rec_sz, fields) in &m.all_tag_layouts {
        e.u32(*rec_sz);
        e.u32(fields.len() as u32);
        for &(ty_c, foff, rk) in fields {
            e.u8(ty_c);
            e.i32(foff);
            e.u8(rk);
        }
    }
    e.u32(m.exported_funcs.len() as u32);
    for (na, fi, tramp_i) in &m.exported_funcs {
        e.str(na);
        e.u32(*fi);
        e.u32(*tramp_i);
    }
    e.u32(m.exported_globals.len() as u32);
    for (na, gi, vt, mut_, off) in &m.exported_globals {
        e.str(na);
        e.u32(*gi);
        e.val_type(*vt);
        e.bool(*mut_);
        e.i32(*off);
    }
    e.u32(m.exported_mems.len() as u32);
    for (na, mi) in &m.exported_mems {
        e.str(na);
        e.u32(*mi);
    }
    e.u32(m.exported_tables.len() as u32);
    for (na, ti) in &m.exported_tables {
        e.str(na);
        e.u32(*ti);
    }
    e.u32(m.exported_tags.len() as u32);
    for (na, ti) in &m.exported_tags {
        e.str(na);
        e.u32(*ti);
    }
    e.u32(m.passive_gc_elem_ranges.len() as u32);
    for &(ei, boff, cnt) in &m.passive_gc_elem_ranges {
        e.u32(ei);
        e.i32(boff);
        e.u32(cnt);
    }
    match m.elem_dropped_off {
        Some(off) => {
            e.u8(1);
            e.i32(off);
        }
        None => e.u8(0),
    }
    e.u32(m.host_tramp_sigs.len() as u32);
    for (ps, rs) in &m.host_tramp_sigs {
        e.u32(ps.len() as u32);
        e.buf.extend_from_slice(ps);
        e.u32(rs.len() as u32);
        e.buf.extend_from_slice(rs);
    }
    e.u32(m.export_tramp_sigs.len() as u32);
    for (ps, rs) in &m.export_tramp_sigs {
        e.u32(ps.len() as u32);
        e.buf.extend_from_slice(ps);
        e.u32(rs.len() as u32);
        e.buf.extend_from_slice(rs);
    }
    e.buf
}

/// Decodes a `WTST0001` binary metadata blob from a loaded `StoreCdylib` `.so`.
pub fn decode_store_metadata(bytes: &[u8]) -> Result<StoreModuleMeta, String> {
    let mut d = Dec::new(bytes);
    if d.take(8)? != STORE_META_MAGIC {
        return Err("store_meta: bad magic".into());
    }
    let abi_version = d.u32()?;
    if abi_version != STORE_ABI_VERSION {
        return Err(format!(
            "store_meta: unsupported ABI version {abi_version} (expected {STORE_ABI_VERSION})"
        ));
    }
    let uses_gc = d.bool()?;
    let uses_exceptions = d.bool()?;
    let uses_threads = d.bool()?;
    let ctx_total_bytes = d.u64()?;
    let gc_elem_buf_bytes = d.u64()?;
    let n_imports = d.u32()?;
    let n_defs = d.u32()?;
    let n_local_canon = d.u32()?;
    let start_func = if d.bool()? { Some(d.u32()?) } else { None };
    let n_tg = d.u32()? as usize;
    let mut raw_type_groups = Vec::with_capacity(n_tg);
    for _ in 0..n_tg {
        raw_type_groups.push(d.type_group()?);
    }
    let n_tc = d.u32()? as usize;
    let mut type_canon = Vec::with_capacity(n_tc);
    for _ in 0..n_tc {
        type_canon.push(d.u32()?);
    }
    let n_ft = d.u32()? as usize;
    let mut func_type_idx = Vec::with_capacity(n_ft);
    for _ in 0..n_ft {
        func_type_idx.push(d.u32()?);
    }
    let n_ord = d.u32()? as usize;
    let mut import_order = Vec::with_capacity(n_ord);
    for _ in 0..n_ord {
        import_order.push((d.u8()?, d.u32()?));
    }
    let n_if = d.u32()? as usize;
    let mut imported_funcs = Vec::with_capacity(n_if);
    for _ in 0..n_if {
        imported_funcs.push((d.str()?, d.str()?, d.u32()?));
    }
    let n_ig = d.u32()? as usize;
    let mut imported_globals = Vec::with_capacity(n_ig);
    for _ in 0..n_ig {
        imported_globals.push((d.str()?, d.str()?, d.val_type()?, d.bool()?, d.i32()?));
    }
    let n_im = d.u32()? as usize;
    let mut imported_mems = Vec::with_capacity(n_im);
    for _ in 0..n_im {
        let mo = d.str()?;
        let na = d.str()?;
        let min = d.u64()?;
        let max = d.u64()?;
        let dmax = if d.bool()? { Some(d.u64()?) } else { None };
        let is64 = d.bool()?;
        let sh = d.bool()?;
        imported_mems.push((mo, na, min, max, dmax, is64, sh));
    }
    let n_it = d.u32()? as usize;
    let mut imported_tables = Vec::with_capacity(n_it);
    for _ in 0..n_it {
        imported_tables.push((d.str()?, d.str()?, d.table_type()?));
    }
    let n_itag = d.u32()? as usize;
    let mut imported_tags = Vec::with_capacity(n_itag);
    for _ in 0..n_itag {
        imported_tags.push((d.str()?, d.str()?, d.u32()?));
    }
    let n_dg = d.u32()? as usize;
    let mut defined_globals = Vec::with_capacity(n_dg);
    for _ in 0..n_dg {
        defined_globals.push((d.u32()?, d.val_type()?, d.bool()?, d.i32()?, d.bool()?));
    }
    let n_dm = d.u32()? as usize;
    let mut defined_mems = Vec::with_capacity(n_dm);
    for _ in 0..n_dm {
        let mi = d.u32()?;
        let min = d.u64()?;
        let max = d.u64()?;
        let dmax = if d.bool()? { Some(d.u64()?) } else { None };
        let is64 = d.bool()?;
        let sh = d.bool()?;
        defined_mems.push((mi, min, max, dmax, is64, sh));
    }
    let n_dt = d.u32()? as usize;
    let mut defined_tables = Vec::with_capacity(n_dt);
    for _ in 0..n_dt {
        defined_tables.push((
            d.u32()?,
            d.u32()?,
            d.u32()?,
            d.bool()?,
            d.bool()?,
            d.u8()?,
            d.table_type()?,
        ));
    }
    let n_dtag = d.u32()? as usize;
    let mut defined_tags = Vec::with_capacity(n_dtag);
    for _ in 0..n_dtag {
        defined_tags.push((d.u32()?, d.u32()?));
    }
    let n_tl = d.u32()? as usize;
    let mut all_tag_layouts = Vec::with_capacity(n_tl);
    for _ in 0..n_tl {
        let rec_sz = d.u32()?;
        let nf = d.u32()? as usize;
        let mut fields = Vec::with_capacity(nf);
        for _ in 0..nf {
            fields.push((d.u8()?, d.i32()?, d.u8()?));
        }
        all_tag_layouts.push((rec_sz, fields));
    }
    let n_ef = d.u32()? as usize;
    let mut exported_funcs = Vec::with_capacity(n_ef);
    for _ in 0..n_ef {
        exported_funcs.push((d.str()?, d.u32()?, d.u32()?));
    }
    let n_eg = d.u32()? as usize;
    let mut exported_globals = Vec::with_capacity(n_eg);
    for _ in 0..n_eg {
        exported_globals.push((d.str()?, d.u32()?, d.val_type()?, d.bool()?, d.i32()?));
    }
    let n_em = d.u32()? as usize;
    let mut exported_mems = Vec::with_capacity(n_em);
    for _ in 0..n_em {
        exported_mems.push((d.str()?, d.u32()?));
    }
    let n_et = d.u32()? as usize;
    let mut exported_tables = Vec::with_capacity(n_et);
    for _ in 0..n_et {
        exported_tables.push((d.str()?, d.u32()?));
    }
    let n_etag = d.u32()? as usize;
    let mut exported_tags = Vec::with_capacity(n_etag);
    for _ in 0..n_etag {
        exported_tags.push((d.str()?, d.u32()?));
    }
    let n_pg = d.u32()? as usize;
    let mut passive_gc_elem_ranges = Vec::with_capacity(n_pg);
    for _ in 0..n_pg {
        passive_gc_elem_ranges.push((d.u32()?, d.i32()?, d.u32()?));
    }
    let elem_dropped_off = if d.bool()? { Some(d.i32()?) } else { None };
    let n_ht = d.u32()? as usize;
    let mut host_tramp_sigs = Vec::with_capacity(n_ht);
    for _ in 0..n_ht {
        let np = d.u32()? as usize;
        let ps = d.take(np)?.to_vec();
        let nr = d.u32()? as usize;
        let rs = d.take(nr)?.to_vec();
        host_tramp_sigs.push((ps, rs));
    }
    let n_ext = d.u32()? as usize;
    let mut export_tramp_sigs = Vec::with_capacity(n_ext);
    for _ in 0..n_ext {
        let np = d.u32()? as usize;
        let ps = d.take(np)?.to_vec();
        let nr = d.u32()? as usize;
        let rs = d.take(nr)?.to_vec();
        export_tramp_sigs.push((ps, rs));
    }
    Ok(StoreModuleMeta {
        abi_version,
        uses_gc,
        uses_exceptions,
        uses_threads,
        ctx_total_bytes,
        gc_elem_buf_bytes,
        n_imports,
        n_defs,
        n_local_canon,
        start_func,
        raw_type_groups,
        type_canon,
        func_type_idx,
        import_order,
        imported_funcs,
        imported_globals,
        imported_mems,
        imported_tables,
        imported_tags,
        defined_globals,
        defined_mems,
        defined_tables,
        defined_tags,
        all_tag_layouts,
        exported_funcs,
        exported_globals,
        exported_mems,
        exported_tables,
        exported_tags,
        passive_gc_elem_ranges,
        elem_dropped_off,
        host_tramp_sigs,
        export_tramp_sigs,
    })
}

fn ref_family_code(f: GcRefFamily) -> u8 {
    match f {
        GcRefFamily::Func => 0,
        GcRefFamily::Extern => 1,
        GcRefFamily::Any => 2,
        GcRefFamily::Exn => 3,
    }
}

/// Appends the `StoreCdylib` relocation tables (def fns, host trampolines, export trampolines)
/// and binary metadata blob into `rodata`.
pub fn build_store_rodata(
    rodata: &mut Vec<u8>,
    module: &X64Module,
    layout: &CtxLayout,
    n_defs: usize,
) -> Result<StoreRodataInfo, String> {
    let align_ro = |buf: &mut Vec<u8>, align: usize| -> Result<u32, String> {
        let rem = buf.len() & (align - 1);
        if rem != 0 {
            buf.resize(buf.len() + (align - rem), 0);
        }
        if buf.len() > super::module::MAX_TOTAL_RODATA_BYTES {
            return Err("x64: StoreCdylib .rodata exceeds cap".into());
        }
        u32::try_from(buf.len()).map_err(|_| "x64: StoreCdylib .rodata offset exceeds u32".into())
    };

    // 1. Deduplicate machine-ABI signatures for BOTH host trampolines and export entry
    //    trampolines across every declared Func type in the module (so ANY funcref, even
    //    one extracted from a table, global, or GC struct, can be called from the Rust host).
    let mut host_sig_map: BTreeMap<(Vec<u8>, Vec<u8>), u32> = BTreeMap::new();
    let mut host_tramp_sigs: Vec<(Vec<Ty>, Vec<Ty>)> = Vec::new();
    let mut host_tramp_codes: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for (ti, fty) in module.module_types.iter().enumerate() {
        if module.type_kinds.get(ti).copied().unwrap_or(0) != 0 {
            continue;
        }
        let ps_ty: Vec<Ty> = fty
            .params
            .iter()
            .map(|&v| val_type_to_machine_ty(v))
            .collect();
        let rs_ty: Vec<Ty> = fty
            .results
            .iter()
            .map(|&v| val_type_to_machine_ty(v))
            .collect();
        let ps_c: Vec<u8> = ps_ty.iter().map(|&t| ty_to_code(t)).collect();
        let rs_c: Vec<u8> = rs_ty.iter().map(|&t| ty_to_code(t)).collect();
        let key = (ps_c.clone(), rs_c.clone());
        if let std::collections::btree_map::Entry::Vacant(e) = host_sig_map.entry(key) {
            let idx = host_tramp_sigs.len() as u32;
            e.insert(idx);
            host_tramp_sigs.push((ps_ty, rs_ty));
            host_tramp_codes.push((ps_c, rs_c));
        }
    }

    let export_tramp_sigs = host_tramp_sigs.clone();
    let export_tramp_codes = host_tramp_codes.clone();
    let mut exported_funcs = Vec::with_capacity(module.exported_funcs.len());
    for (name, fi) in &module.exported_funcs {
        let fty = module
            .func_type(*fi)
            .ok_or_else(|| format!("x64: exported func {fi} missing type"))?;
        let ps_c: Vec<u8> = fty
            .params
            .iter()
            .map(|&v| ty_to_code(val_type_to_machine_ty(v)))
            .collect();
        let rs_c: Vec<u8> = fty
            .results
            .iter()
            .map(|&v| ty_to_code(val_type_to_machine_ty(v)))
            .collect();
        let tramp_idx = *host_sig_map.get(&(ps_c, rs_c)).ok_or_else(|| {
            format!("x64: exported func {fi} signature missing from trampoline table")
        })?;
        exported_funcs.push((name.clone(), *fi as u32, tramp_idx));
    }

    // 2. Assemble `StoreModuleMeta`.
    let n_imp_funcs = module.imports.len();
    let imported_funcs: Vec<(String, String, u32)> = module
        .imports
        .iter()
        .enumerate()
        .map(|(i, (m, n))| (m.clone(), n.clone(), module.func_type_idx[i]))
        .collect();
    let imported_globals: Vec<(String, String, ValType, bool, i32)> = module
        .imported_globals
        .iter()
        .enumerate()
        .map(|(i, (m, n, vt, mut_))| (m.clone(), n.clone(), *vt, *mut_, layout.global_offs[i]))
        .collect();
    let defined_globals: Vec<(u32, ValType, bool, i32, bool)> = module
        .globals
        .iter()
        .enumerate()
        .skip(module.imported_globals.len())
        .map(|(gi, &(vt, mut_))| {
            let off = layout.global_offs[gi];
            let traced = off >= 0 && module.val_ref_kind(vt).is_traced();
            (gi as u32, vt, mut_, off, traced)
        })
        .collect();

    let n_imp_mems = module.imported_mems.len();
    let mut defined_mems = Vec::new();
    let total_mems = usize::from(module.memory.is_some()) + module.extra_memories.len();
    for mi in n_imp_mems..total_mems {
        let dmax = module.all_mem_declared_max.get(mi).copied().flatten();
        if mi == 0 {
            let (min_p, max_p) = module.memory.unwrap();
            defined_mems.push((
                0u32,
                min_p,
                max_p,
                dmax,
                module.memory64,
                module.memory_shared,
            ));
        } else {
            let em = module.extra_memories[mi - 1];
            defined_mems.push((
                mi as u32,
                em.min_pages,
                em.max_pages,
                dmax,
                em.memory64,
                em.shared,
            ));
        }
    }

    let n_imp_tables = module.imported_tables.len();
    let mut defined_tables = Vec::new();
    for ti in n_imp_tables..module.table_lens.len() {
        let t = ti as u32;
        let meta = &module.dyn_tables[&t];
        let tt = module.all_table_types[ti];
        defined_tables.push((
            t,
            meta.min_len,
            meta.eff_max_len,
            meta.is_table64,
            meta.is_funcref,
            ref_family_code(meta.ref_family),
            tt,
        ));
    }

    let n_imp_tags = module.imported_tags.len();
    let defined_tags: Vec<(u32, u32)> = module
        .tags
        .iter()
        .enumerate()
        .skip(n_imp_tags)
        .map(|(i, &ti)| (i as u32, ti))
        .collect();

    let mut all_tag_layouts = Vec::with_capacity(module.tags.len());
    for (ti, &type_idx) in module.tags.iter().enumerate() {
        let fty = &module.module_types[type_idx as usize];
        let (ref fields, rec_sz) = layout.tag_layouts[ti];
        let mut fspecs = Vec::with_capacity(fields.len());
        for (&(ty, foff), &vt) in fields.iter().zip(&fty.params) {
            fspecs.push((ty_to_code(ty), foff, module.val_ref_kind(vt) as u8));
        }
        all_tag_layouts.push((rec_sz, fspecs));
    }

    let exported_globals: Vec<(String, u32, ValType, bool, i32)> = module
        .exported_globals
        .iter()
        .map(|(name, gi, vt, mut_)| {
            (
                name.clone(),
                *gi as u32,
                *vt,
                *mut_,
                layout.global_offs[*gi],
            )
        })
        .collect();

    let mut passive_gc_elem_ranges = Vec::new();
    let gc_elem_buf_bytes = if let Some(ref gc) = layout.gc {
        for (ei, seg) in module.elem_segs.iter().enumerate() {
            if seg.is_passive && seg.ref_family.is_traced() {
                let boff = gc.passive_gc_elem_offs[ei];
                let cnt = seg.gc_entries.as_ref().map_or(0, Vec::len) as u32;
                if boff >= 0 && cnt > 0 {
                    passive_gc_elem_ranges.push((ei as u32, boff, cnt));
                }
            }
        }
        gc.gc_elem_buf_bytes
    } else {
        0
    };

    let meta = StoreModuleMeta {
        abi_version: STORE_ABI_VERSION,
        uses_gc: module.uses_gc,
        uses_exceptions: module.uses_exceptions,
        uses_threads: module.uses_threads,
        ctx_total_bytes: layout.total_bytes as u64,
        gc_elem_buf_bytes,
        n_imports: n_imp_funcs as u32,
        n_defs: n_defs as u32,
        n_local_canon: module.gc_canon_types.len() as u32,
        start_func: module.start,
        raw_type_groups: module.raw_type_groups.clone(),
        type_canon: module.type_canon.clone(),
        func_type_idx: module.func_type_idx.clone(),
        import_order: module.import_order.clone(),
        imported_funcs,
        imported_globals,
        imported_mems: module.imported_mems.clone(),
        imported_tables: module.imported_tables.clone(),
        imported_tags: module.imported_tags.clone(),
        defined_globals,
        defined_mems,
        defined_tables,
        defined_tags,
        all_tag_layouts,
        exported_funcs,
        exported_globals,
        exported_mems: module.exported_mems.clone(),
        exported_tables: module.exported_tables.clone(),
        exported_tags: module.exported_tags.clone(),
        passive_gc_elem_ranges,
        elem_dropped_off: layout.elem_dropped_off,
        host_tramp_sigs: host_tramp_codes,
        export_tramp_sigs: export_tramp_codes,
    };
    let meta_bytes = encode_store_metadata(&meta);

    // 3. Reserve `.rodata` tables (patched post-link once `.text` offsets are final).
    let def_fn_rel_ro_off = align_ro(rodata, 4)?;
    rodata.resize(rodata.len() + n_defs * 4, 0);
    let host_tramp_rel_ro_off = align_ro(rodata, 4)?;
    rodata.resize(rodata.len() + host_tramp_sigs.len() * 4, 0);
    let export_tramp_rel_ro_off = align_ro(rodata, 4)?;
    rodata.resize(rodata.len() + export_tramp_sigs.len() * 4, 0);
    let meta_ro_off = align_ro(rodata, 8)?;
    let meta_len = meta_bytes.len() as u64;
    if rodata.len().saturating_add(meta_bytes.len()) > super::module::MAX_TOTAL_RODATA_BYTES {
        return Err("x64: StoreCdylib metadata exceeds .rodata cap".into());
    }
    rodata.extend_from_slice(&meta_bytes);

    Ok(StoreRodataInfo {
        meta_ro_off,
        meta_len,
        def_fn_rel_ro_off,
        host_tramp_rel_ro_off,
        export_tramp_rel_ro_off,
        host_tramp_sigs,
        export_tramp_sigs,
    })
}

/// Emits the single ELF export `wt_store_get_desc(out: *mut StoreModuleDesc)`.
///
/// `StoreModuleDesc` (`#[repr(C)]`, 96 bytes):
///   `+0`:  abi_version: u32
///   `+4`:  flags: u32 (bit0=uses_gc, bit1=uses_exceptions, bit2=uses_threads)
///   `+8`:  ctx_total_bytes: u64
///   `+16`: code_start: *const u8
///   `+24`: code_end: *const u8
///   `+32`: safepoint_header: *const u8 (0 if !uses_gc)
///   `+40`: rodata_base: *const u8
///   `+48`: metadata_ptr: *const u8
///   `+56`: metadata_len: u64
///   `+64`: init_instance_fn: *const u8
///   `+72`: def_fn_table: *const i32
///   `+80`: host_tramp_table: *const i32
///   `+88`: export_tramp_table: *const i32
pub fn emit_store_get_desc(
    module: &X64Module,
    layout: &CtxLayout,
    store_ro: &StoreRodataInfo,
) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(256);
    let flags = u32::from(module.uses_gc)
        | (u32::from(module.uses_exceptions) << 1)
        | (u32::from(module.uses_threads) << 2);
    a.mov_mi32(false, Mem::base_disp(Gp::Rdi, 0), STORE_ABI_VERSION as i32);
    a.mov_mi32(false, Mem::base_disp(Gp::Rdi, 4), flags as i32);
    a.mov_ri64(Gp::Rax, layout.total_bytes as u64);
    a.mov_mr(true, Mem::base_disp(Gp::Rdi, 8), Gp::Rax);

    a.lea_rip(Gp::Rax, RelocTarget::GuestCodeStart);
    a.mov_mr(true, Mem::base_disp(Gp::Rdi, 16), Gp::Rax);
    a.lea_rip(Gp::Rax, RelocTarget::GuestCodeEnd);
    a.mov_mr(true, Mem::base_disp(Gp::Rdi, 24), Gp::Rax);
    if module.uses_gc {
        a.lea_rip(Gp::Rax, RelocTarget::GcSafepointHeader);
    } else {
        a.xor_rr(false, Gp::Rax, Gp::Rax);
    }
    a.mov_mr(true, Mem::base_disp(Gp::Rdi, 32), Gp::Rax);
    a.lea_rip(Gp::Rax, RelocTarget::Rodata(0));
    a.mov_mr(true, Mem::base_disp(Gp::Rdi, 40), Gp::Rax);
    a.lea_rip(Gp::Rax, RelocTarget::Rodata(store_ro.meta_ro_off));
    a.mov_mr(true, Mem::base_disp(Gp::Rdi, 48), Gp::Rax);
    a.mov_ri64(Gp::Rax, store_ro.meta_len);
    a.mov_mr(true, Mem::base_disp(Gp::Rdi, 56), Gp::Rax);
    a.lea_rip(Gp::Rax, RelocTarget::Helper(HELPER_INITIALIZE));
    a.mov_mr(true, Mem::base_disp(Gp::Rdi, 64), Gp::Rax);
    a.lea_rip(Gp::Rax, RelocTarget::Rodata(store_ro.def_fn_rel_ro_off));
    a.mov_mr(true, Mem::base_disp(Gp::Rdi, 72), Gp::Rax);
    a.lea_rip(Gp::Rax, RelocTarget::Rodata(store_ro.host_tramp_rel_ro_off));
    a.mov_mr(true, Mem::base_disp(Gp::Rdi, 80), Gp::Rax);
    a.lea_rip(
        Gp::Rax,
        RelocTarget::Rodata(store_ro.export_tramp_rel_ro_off),
    );
    a.mov_mr(true, Mem::base_disp(Gp::Rdi, 88), Gp::Rax);
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

fn emit_save_xmm0_15(a: &mut Asm, base_off: i32) {
    for i in 0u8..16 {
        a.movdqu_mx(
            Mem::base_disp(Gp::Rsp, base_off + i32::from(i) * 16),
            Xmm::from_id(i),
        );
    }
}

fn emit_restore_xmm0_15(a: &mut Asm, base_off: i32) {
    for i in 0u8..16 {
        a.movdqu_xm(
            Xmm::from_id(i),
            Mem::base_disp(Gp::Rsp, base_off + i32::from(i) * 16),
        );
    }
}

fn emit_ud2_stub() -> Asm {
    let mut a = Asm::with_capacity(4);
    a.ud2();
    a
}

/// Emits all `NUM_HELPERS` runtime helpers for `X64Target::StoreCdylib`.
pub fn emit_store_helpers(
    module: &X64Module,
    layout: &CtxLayout,
    ro: &RodataConsts,
    tables: &RuntimeTables,
) -> Result<Vec<Asm>, String> {
    let trunc_ops = [
        TruncOp::I32F32S,
        TruncOp::I32F32U,
        TruncOp::I32F64S,
        TruncOp::I32F64U,
        TruncOp::I64F32S,
        TruncOp::I64F32U,
        TruncOp::I64F64S,
        TruncOp::I64F64U,
        TruncOp::I32F32SatS,
        TruncOp::I32F32SatU,
        TruncOp::I32F64SatS,
        TruncOp::I32F64SatU,
        TruncOp::I64F32SatS,
        TruncOp::I64F32SatU,
        TruncOp::I64F64SatS,
        TruncOp::I64F64SatU,
    ];
    let mut helpers = Vec::with_capacity(NUM_HELPERS);
    for (i, &op) in trunc_ops.iter().enumerate() {
        debug_assert_eq!(HELPER_TRUNC_BASE + i as u32, op as u32);
        helpers.push(emit_trunc_helper(op, ro)?);
    }

    // 16: HELPER_MEM_GROW
    debug_assert_eq!(helpers.len(), HELPER_MEM_GROW as usize);
    helpers.push(emit_store_mem_grow(module.memory64)?);
    // 17: HELPER_MEM_COPY
    debug_assert_eq!(helpers.len(), HELPER_MEM_COPY as usize);
    helpers.push(emit_store_mem_copy(module.memory64)?);
    // 18: HELPER_MEM_FILL
    debug_assert_eq!(helpers.len(), HELPER_MEM_FILL as usize);
    helpers.push(emit_store_mem_fill(module.memory64)?);
    // 19: HELPER_HOST_CALL (unused in StoreCdylib)
    helpers.push(emit_ud2_stub());
    // 20: HELPER_GROW_CORE (unused in StoreCdylib)
    helpers.push(emit_ud2_stub());
    // 21: HELPER_INITIALIZE -> `wt_store_init_instance(ctx: *mut u8) -> u32`
    debug_assert_eq!(helpers.len(), HELPER_INITIALIZE as usize);
    helpers.push(emit_store_init_instance(module, layout, ro, tables)?);
    // 22: HELPER_READ_GLOBAL (unused)
    helpers.push(emit_ud2_stub());
    // 23: HELPER_GOOSE_FMT_FLOAT (unused)
    helpers.push(emit_ud2_stub());
    // 24: HELPER_EXTRA_MEM_GROW
    debug_assert_eq!(helpers.len(), HELPER_EXTRA_MEM_GROW as usize);
    helpers.push(emit_store_extra_mem_grow()?);
    // 25: HELPER_EXTRA_MEM_COPY
    debug_assert_eq!(helpers.len(), HELPER_EXTRA_MEM_COPY as usize);
    helpers.push(emit_store_extra_mem_copy()?);
    // 26: HELPER_EXTRA_MEM_FILL
    debug_assert_eq!(helpers.len(), HELPER_EXTRA_MEM_FILL as usize);
    helpers.push(emit_store_extra_mem_fill()?);
    // 27..=28: standalone FS/reserve (unused)
    helpers.push(emit_ud2_stub());
    helpers.push(emit_ud2_stub());
    // 29: HELPER_MEM_INIT
    debug_assert_eq!(helpers.len(), HELPER_MEM_INIT as usize);
    if module.uses_bulk_data {
        helpers.push(emit_store_mem_init(module, layout, tables)?);
    } else {
        helpers.push(emit_ud2_stub());
    }
    // 30..=33: table helpers
    debug_assert_eq!(helpers.len(), HELPER_TABLE_INIT as usize);
    if module.uses_bulk_elem {
        helpers.push(emit_store_table_init(module, layout, tables)?);
    } else {
        helpers.push(emit_ud2_stub());
    }
    debug_assert_eq!(helpers.len(), HELPER_TABLE_COPY as usize);
    helpers.push(emit_store_table_copy()?);
    debug_assert_eq!(helpers.len(), HELPER_TABLE_GROW as usize);
    helpers.push(emit_store_table_grow()?);
    debug_assert_eq!(helpers.len(), HELPER_TABLE_FILL as usize);
    helpers.push(emit_store_table_fill()?);

    // 34..=39: GC helpers
    debug_assert_eq!(helpers.len(), HELPER_GC_COLLECT as usize);
    if module.uses_gc {
        helpers.push(emit_store_gc_collect_stub()?);
        debug_assert_eq!(helpers.len(), HELPER_GC_ARRAY_FILL_RAW as usize);
        helpers.push(gc_rt::emit_helper_gc_array_fill_raw()?);
        debug_assert_eq!(helpers.len(), HELPER_GC_ARRAY_FILL as usize);
        helpers.push(gc_rt::emit_helper_gc_array_fill()?);
        debug_assert_eq!(helpers.len(), HELPER_GC_ARRAY_COPY as usize);
        helpers.push(gc_rt::emit_helper_gc_array_copy()?);
        debug_assert_eq!(helpers.len(), HELPER_GC_ARRAY_INIT_DATA as usize);
        if let Some(drop_off) = layout.data_dropped_off {
            helpers.push(gc_rt::emit_helper_gc_array_init_data(
                tables.passive_data_table_off,
                drop_off,
            )?);
        } else {
            helpers.push(emit_ud2_stub());
        }
        debug_assert_eq!(helpers.len(), HELPER_GC_ARRAY_INIT_ELEM as usize);
        helpers.push(emit_store_gc_array_init_elem(module, layout)?);
    } else {
        for _ in 0..6 {
            helpers.push(emit_ud2_stub());
        }
    }

    // 40..=41: Atomic wait/notify
    debug_assert_eq!(helpers.len(), HELPER_ATOMIC_WAIT as usize);
    if module.uses_threads {
        helpers.push(emit_store_atomic_wait()?);
        debug_assert_eq!(helpers.len(), HELPER_ATOMIC_NOTIFY as usize);
        helpers.push(emit_store_atomic_notify()?);
    } else {
        helpers.push(emit_ud2_stub());
        helpers.push(emit_ud2_stub());
    }

    // 42: HELPER_STORE_EXN_ALLOC
    debug_assert_eq!(helpers.len(), HELPER_STORE_EXN_ALLOC as usize);
    if module.uses_exceptions {
        helpers.push(emit_store_exn_alloc_stub()?);
    } else {
        helpers.push(emit_ud2_stub());
    }

    debug_assert_eq!(helpers.len(), NUM_HELPERS);
    Ok(helpers)
}

/// Preserve-all wrapper for `HELPER_GC_COLLECT`:
/// Entry: `R10 = req_bytes`, `R15 = Ctx*`, `[rsp]` = caller Wasm return address.
/// Calls `store->gc_collect_fn(store_ptr, req_bytes, gc_call_ret_rsp) -> u32`.
/// Returns trap code in `R11d`; preserves ALL GP regs (`Rax..R15` except `R11`) and `Xmm0..15`.
fn emit_store_gc_collect_stub() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(256);
    // Entry `rsp % 16 == 8`. Push 9 GPs (72B) => `rsp % 16 == 0`; `gc_call_ret_rsp = rsp + 72`.
    let saved_gps = [
        Gp::Rax,
        Gp::Rcx,
        Gp::Rdx,
        Gp::Rsi,
        Gp::Rdi,
        Gp::R8,
        Gp::R9,
        Gp::R10,
        Gp::Rbp,
    ];
    for &g in &saved_gps {
        a.push(g);
    }
    a.lea(
        true,
        Gp::Rdx,
        Mem::base_disp(Gp::Rsp, (saved_gps.len() as i32) * 8),
    );
    a.mov_rr(true, Gp::Rsi, Gp::R10);
    a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::R15, OFF_STORE_PTR));
    a.sub_ri(true, Gp::Rsp, 256);
    emit_save_xmm0_15(&mut a, 0);
    a.mov_rm(
        true,
        Gp::Rax,
        Mem::base_disp(Gp::Rdi, STORE_OFF_GC_COLLECT_FN),
    );
    a.call_reg(Gp::Rax);
    a.mov_rr(false, Gp::R11, Gp::Rax);
    emit_restore_xmm0_15(&mut a, 0);
    a.add_ri(true, Gp::Rsp, 256);
    for &g in saved_gps.iter().rev() {
        a.pop(g);
    }
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// Preserve-all wrapper for `HELPER_STORE_EXN_ALLOC`:
/// Entry: `R11d = global_tag_id`, `R10d = rec_size`, `R15 = Ctx*`.
/// Calls `store->exn_alloc_fn(store_ptr, global_tag_id, rec_size) -> *mut u8`.
/// Returns `R10 = exn_ptr`, `R11d = 0 | TRAP_ALLOC_FAILED`; preserves all other GPs and `Xmm0..15`.
fn emit_store_exn_alloc_stub() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(256);
    let lbl_ok = a.new_label();
    let saved_gps = [
        Gp::Rax,
        Gp::Rcx,
        Gp::Rdx,
        Gp::Rsi,
        Gp::Rdi,
        Gp::R8,
        Gp::R9,
        Gp::Rbp,
    ];
    for &g in &saved_gps {
        a.push(g);
    }
    // 8 (ret) + 64 (8 pushes) + 264 = 336 (16B aligned!).
    a.sub_ri(true, Gp::Rsp, 264);
    emit_save_xmm0_15(&mut a, 0);
    a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::R15, OFF_STORE_PTR));
    a.mov_rr(false, Gp::Rsi, Gp::R11);
    a.mov_rr(false, Gp::Rdx, Gp::R10);
    a.mov_rm(
        true,
        Gp::Rax,
        Mem::base_disp(Gp::Rdi, STORE_OFF_EXN_ALLOC_FN),
    );
    a.call_reg(Gp::Rax);
    a.mov_rr(true, Gp::R10, Gp::Rax);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.test_rr(true, Gp::R10, Gp::R10);
    a.jcc(Cond::Ne, lbl_ok);
    a.mov_ri32(Gp::R11, u32::from(TRAP_ALLOC_FAILED));
    a.bind(lbl_ok);
    emit_restore_xmm0_15(&mut a, 0);
    a.add_ri(true, Gp::Rsp, 264);
    for &g in saved_gps.iter().rev() {
        a.pop(g);
    }
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// `HELPER_MEM_GROW`: `Rdi = delta_pages`, `R15 = Ctx*` -> `Rax = old_pages | -1`, updates `R13`.
fn emit_store_mem_grow(memory64: bool) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(192);
    a.sub_ri(true, Gp::Rsp, 264);
    emit_save_xmm0_15(&mut a, 0);
    if !memory64 {
        a.mov_rr(false, Gp::Rdx, Gp::Rdi);
    } else {
        a.mov_rr(true, Gp::Rdx, Gp::Rdi);
    }
    a.mov_rm(
        true,
        Gp::Rsi,
        Mem::base_disp(Gp::R15, OFF_MEM_RESERVE_BYTES),
    );
    a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::R15, OFF_STORE_PTR));
    a.mov_rm(
        true,
        Gp::Rax,
        Mem::base_disp(Gp::Rdi, STORE_OFF_MEM_GROW_FN),
    );
    a.call_reg(Gp::Rax);
    // Reload live `R13 = mem0_desc->len`.
    a.mov_rm(
        true,
        Gp::R10,
        Mem::base_disp(Gp::R15, OFF_MEM_RESERVE_BYTES),
    );
    a.mov_rm(true, Gp::R13, Mem::base_disp(Gp::R10, 8));
    if !memory64 {
        a.mov_rr(false, Gp::Rax, Gp::Rax);
    }
    emit_restore_xmm0_15(&mut a, 0);
    a.add_ri(true, Gp::Rsp, 264);
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// `HELPER_EXTRA_MEM_GROW`: `Rdi = delta_pages`, `Rsi = mem_idx`, `R15 = Ctx*` -> `Rax = old_pages | -1`.
fn emit_store_extra_mem_grow() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(224);
    let lbl_not0 = a.new_label();
    a.push(Gp::Rbx);
    // 8 (ret) + 8 (rbx) + 256 = 272 (16B aligned!).
    a.sub_ri(true, Gp::Rsp, 256);
    emit_save_xmm0_15(&mut a, 0);
    a.mov_rr(false, Gp::Rbx, Gp::Rsi); // save mem_idx across C call
    a.mov_rr(true, Gp::Rdx, Gp::Rdi);
    a.mov_rm(true, Gp::R10, Mem::base_disp(Gp::R15, OFF_STORE_MEM_DESCS));
    a.mov_rm(true, Gp::Rsi, Mem::indexed(Gp::R10, Gp::Rbx, 8, 0));
    a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::R15, OFF_STORE_PTR));
    a.mov_rm(
        true,
        Gp::Rax,
        Mem::base_disp(Gp::Rdi, STORE_OFF_MEM_GROW_FN),
    );
    a.call_reg(Gp::Rax);
    a.test_rr(false, Gp::Rbx, Gp::Rbx);
    a.jcc(Cond::Ne, lbl_not0);
    a.mov_rm(
        true,
        Gp::R10,
        Mem::base_disp(Gp::R15, OFF_MEM_RESERVE_BYTES),
    );
    a.mov_rm(true, Gp::R13, Mem::base_disp(Gp::R10, 8));
    a.bind(lbl_not0);
    emit_restore_xmm0_15(&mut a, 0);
    a.add_ri(true, Gp::Rsp, 256);
    a.pop(Gp::Rbx);
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// `HELPER_MEM_COPY`: `Rdi = dst, Rsi = src, Rdx = len` on memory 0 -> `R11d = 0 | TRAP_OOB`.
fn emit_store_mem_copy(memory64: bool) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(160);
    let lbl_oob = a.new_label();
    let lbl_bwd = a.new_label();
    let lbl_done = a.new_label();
    if !memory64 {
        a.mov_rr(false, Gp::Rdi, Gp::Rdi);
        a.mov_rr(false, Gp::Rsi, Gp::Rsi);
        a.mov_rr(false, Gp::Rdx, Gp::Rdx);
    }
    a.mov_rm(
        true,
        Gp::R10,
        Mem::base_disp(Gp::R15, OFF_MEM_RESERVE_BYTES),
    );
    a.mov_rm(true, Gp::R13, Mem::base_disp(Gp::R10, 8));
    a.mov_rr(true, Gp::Rax, Gp::Rdi);
    a.add_rr(true, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rr(true, Gp::Rax, Gp::R13);
    a.jcc(Cond::A, lbl_oob);
    a.mov_rr(true, Gp::Rax, Gp::Rsi);
    a.add_rr(true, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rr(true, Gp::Rax, Gp::R13);
    a.jcc(Cond::A, lbl_oob);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_done);
    a.add_rr(true, Gp::Rdi, Gp::R14);
    a.add_rr(true, Gp::Rsi, Gp::R14);
    a.mov_rr(true, Gp::Rcx, Gp::Rdx);
    a.cmp_rr(true, Gp::Rdi, Gp::Rsi);
    a.jcc(Cond::A, lbl_bwd);
    a.rep_movsb();
    a.jmp(lbl_done);
    a.bind(lbl_bwd);
    a.lea(true, Gp::Rdi, Mem::indexed(Gp::Rdi, Gp::Rcx, 1, -1));
    a.lea(true, Gp::Rsi, Mem::indexed(Gp::Rsi, Gp::Rcx, 1, -1));
    a.std_dir();
    a.rep_movsb();
    a.cld();
    a.bind(lbl_done);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();
    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_OOB));
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// `HELPER_MEM_FILL`: `Rdi = dst, Rsi = val, Rdx = len` on memory 0 -> `R11d = 0 | TRAP_OOB`.
fn emit_store_mem_fill(memory64: bool) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(128);
    let lbl_oob = a.new_label();
    let lbl_done = a.new_label();
    if !memory64 {
        a.mov_rr(false, Gp::Rdi, Gp::Rdi);
        a.mov_rr(false, Gp::Rdx, Gp::Rdx);
    }
    a.mov_rm(
        true,
        Gp::R10,
        Mem::base_disp(Gp::R15, OFF_MEM_RESERVE_BYTES),
    );
    a.mov_rm(true, Gp::R13, Mem::base_disp(Gp::R10, 8));
    a.mov_rr(true, Gp::Rax, Gp::Rdi);
    a.add_rr(true, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rr(true, Gp::Rax, Gp::R13);
    a.jcc(Cond::A, lbl_oob);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_done);
    a.add_rr(true, Gp::Rdi, Gp::R14);
    a.mov_rr(false, Gp::Rax, Gp::Rsi);
    a.mov_rr(true, Gp::Rcx, Gp::Rdx);
    a.rep_stosb();
    a.bind(lbl_done);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();
    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_OOB));
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// `HELPER_EXTRA_MEM_COPY`: `Rdi = dst, Rsi = src, Rdx = len, Rcx = dst_mem, R8 = src_mem`.
fn emit_store_extra_mem_copy() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(200);
    let lbl_oob = a.new_label();
    let lbl_bwd = a.new_label();
    let lbl_done = a.new_label();
    a.mov_rm(true, Gp::R10, Mem::base_disp(Gp::R15, OFF_STORE_MEM_DESCS));
    a.mov_rr(false, Gp::Rcx, Gp::Rcx);
    a.mov_rr(false, Gp::R8, Gp::R8);
    a.mov_rm(true, Gp::R9, Mem::indexed(Gp::R10, Gp::Rcx, 8, 0)); // dst_desc
    a.mov_rm(true, Gp::R10, Mem::indexed(Gp::R10, Gp::R8, 8, 0)); // src_desc
    a.mov_rr(true, Gp::Rax, Gp::Rdi);
    a.add_rr(true, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rm(true, Gp::Rax, Mem::base_disp(Gp::R9, 8));
    a.jcc(Cond::A, lbl_oob);
    a.mov_rr(true, Gp::Rax, Gp::Rsi);
    a.add_rr(true, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rm(true, Gp::Rax, Mem::base_disp(Gp::R10, 8));
    a.jcc(Cond::A, lbl_oob);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_done);
    a.add_rm(true, Gp::Rdi, Mem::base_disp(Gp::R9, 0));
    a.add_rm(true, Gp::Rsi, Mem::base_disp(Gp::R10, 0));
    a.mov_rr(true, Gp::Rcx, Gp::Rdx);
    a.cmp_rr(true, Gp::Rdi, Gp::Rsi);
    a.jcc(Cond::A, lbl_bwd);
    a.rep_movsb();
    a.jmp(lbl_done);
    a.bind(lbl_bwd);
    a.lea(true, Gp::Rdi, Mem::indexed(Gp::Rdi, Gp::Rcx, 1, -1));
    a.lea(true, Gp::Rsi, Mem::indexed(Gp::Rsi, Gp::Rcx, 1, -1));
    a.std_dir();
    a.rep_movsb();
    a.cld();
    a.bind(lbl_done);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();
    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_OOB));
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// `HELPER_EXTRA_MEM_FILL`: `Rdi = dst, Rsi = val, Rdx = len, Rcx = mem_idx`.
fn emit_store_extra_mem_fill() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(144);
    let lbl_oob = a.new_label();
    let lbl_done = a.new_label();
    a.mov_rm(true, Gp::R10, Mem::base_disp(Gp::R15, OFF_STORE_MEM_DESCS));
    a.mov_rr(false, Gp::Rcx, Gp::Rcx);
    a.mov_rm(true, Gp::R9, Mem::indexed(Gp::R10, Gp::Rcx, 8, 0)); // mem_desc
    a.mov_rr(true, Gp::Rax, Gp::Rdi);
    a.add_rr(true, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rm(true, Gp::Rax, Mem::base_disp(Gp::R9, 8));
    a.jcc(Cond::A, lbl_oob);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_done);
    a.add_rm(true, Gp::Rdi, Mem::base_disp(Gp::R9, 0));
    a.mov_rr(false, Gp::Rax, Gp::Rsi);
    a.mov_rr(true, Gp::Rcx, Gp::Rdx);
    a.rep_stosb();
    a.bind(lbl_done);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();
    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_OOB));
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// `HELPER_MEM_INIT`: `Rdi = dst, Rsi = src, Rdx = len, Rcx = data_idx, R8 = mem_idx`.
fn emit_store_mem_init(
    _module: &X64Module,
    layout: &CtxLayout,
    tables: &RuntimeTables,
) -> Result<Asm, String> {
    let drop_off = layout
        .data_dropped_off
        .ok_or("x64: StoreCdylib HELPER_MEM_INIT missing data_dropped_off")?;
    let mut a = Asm::with_capacity(200);
    let lbl_oob = a.new_label();
    let lbl_not_dropped = a.new_label();
    let lbl_cmp_src = a.new_label();
    let lbl_done = a.new_label();
    a.mov_rr(false, Gp::Rcx, Gp::Rcx); // data_idx
    a.mov_rr(false, Gp::R8, Gp::R8); // mem_idx
    a.mov_rm(true, Gp::R10, Mem::base_disp(Gp::R15, OFF_STORE_MEM_DESCS));
    a.mov_rm(true, Gp::R9, Mem::indexed(Gp::R10, Gp::R8, 8, 0)); // mem_desc
    // Check dst + len <= mem_desc->len
    a.mov_rr(true, Gp::Rax, Gp::Rdi);
    a.add_rr(true, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rm(true, Gp::Rax, Mem::base_disp(Gp::R9, 8));
    a.jcc(Cond::A, lbl_oob);
    // Check dropped flag & src + len <= seg_len
    a.movzx_rm8(false, Gp::R10, Mem::indexed(Gp::R15, Gp::Rcx, 1, drop_off));
    a.test_rr(false, Gp::R10, Gp::R10);
    a.jcc(Cond::E, lbl_not_dropped);
    a.xor_rr(false, Gp::R10, Gp::R10); // dropped => seg_len = 0
    a.jmp(lbl_cmp_src);
    a.bind(lbl_not_dropped);
    a.lea_rip(Gp::R11, RelocTarget::Rodata(tables.passive_data_table_off));
    a.mov_rm(false, Gp::R10, Mem::indexed(Gp::R11, Gp::Rcx, 8, 4)); // seg_len
    a.bind(lbl_cmp_src);
    a.mov_rr(true, Gp::Rax, Gp::Rsi);
    a.add_rr(true, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rr(true, Gp::Rax, Gp::R10);
    a.jcc(Cond::A, lbl_oob);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_done);
    a.add_rm(true, Gp::Rdi, Mem::base_disp(Gp::R9, 0));
    a.movsxd_rm(Gp::Rax, Mem::indexed(Gp::R11, Gp::Rcx, 8, 0));
    a.add_rr(true, Gp::R11, Gp::Rax); // seg_base
    a.add_rr(true, Gp::Rsi, Gp::R11);
    a.mov_rr(true, Gp::Rcx, Gp::Rdx);
    a.rep_movsb();
    a.bind(lbl_done);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();
    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_OOB));
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// `HELPER_TABLE_GROW`: `Rdi = init_val, Rsi = delta, Rdx = table_idx` -> `Rax = old_len | -1`.
fn emit_store_table_grow() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(192);
    a.sub_ri(true, Gp::Rsp, 264);
    emit_save_xmm0_15(&mut a, 0);
    // C ABI: table_grow_fn(store_ptr [Rdi], tbl_desc [Rsi], init_val [Rdx], delta [Rcx])
    a.mov_rr(true, Gp::Rcx, Gp::Rsi); // delta
    a.mov_rr(false, Gp::R10, Gp::Rdx); // tbl_idx
    a.mov_rr(true, Gp::Rdx, Gp::Rdi); // init_val
    a.mov_rm(
        true,
        Gp::R11,
        Mem::base_disp(Gp::R15, OFF_STORE_TABLE_DESCS),
    );
    a.mov_rm(true, Gp::Rsi, Mem::indexed(Gp::R11, Gp::R10, 8, 0)); // tbl_desc
    a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::R15, OFF_STORE_PTR));
    a.mov_rm(
        true,
        Gp::Rax,
        Mem::base_disp(Gp::Rdi, STORE_OFF_TABLE_GROW_FN),
    );
    a.call_reg(Gp::Rax);
    emit_restore_xmm0_15(&mut a, 0);
    a.add_ri(true, Gp::Rsp, 264);
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// `HELPER_TABLE_FILL`: `Rdi = dst, Rsi = val, Rdx = len, Rcx = tbl_idx` -> `R11d = 0 | TRAP_TABLE_OOB`.
fn emit_store_table_fill() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(144);
    let lbl_oob = a.new_label();
    let lbl_lp = a.new_label();
    let lbl_done = a.new_label();
    a.mov_rr(false, Gp::Rcx, Gp::Rcx);
    a.mov_rm(
        true,
        Gp::R10,
        Mem::base_disp(Gp::R15, OFF_STORE_TABLE_DESCS),
    );
    a.mov_rm(true, Gp::R9, Mem::indexed(Gp::R10, Gp::Rcx, 8, 0));
    a.mov_rm(false, Gp::R10, Mem::base_disp(Gp::R9, 8)); // cur_len zero-ext u64
    a.mov_rr(true, Gp::Rax, Gp::Rdi);
    a.add_rr(true, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rr(true, Gp::Rax, Gp::R10);
    a.jcc(Cond::A, lbl_oob);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_done);
    a.mov_rm(true, Gp::R8, Mem::base_disp(Gp::R9, 0));
    a.lea(true, Gp::R8, Mem::indexed(Gp::R8, Gp::Rdi, 8, 0));
    a.bind(lbl_lp);
    a.mov_mr(true, Mem::base(Gp::R8), Gp::Rsi);
    a.add_ri(true, Gp::R8, 8);
    a.sub_ri(true, Gp::Rdx, 1);
    a.jcc(Cond::Ne, lbl_lp);
    a.bind(lbl_done);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();
    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_TABLE_OOB));
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// `HELPER_TABLE_COPY`: `Rdi = dst, Rsi = src, Rdx = len, Rcx = dst_tbl, R8 = src_tbl`.
fn emit_store_table_copy() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(200);
    let lbl_oob = a.new_label();
    let lbl_bwd = a.new_label();
    let lbl_done = a.new_label();
    a.mov_rr(false, Gp::Rcx, Gp::Rcx);
    a.mov_rr(false, Gp::R8, Gp::R8);
    a.mov_rm(
        true,
        Gp::R10,
        Mem::base_disp(Gp::R15, OFF_STORE_TABLE_DESCS),
    );
    a.mov_rm(true, Gp::R9, Mem::indexed(Gp::R10, Gp::Rcx, 8, 0)); // dst_desc
    a.mov_rm(true, Gp::R10, Mem::indexed(Gp::R10, Gp::R8, 8, 0)); // src_desc
    a.mov_rm(false, Gp::Rax, Mem::base_disp(Gp::R9, 8)); // dst_len
    a.mov_rr(true, Gp::Rcx, Gp::Rdi);
    a.add_rr(true, Gp::Rcx, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rr(true, Gp::Rcx, Gp::Rax);
    a.jcc(Cond::A, lbl_oob);
    a.mov_rm(false, Gp::Rax, Mem::base_disp(Gp::R10, 8)); // src_len
    a.mov_rr(true, Gp::Rcx, Gp::Rsi);
    a.add_rr(true, Gp::Rcx, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rr(true, Gp::Rcx, Gp::Rax);
    a.jcc(Cond::A, lbl_oob);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_done);
    a.shl_imm(true, Gp::Rdi, 3);
    a.add_rm(true, Gp::Rdi, Mem::base_disp(Gp::R9, 0));
    a.shl_imm(true, Gp::Rsi, 3);
    a.add_rm(true, Gp::Rsi, Mem::base_disp(Gp::R10, 0));
    a.mov_rr(true, Gp::Rcx, Gp::Rdx);
    a.shl_imm(true, Gp::Rcx, 3); // byte len
    a.cmp_rr(true, Gp::Rdi, Gp::Rsi);
    a.jcc(Cond::A, lbl_bwd);
    a.rep_movsb();
    a.jmp(lbl_done);
    a.bind(lbl_bwd);
    a.lea(true, Gp::Rdi, Mem::indexed(Gp::Rdi, Gp::Rcx, 1, -1));
    a.lea(true, Gp::Rsi, Mem::indexed(Gp::Rsi, Gp::Rcx, 1, -1));
    a.std_dir();
    a.rep_movsb();
    a.cld();
    a.bind(lbl_done);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();
    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_TABLE_OOB));
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// `HELPER_TABLE_INIT`: `Rdi = dst, Rsi = src, Rdx = len, Rcx = elem_idx, R8 = tbl_idx`.
/// Translates static `u32` funcref entries (`enc -> StoreFuncRef*`) and copies passive GC elem entries directly.
fn emit_store_table_init(
    module: &X64Module,
    layout: &CtxLayout,
    tables: &RuntimeTables,
) -> Result<Asm, String> {
    let drop_off = layout
        .elem_dropped_off
        .ok_or("x64: StoreCdylib HELPER_TABLE_INIT missing elem_dropped_off")?;
    let mut a = Asm::with_capacity(280);
    let lbl_oob = a.new_label();
    let lbl_not_dropped = a.new_label();
    let lbl_cmp_src = a.new_label();
    let lbl_done = a.new_label();
    let lbl_gc_copy = a.new_label();
    let lbl_gc_lp = a.new_label();
    let lbl_fn_lp = a.new_label();
    let lbl_fn_store = a.new_label();

    a.mov_rr(false, Gp::Rcx, Gp::Rcx); // elem_idx
    a.mov_rr(false, Gp::R8, Gp::R8); // tbl_idx
    a.mov_rm(
        true,
        Gp::R10,
        Mem::base_disp(Gp::R15, OFF_STORE_TABLE_DESCS),
    );
    a.mov_rm(true, Gp::R9, Mem::indexed(Gp::R10, Gp::R8, 8, 0)); // tbl_desc
    // Check dst + len <= tbl_desc->cur_len
    a.mov_rm(false, Gp::R10, Mem::base_disp(Gp::R9, 8));
    a.mov_rr(true, Gp::Rax, Gp::Rdi);
    a.add_rr(true, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rr(true, Gp::Rax, Gp::R10);
    a.jcc(Cond::A, lbl_oob);
    // Check dropped flag
    a.movzx_rm8(false, Gp::R10, Mem::indexed(Gp::R15, Gp::Rcx, 1, drop_off));
    a.test_rr(false, Gp::R10, Gp::R10);
    a.jcc(Cond::E, lbl_not_dropped);
    a.xor_rr(false, Gp::R10, Gp::R10);
    a.jmp(lbl_cmp_src);

    a.bind(lbl_not_dropped);
    if let Some(ref gc) = layout.gc
        && gc.gc_elem_desc_ro_off != 0
    {
        // `GcElemDesc[elem_idx]` (16B): +0 ro_rel, +4 elem_len, +8 gc_buf_off (i32), +12 is_traced.
        a.lea_rip(Gp::R11, RelocTarget::Rodata(gc.gc_elem_desc_ro_off));
        a.mov_rr(true, Gp::R8, Gp::Rcx);
        a.shl_imm(true, Gp::R8, 4);
        a.mov_rm(false, Gp::R10, Mem::indexed(Gp::R11, Gp::R8, 1, 4)); // elem_len
    } else {
        let _ = module;
        a.lea_rip(Gp::R11, RelocTarget::Rodata(tables.elem_seg_table_off));
        a.mov_rm(false, Gp::R10, Mem::indexed(Gp::R11, Gp::Rcx, 8, 4));
    }

    a.bind(lbl_cmp_src);
    a.mov_rr(true, Gp::Rax, Gp::Rsi);
    a.add_rr(true, Gp::Rax, Gp::Rdx);
    a.jcc(Cond::B, lbl_oob);
    a.cmp_rr(true, Gp::Rax, Gp::R10);
    a.jcc(Cond::A, lbl_oob);
    a.test_rr(true, Gp::Rdx, Gp::Rdx);
    a.jcc(Cond::E, lbl_done);

    // Dst pointer = tbl_desc->ptr + dst * 8
    a.mov_rm(true, Gp::R9, Mem::base_disp(Gp::R9, 0));
    a.lea(true, Gp::Rdi, Mem::indexed(Gp::R9, Gp::Rdi, 8, 0));

    if let Some(ref gc) = layout.gc
        && gc.gc_elem_desc_ro_off != 0
    {
        a.movsxd_rm(Gp::Rax, Mem::indexed(Gp::R11, Gp::R8, 1, 8)); // gc_buf_off
        a.test_rr(true, Gp::Rax, Gp::Rax);
        a.jcc(Cond::Ns, lbl_gc_copy);
        // Static u32 funcref segment in `.rodata`
        a.movsxd_rm(Gp::Rax, Mem::indexed(Gp::R11, Gp::R8, 1, 0));
        a.add_rr(true, Gp::R11, Gp::Rax);
    } else {
        a.movsxd_rm(Gp::Rax, Mem::indexed(Gp::R11, Gp::Rcx, 8, 0));
        a.add_rr(true, Gp::R11, Gp::Rax);
    }

    // Translate `u32` funcref entries: `0 -> 0`, `f+1 -> OFF_STORE_FUNC_REFS[f]`.
    a.lea(true, Gp::Rsi, Mem::indexed(Gp::R11, Gp::Rsi, 4, 0));
    a.mov_rm(true, Gp::R8, Mem::base_disp(Gp::R15, OFF_STORE_FUNC_REFS));
    a.bind(lbl_fn_lp);
    a.mov_rm(false, Gp::Rax, Mem::base(Gp::Rsi));
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, lbl_fn_store);
    a.sub_ri(true, Gp::Rax, 1);
    a.mov_rm(true, Gp::Rax, Mem::indexed(Gp::R8, Gp::Rax, 8, 0));
    a.bind(lbl_fn_store);
    a.mov_mr(true, Mem::base(Gp::Rdi), Gp::Rax);
    a.add_ri(true, Gp::Rsi, 4);
    a.add_ri(true, Gp::Rdi, 8);
    a.sub_ri(true, Gp::Rdx, 1);
    a.jcc(Cond::Ne, lbl_fn_lp);
    a.jmp(lbl_done);

    if let Some(ref gc) = layout.gc
        && gc.gc_elem_desc_ro_off != 0
    {
        a.bind(lbl_gc_copy);
        // Copy `len` qwords from `[elem_buf_ptr + gc_buf_off + src*8]` to `Rdi`.
        a.mov_rm(
            true,
            Gp::R8,
            Mem::base_disp(Gp::R15, OFF_STORE_ELEM_BUF_PTR),
        );
        a.add_rr(true, Gp::R8, Gp::Rax);
        a.lea(true, Gp::Rsi, Mem::indexed(Gp::R8, Gp::Rsi, 8, 0));
        a.bind(lbl_gc_lp);
        a.mov_rm(true, Gp::Rax, Mem::base(Gp::Rsi));
        a.mov_mr(true, Mem::base(Gp::Rdi), Gp::Rax);
        a.add_ri(true, Gp::Rsi, 8);
        a.add_ri(true, Gp::Rdi, 8);
        a.sub_ri(true, Gp::Rdx, 1);
        a.jcc(Cond::Ne, lbl_gc_lp);
    }

    a.bind(lbl_done);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();
    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_TABLE_OOB));
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// `HELPER_GC_ARRAY_INIT_ELEM` for `StoreCdylib`:
/// `Rdi = arr, Rsi = dst (u32), Rdx = src (u32), Rcx = len (u32), R8 = elem_idx (u32)`.
fn emit_store_gc_array_init_elem(_module: &X64Module, layout: &CtxLayout) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(280);
    let Some(ref gc) = layout.gc else {
        a.ud2();
        return Ok(a);
    };
    let Some(drop_off) = layout.elem_dropped_off else {
        a.ud2();
        return Ok(a);
    };
    if gc.gc_elem_desc_ro_off == 0 {
        a.ud2();
        return Ok(a);
    }
    let lbl_null = a.new_label();
    let lbl_oob = a.new_label();
    let lbl_not_dropped = a.new_label();
    let lbl_cmp_src = a.new_label();
    let lbl_gc_copy = a.new_label();
    let lbl_gc_lp = a.new_label();
    let lbl_fn_lp = a.new_label();
    let lbl_fn_store = a.new_label();
    let lbl_done = a.new_label();

    a.test_rr(true, Gp::Rdi, Gp::Rdi);
    a.jcc(Cond::E, lbl_null);
    a.mov_rr(false, Gp::Rsi, Gp::Rsi);
    a.mov_rr(false, Gp::Rdx, Gp::Rdx);
    a.mov_rr(false, Gp::Rcx, Gp::Rcx);
    a.mov_rr(false, Gp::R8, Gp::R8);

    // Check dst + len <= arr.len
    a.mov_rm(false, Gp::R9, Mem::base_disp(Gp::Rdi, 4));
    a.mov_rr(true, Gp::Rax, Gp::Rsi);
    a.add_rr(true, Gp::Rax, Gp::Rcx);
    a.cmp_rr(true, Gp::Rax, Gp::R9);
    a.jcc(Cond::A, lbl_oob);

    // Check dropped flag & src + len <= seg_len
    a.movzx_rm8(false, Gp::R10, Mem::indexed(Gp::R15, Gp::R8, 1, drop_off));
    a.test_rr(false, Gp::R10, Gp::R10);
    a.jcc(Cond::E, lbl_not_dropped);
    a.xor_rr(false, Gp::R10, Gp::R10);
    a.jmp(lbl_cmp_src);
    a.bind(lbl_not_dropped);
    a.lea_rip(Gp::R11, RelocTarget::Rodata(gc.gc_elem_desc_ro_off));
    a.mov_rr(true, Gp::R9, Gp::R8);
    a.shl_imm(true, Gp::R9, 4);
    a.mov_rm(false, Gp::R10, Mem::indexed(Gp::R11, Gp::R9, 1, 4));
    a.bind(lbl_cmp_src);
    a.mov_rr(true, Gp::Rax, Gp::Rdx);
    a.add_rr(true, Gp::Rax, Gp::Rcx);
    a.cmp_rr(true, Gp::Rax, Gp::R10);
    a.jcc(Cond::A, lbl_oob);
    a.test_rr(true, Gp::Rcx, Gp::Rcx);
    a.jcc(Cond::E, lbl_done);

    // Dst ptr = arr + 8 + dst * 8
    a.lea(true, Gp::Rdi, Mem::indexed(Gp::Rdi, Gp::Rsi, 8, 8));
    a.movsxd_rm(Gp::Rax, Mem::indexed(Gp::R11, Gp::R9, 1, 8)); // gc_buf_off
    a.test_rr(true, Gp::Rax, Gp::Rax);
    a.jcc(Cond::Ns, lbl_gc_copy);

    // Static u32 funcref segment -> translate via OFF_STORE_FUNC_REFS
    a.movsxd_rm(Gp::Rax, Mem::indexed(Gp::R11, Gp::R9, 1, 0));
    a.add_rr(true, Gp::R11, Gp::Rax);
    a.lea(true, Gp::Rsi, Mem::indexed(Gp::R11, Gp::Rdx, 4, 0));
    a.mov_rm(true, Gp::R8, Mem::base_disp(Gp::R15, OFF_STORE_FUNC_REFS));
    a.bind(lbl_fn_lp);
    a.mov_rm(false, Gp::Rax, Mem::base(Gp::Rsi));
    a.test_rr(false, Gp::Rax, Gp::Rax);
    a.jcc(Cond::E, lbl_fn_store);
    a.sub_ri(true, Gp::Rax, 1);
    a.mov_rm(true, Gp::Rax, Mem::indexed(Gp::R8, Gp::Rax, 8, 0));
    a.bind(lbl_fn_store);
    a.mov_mr(true, Mem::base(Gp::Rdi), Gp::Rax);
    a.add_ri(true, Gp::Rsi, 4);
    a.add_ri(true, Gp::Rdi, 8);
    a.sub_ri(true, Gp::Rcx, 1);
    a.jcc(Cond::Ne, lbl_fn_lp);
    a.jmp(lbl_done);

    a.bind(lbl_gc_copy);
    a.mov_rm(
        true,
        Gp::R8,
        Mem::base_disp(Gp::R15, OFF_STORE_ELEM_BUF_PTR),
    );
    a.add_rr(true, Gp::R8, Gp::Rax);
    a.lea(true, Gp::Rsi, Mem::indexed(Gp::R8, Gp::Rdx, 8, 0));
    a.bind(lbl_gc_lp);
    a.mov_rm(true, Gp::Rax, Mem::base(Gp::Rsi));
    a.mov_mr(true, Mem::base(Gp::Rdi), Gp::Rax);
    a.add_ri(true, Gp::Rsi, 8);
    a.add_ri(true, Gp::Rdi, 8);
    a.sub_ri(true, Gp::Rcx, 1);
    a.jcc(Cond::Ne, lbl_gc_lp);

    a.bind(lbl_done);
    a.xor_rr(false, Gp::R11, Gp::R11);
    a.ret();
    a.bind(lbl_null);
    a.mov_ri32(Gp::R11, u32::from(super::TRAP_INDIRECT_CALL));
    a.ret();
    a.bind(lbl_oob);
    a.mov_ri32(Gp::R11, u32::from(TRAP_OOB));
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// `HELPER_ATOMIC_WAIT`: `Rdi = host_ptr, Rsi = expected, Rdx = timeout_ns, Rcx = is_64` -> `Eax`.
fn emit_store_atomic_wait() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(192);
    a.sub_ri(true, Gp::Rsp, 264);
    emit_save_xmm0_15(&mut a, 0);
    // C ABI: atomic_wait_fn(store [Rdi], host_ptr [Rsi], expected [Rdx], timeout [Rcx], is_64 [R8])
    a.mov_rr(false, Gp::R8, Gp::Rcx);
    a.mov_rr(true, Gp::Rcx, Gp::Rdx);
    a.mov_rr(true, Gp::Rdx, Gp::Rsi);
    a.mov_rr(true, Gp::Rsi, Gp::Rdi);
    a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::R15, OFF_STORE_PTR));
    a.mov_rm(
        true,
        Gp::Rax,
        Mem::base_disp(Gp::Rdi, STORE_OFF_ATOMIC_WAIT_FN),
    );
    a.call_reg(Gp::Rax);
    emit_restore_xmm0_15(&mut a, 0);
    a.add_ri(true, Gp::Rsp, 264);
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// `HELPER_ATOMIC_NOTIFY`: `Rdi = host_ptr, Rsi = count` -> `Eax`.
fn emit_store_atomic_notify() -> Result<Asm, String> {
    let mut a = Asm::with_capacity(176);
    a.sub_ri(true, Gp::Rsp, 264);
    emit_save_xmm0_15(&mut a, 0);
    // C ABI: atomic_notify_fn(store [Rdi], host_ptr [Rsi], count [Rdx])
    a.mov_rr(false, Gp::Rdx, Gp::Rsi);
    a.mov_rr(true, Gp::Rsi, Gp::Rdi);
    a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::R15, OFF_STORE_PTR));
    a.mov_rm(
        true,
        Gp::Rax,
        Mem::base_disp(Gp::Rdi, STORE_OFF_ATOMIC_NOTIFY_FN),
    );
    a.call_reg(Gp::Rax);
    emit_restore_xmm0_15(&mut a, 0);
    a.add_ri(true, Gp::Rsp, 264);
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// Emits `wt_store_init_instance(ctx: *mut u8 [Rdi]) -> u32 [Eax]` (C ABI, stored at `HELPER_INITIALIZE`).
///
/// Initializes defined globals, defined table defaults, GC const expressions (including passive GC
/// element segments), active element segments, active data segments, and runs `start` (if present).
fn emit_store_init_instance(
    module: &X64Module,
    layout: &CtxLayout,
    ro: &RodataConsts,
    tables: &RuntimeTables,
) -> Result<Asm, String> {
    let mut a = Asm::with_capacity(512);
    let lbl_trap_oob = a.new_label();
    let lbl_trap_table_oob = a.new_label();
    let lbl_trap_alloc = a.new_label();
    let lbl_done = a.new_label();

    // Save SysV callee-saved regs (6 pushes = 48B; entry rsp % 16 == 8 -> after 6 pushes rsp % 16 == 8).
    // Note `gc_rt::emit_gc_const_inits` expects `rsp % 16 == 8` on entry!
    // Reserve 16 bytes for saved MXCSR (at +0) and saved prev_walk_stop_rsp (at +8) -> still `rsp % 16 == 8`!
    a.push(Gp::Rbx);
    a.push(Gp::Rbp);
    a.push(Gp::R12);
    a.push(Gp::R13);
    a.push(Gp::R14);
    a.push(Gp::R15);
    a.sub_ri(true, Gp::Rsp, 16);
    a.stmxcsr(Mem::base_disp(Gp::Rsp, 0));
    a.ldmxcsr_rip(RelocTarget::Rodata(ro.mxcsr_wasm));

    a.mov_rr(true, Gp::R15, Gp::Rdi);
    a.mov_rm(true, Gp::R14, Mem::base_disp(Gp::R15, OFF_MEM_BASE));
    a.mov_rm(
        true,
        Gp::R10,
        Mem::base_disp(Gp::R15, OFF_MEM_RESERVE_BYTES),
    );
    a.mov_rm(true, Gp::R13, Mem::base_disp(Gp::R10, 8));

    // 1. Initialize defined non-folded scalar / FuncRef / V128 globals (`gi >= imported_globals.len()`).
    for (gi, &init) in module
        .global_inits
        .iter()
        .enumerate()
        .skip(module.imported_globals.len())
    {
        if module.global_consts[gi].is_some() {
            continue;
        }
        let off = layout.global_offs[gi];
        match init {
            X64ConstVal::I32(v) => {
                a.mov_mi32(false, Mem::base_disp(Gp::R15, off), v);
            }
            X64ConstVal::I64(v) => {
                a.mov_ri64(Gp::Rax, v as u64);
                a.mov_mr(true, Mem::base_disp(Gp::R15, off), Gp::Rax);
            }
            X64ConstVal::F32(bits) => {
                a.mov_mi32(false, Mem::base_disp(Gp::R15, off), bits as i32);
            }
            X64ConstVal::F64(bits) => {
                a.mov_ri64(Gp::Rax, bits);
                a.mov_mr(true, Mem::base_disp(Gp::R15, off), Gp::Rax);
            }
            X64ConstVal::RefNull => {
                a.xor_rr(false, Gp::Rax, Gp::Rax);
                a.mov_mr(true, Mem::base_disp(Gp::R15, off), Gp::Rax);
            }
            X64ConstVal::FuncRef(f) => {
                a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::R15, OFF_STORE_FUNC_REFS));
                a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rax, (f as i32) * 8));
                a.mov_mr(true, Mem::base_disp(Gp::R15, off), Gp::Rax);
            }
            X64ConstVal::I31(enc) => {
                a.mov_ri64(Gp::Rax, u64::from(enc));
                a.mov_mr(true, Mem::base_disp(Gp::R15, off), Gp::Rax);
            }
            X64ConstVal::ExternI31(enc) => {
                a.mov_ri64(Gp::Rax, super::module::GC_EXT_I31_TAG_HI | u64::from(enc));
                a.mov_mr(true, Mem::base_disp(Gp::R15, off), Gp::Rax);
            }
            X64ConstVal::V128(_) | X64ConstVal::RuntimeGc => {}
        }
    }
    for &(gi, ro_off) in &tables.v128_global_inits {
        if gi >= module.imported_globals.len() && module.global_consts[gi].is_none() {
            let off = layout.global_offs[gi];
            a.movdqu_rip(Xmm::Xmm15, RelocTarget::Rodata(ro_off));
            a.movdqu_mx(Mem::base_disp(Gp::R15, off), Xmm::Xmm15);
        }
    }

    // 2. Initialize defined tables' non-zero scalar / FuncRef defaults (`ti >= imported_tables.len()`).
    for (&tbl, meta) in &module.dyn_tables {
        if (tbl as usize) < module.imported_tables.len() || meta.init_val == 0 || meta.min_len == 0
        {
            continue;
        }
        let lp = a.new_label();
        if meta.is_funcref {
            let f = (meta.init_val - 1) as i32;
            a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::R15, OFF_STORE_FUNC_REFS));
            a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rax, f * 8));
        } else {
            a.mov_ri64(Gp::Rax, meta.init_val);
        }
        a.mov_rm(
            true,
            Gp::Rdi,
            Mem::base_disp(Gp::R15, OFF_STORE_TABLE_DESCS),
        );
        a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::Rdi, (tbl as i32) * 8));
        a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::Rdi, 0));
        a.xor_rr(false, Gp::Rcx, Gp::Rcx);
        a.mov_ri32(Gp::Rdx, meta.min_len);
        a.bind(lp);
        a.mov_mr(true, Mem::indexed(Gp::Rdi, Gp::Rcx, 8, 0), Gp::Rax);
        a.add_ri(true, Gp::Rcx, 1);
        a.cmp_rr(true, Gp::Rcx, Gp::Rdx);
        a.jcc(Cond::B, lp);
    }

    // 3. Evaluate GC const expressions (runtime globals, runtime table inits, active/passive GC elem segs).
    let mut need_table_oob = false;
    let mut need_alloc_trap = false;
    let mut need_oob = false;
    if module.uses_gc {
        gc_rt::emit_gc_const_inits(
            &mut a,
            module,
            layout,
            lbl_trap_table_oob,
            lbl_trap_alloc,
            &mut need_table_oob,
            &mut need_alloc_trap,
        )?;
    }

    // 4. Initialize active non-GC (`funcref`) element segments into `StoreTableDesc`s.
    for &(tbl, dst_off, ro_off, elem_len) in &tables.active_dyn_elem_inits {
        need_table_oob = true;
        a.mov_rm(true, Gp::R9, Mem::base_disp(Gp::R15, OFF_STORE_TABLE_DESCS));
        a.mov_rm(true, Gp::R9, Mem::base_disp(Gp::R9, (tbl as i32) * 8));
        a.mov_rm(false, Gp::R10, Mem::base_disp(Gp::R9, 8)); // cur_len zero-ext u64
        if let Some(end) = dst_off.checked_add(u64::from(elem_len)) {
            a.mov_ri64(Gp::Rax, end);
            a.cmp_rr(true, Gp::Rax, Gp::R10);
            a.jcc(Cond::A, lbl_trap_table_oob);
        } else {
            a.jmp(lbl_trap_table_oob);
            continue;
        }
        if elem_len > 0 {
            let lp = a.new_label();
            let st = a.new_label();
            a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::R9, 0));
            a.mov_ri64(Gp::Rcx, dst_off);
            a.lea(true, Gp::Rdi, Mem::indexed(Gp::Rdi, Gp::Rcx, 8, 0));
            a.lea_rip(Gp::Rsi, RelocTarget::Rodata(ro_off));
            a.mov_rm(true, Gp::R8, Mem::base_disp(Gp::R15, OFF_STORE_FUNC_REFS));
            a.mov_ri32(Gp::Rdx, elem_len);
            a.bind(lp);
            a.mov_rm(false, Gp::Rax, Mem::base(Gp::Rsi));
            a.test_rr(false, Gp::Rax, Gp::Rax);
            a.jcc(Cond::E, st);
            a.sub_ri(true, Gp::Rax, 1);
            a.mov_rm(true, Gp::Rax, Mem::indexed(Gp::R8, Gp::Rax, 8, 0));
            a.bind(st);
            a.mov_mr(true, Mem::base(Gp::Rdi), Gp::Rax);
            a.add_ri(true, Gp::Rsi, 4);
            a.add_ri(true, Gp::Rdi, 8);
            a.sub_ri(true, Gp::Rdx, 1);
            a.jcc(Cond::Ne, lp);
        }
    }

    // Mark active/declarative element segments dropped in `elem_dropped_off`.
    if let Some(drop_off) = layout.elem_dropped_off {
        for (ei, seg) in module.elem_segs.iter().enumerate() {
            if !seg.is_passive {
                a.mov_mi8(Mem::base_disp(Gp::R15, drop_off + (ei as i32)), 1);
            }
        }
    }

    // 5. Initialize active data segments into `StoreMemDesc`s.
    for &(mem_idx, mem_off, ro_off, len) in &tables.data_segs {
        need_oob = true;
        a.mov_rm(true, Gp::R9, Mem::base_disp(Gp::R15, OFF_STORE_MEM_DESCS));
        a.mov_rm(true, Gp::R9, Mem::base_disp(Gp::R9, (mem_idx as i32) * 8));
        if let Some(end) = mem_off.checked_add(u64::from(len)) {
            a.mov_ri64(Gp::Rax, end);
            a.cmp_rm(true, Gp::Rax, Mem::base_disp(Gp::R9, 8));
            a.jcc(Cond::A, lbl_trap_oob);
        } else {
            a.jmp(lbl_trap_oob);
            continue;
        }
        if len > 0 {
            a.mov_rm(true, Gp::Rdi, Mem::base_disp(Gp::R9, 0));
            if mem_off != 0 {
                a.mov_ri64(Gp::Rax, mem_off);
                a.add_rr(true, Gp::Rdi, Gp::Rax);
            }
            a.lea_rip(Gp::Rsi, RelocTarget::Rodata(ro_off));
            a.mov_ri32(Gp::Rcx, len);
            a.rep_movsb();
        }
    }

    // Mark active data segments dropped in `data_dropped_off`.
    if let Some(drop_off) = layout.data_dropped_off {
        for &(_, _, di) in &module.active_data_segs {
            a.mov_mi8(Mem::base_disp(Gp::R15, drop_off + (di as i32)), 1);
        }
    }

    // 6. Execute `start` function `() -> ()` (if any), managing `gc_walk_stop_rsp` cleanly.
    a.xor_rr(false, Gp::R11, Gp::R11);
    if let Some(s) = module.start {
        // Align `rsp % 16 == 0` for `call`: currently `rsp % 16 == 8`, so `sub rsp, 8`!
        a.sub_ri(true, Gp::Rsp, 8);
        a.mov_rm(true, Gp::Rbx, Mem::base_disp(Gp::R15, OFF_STORE_PTR));
        a.mov_rm(true, Gp::Rbp, Mem::base_disp(Gp::Rbx, GC_OFF_WALK_STOP_RSP));
        a.lea(true, Gp::Rax, Mem::base_disp(Gp::Rsp, -8));
        a.mov_mr(true, Mem::base_disp(Gp::Rbx, GC_OFF_WALK_STOP_RSP), Gp::Rax);
        if (s as usize) < module.imports.len() {
            a.mov_rr(true, Gp::R12, Gp::R15);
            a.mov_rm(true, Gp::R11, Mem::base_disp(Gp::R15, OFF_STORE_FUNC_REFS));
            a.mov_rm(true, Gp::R11, Mem::base_disp(Gp::R11, (s as i32) * 8));
            a.mov_rm(true, Gp::R15, Mem::base_disp(Gp::R11, 8));
            a.mov_rm(true, Gp::R14, Mem::base_disp(Gp::R15, OFF_MEM_BASE));
            a.mov_rm(
                true,
                Gp::R10,
                Mem::base_disp(Gp::R15, OFF_MEM_RESERVE_BYTES),
            );
            a.mov_rm(true, Gp::R13, Mem::base_disp(Gp::R10, 8));
            a.mov_rm(true, Gp::R11, Mem::base_disp(Gp::R11, 0));
            a.call_reg(Gp::R11);
            a.mov_rr(true, Gp::R15, Gp::R12);
        } else {
            a.call_reloc(RelocTarget::Func(s));
        }
        a.mov_mr(true, Mem::base_disp(Gp::Rbx, GC_OFF_WALK_STOP_RSP), Gp::Rbp);
        a.add_ri(true, Gp::Rsp, 8);
    }

    a.bind(lbl_done);
    a.mov_rr(false, Gp::Rax, Gp::R11);
    a.ldmxcsr(Mem::base_disp(Gp::Rsp, 0));
    a.add_ri(true, Gp::Rsp, 16);
    a.pop(Gp::R15);
    a.pop(Gp::R14);
    a.pop(Gp::R13);
    a.pop(Gp::R12);
    a.pop(Gp::Rbp);
    a.pop(Gp::Rbx);
    a.ret();

    if need_oob {
        a.bind(lbl_trap_oob);
        a.mov_ri32(Gp::R11, u32::from(TRAP_OOB));
        a.jmp(lbl_done);
    }
    if need_table_oob {
        a.bind(lbl_trap_table_oob);
        a.mov_ri32(Gp::R11, u32::from(TRAP_TABLE_OOB));
        a.jmp(lbl_done);
    }
    if need_alloc_trap {
        a.bind(lbl_trap_alloc);
        a.mov_ri32(Gp::R11, u32::from(TRAP_ALLOC_FAILED));
        a.jmp(lbl_done);
    }

    a.finish_labels()?;
    Ok(a)
}

/// Computes flat `[u64]` byte offset for each value in a host-marshalled args/rets buffer:
/// scalar/ref = 8 bytes (1 `u64`), `V128` = 16 bytes (2 `u64`s). Returns `(offsets, total_bytes)`.
fn flat_buf_layout(tys: &[Ty]) -> (Vec<i32>, i32) {
    let mut offs = Vec::with_capacity(tys.len());
    let mut cur = 0i32;
    for &ty in tys {
        offs.push(cur);
        cur += if ty == Ty::V128 { 16 } else { 8 };
    }
    (offs, cur)
}

/// Emits a `StoreCdylib` host-import trampoline for machine signature `(param_tys, result_tys)`.
///
/// Called by Wasm via `StoreFuncRef`:
/// - `R15` = `HostFuncCtx*` (`+0: 0`, `+8: host_func_id (u64)`, `+16: store_ptr`, `+24: &ZERO_MEM_DESC`)
/// - Entry `RSP` points directly to caller's Wasm safepoint return address (`entry_rsp`).
/// - Packs Wasm ABI params into a flat `[u64]` stack buffer, pushes a `GcStackChainNode` onto
///   `store->gc_stack_chain`, calls `store->host_call_fn(store, host_func_id, args, rets) -> u32`,
///   unlinks the chain node, unpacks results into Wasm ABI return regs / overflow return buffer,
///   and returns trap/exn status in `R11d`.
pub fn emit_store_host_trampoline(param_tys: &[Ty], result_tys: &[Ty]) -> Result<Asm, String> {
    let arg_layout = layout_args(param_tys)?;
    let ret_layout = layout_rets(result_tys)?;
    let (arg_flat_offs, arg_flat_bytes) = flat_buf_layout(param_tys);
    let (ret_flat_offs, ret_flat_bytes) = flat_buf_layout(result_tys);

    // Frame layout below 6 callee-saved pushes (48B, `rsp % 16 == 8` after pushes):
    //   +0..args_bytes       : flat args buffer (16B aligned)
    //   +rets_off..          : flat rets buffer (16B aligned)
    //   +xmm_save_off..+96   : saved Wasm callee-saved `Xmm8..Xmm13` (96B, 16B aligned)
    //   +chain_off..+24      : GcStackChainNode { prev_chain, outer_stop_rsp, outer_paused_ret_rsp }
    //   +ret_spill_save_off  : saved `store->ret_spill_ptr` (8B)
    //   +mxcsr_save_off      : saved Wasm MXCSR (8B)
    let args_bytes_al = (arg_flat_bytes + 15) & !15;
    let rets_off = args_bytes_al;
    let rets_bytes_al = (ret_flat_bytes + 15) & !15;
    let xmm_save_off = rets_off + rets_bytes_al;
    let chain_off = xmm_save_off + 96;
    let ret_spill_save_off = chain_off + 24;
    let mxcsr_save_off = ret_spill_save_off + 8;
    let raw_frame = mxcsr_save_off + 8;
    // After 6 pushes (48B) + 8B ret addr = 56B (`56 % 16 == 8`), we need `(56 + frame_sz) % 16 == 0` -> `frame_sz % 16 == 8`.
    let frame_sz = if (raw_frame & 15) == 0 {
        raw_frame + 8
    } else {
        (raw_frame + 15) & !15
    }
    .max(8);
    let frame_sz = if ((56 + frame_sz) & 15) != 0 {
        frame_sz + 8
    } else {
        frame_sz
    };
    let entry_ret_rsp_off = frame_sz + 48; // points to caller's return address!
    let incoming_stack_base = entry_ret_rsp_off + 8;

    let mut a = Asm::with_capacity(256);
    let lbl_trapped = a.new_label();

    a.push(Gp::Rbx);
    a.push(Gp::Rbp);
    a.push(Gp::R12);
    a.push(Gp::R13);
    a.push(Gp::R14);
    a.push(Gp::R15);
    a.sub_ri(true, Gp::Rsp, frame_sz);

    // Load `store_ptr` into `Rbx` (preserved across C call!).
    a.mov_rm(true, Gp::Rbx, Mem::base_disp(Gp::R15, 16));
    if ret_layout.stack_bytes > 0 {
        a.mov_rm(
            true,
            Gp::R10,
            Mem::base_disp(Gp::Rbx, STORE_OFF_RET_SPILL_PTR),
        );
        a.mov_mr(true, Mem::base_disp(Gp::Rsp, ret_spill_save_off), Gp::R10);
    }

    // 1. Pack Wasm ABI arguments into `[rsp + 0 .. arg_flat_bytes]`.
    //    (Note: `Rdi, Rsi, Rdx, Rcx, R8, R9, Xmm0..7` are all untouched so far!)
    for ((&ty, &slot), &foff) in param_tys.iter().zip(&arg_layout.slots).zip(&arg_flat_offs) {
        let dst = Mem::base_disp(Gp::Rsp, foff);
        match slot {
            AbiSlot::Gp(r) => {
                if ty == Ty::I32 {
                    a.mov_rr(false, Gp::R10, r);
                    a.mov_mr(true, dst, Gp::R10);
                } else {
                    a.mov_mr(true, dst, r);
                }
            }
            AbiSlot::Xmm(x) => match ty {
                Ty::F32 => {
                    a.xor_rr(false, Gp::R10, Gp::R10);
                    a.mov_mr(true, dst, Gp::R10);
                    a.movss_mx(dst, x);
                }
                Ty::F64 => a.movsd_mx(dst, x),
                Ty::V128 => a.movdqu_mx(dst, x),
                _ => unreachable!(),
            },
            AbiSlot::Stack(soff) => {
                let src = Mem::base_disp(Gp::Rsp, incoming_stack_base + soff);
                match ty {
                    Ty::I32 | Ty::F32 => {
                        a.mov_rm(false, Gp::R10, src);
                        a.mov_mr(true, dst, Gp::R10);
                    }
                    Ty::I64 | Ty::F64 => {
                        a.mov_rm(true, Gp::R10, src);
                        a.mov_mr(true, dst, Gp::R10);
                    }
                    Ty::V128 => {
                        a.movdqu_xm(Xmm::Xmm15, src);
                        a.movdqu_mx(dst, Xmm::Xmm15);
                    }
                }
            }
        }
    }

    // 2. Save Wasm callee-saved `Xmm8..Xmm13` + Wasm MXCSR (SysV C ABI treats all XMMs as caller-saved!).
    for (i, xmm) in [
        Xmm::Xmm8,
        Xmm::Xmm9,
        Xmm::Xmm10,
        Xmm::Xmm11,
        Xmm::Xmm12,
        Xmm::Xmm13,
    ]
    .into_iter()
    .enumerate()
    {
        a.movdqu_mx(Mem::base_disp(Gp::Rsp, xmm_save_off + (i as i32) * 16), xmm);
    }
    a.stmxcsr(Mem::base_disp(Gp::Rsp, mxcsr_save_off));

    // 3. Push `GcStackChainNode` onto `store->gc_stack_chain` and pause `gc_walk_stop_rsp = 0`.
    a.mov_rm(true, Gp::R10, Mem::base_disp(Gp::Rbx, GC_OFF_STACK_CHAIN));
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, chain_off), Gp::R10);
    a.mov_rm(true, Gp::R10, Mem::base_disp(Gp::Rbx, GC_OFF_WALK_STOP_RSP));
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, chain_off + 8), Gp::R10);
    a.lea(true, Gp::R10, Mem::base_disp(Gp::Rsp, entry_ret_rsp_off));
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, chain_off + 16), Gp::R10);
    a.lea(true, Gp::R10, Mem::base_disp(Gp::Rsp, chain_off));
    a.mov_mr(true, Mem::base_disp(Gp::Rbx, GC_OFF_STACK_CHAIN), Gp::R10);
    a.xor_rr(false, Gp::R10, Gp::R10);
    a.mov_mr(true, Mem::base_disp(Gp::Rbx, GC_OFF_WALK_STOP_RSP), Gp::R10);

    // 3. Call `store->host_call_fn(store_ptr [Rdi], host_func_id [Rsi], args_ptr [Rdx], rets_ptr [Rcx]) -> u32 [Eax]`.
    a.mov_rr(true, Gp::Rdi, Gp::Rbx);
    a.mov_rm(false, Gp::Rsi, Mem::base_disp(Gp::R15, 8)); // host_func_id
    a.lea(true, Gp::Rdx, Mem::base_disp(Gp::Rsp, 0));
    a.lea(true, Gp::Rcx, Mem::base_disp(Gp::Rsp, rets_off));
    a.mov_rm(
        true,
        Gp::Rax,
        Mem::base_disp(Gp::Rbx, STORE_OFF_HOST_CALL_FN),
    );
    a.call_reg(Gp::Rax);
    a.mov_rr(false, Gp::R11, Gp::Rax); // trap / exn status in R11d!

    // 4. Unlink `GcStackChainNode` (restore `gc_stack_chain` & `gc_walk_stop_rsp`).
    a.mov_rm(true, Gp::R10, Mem::base_disp(Gp::Rsp, chain_off));
    a.mov_mr(true, Mem::base_disp(Gp::Rbx, GC_OFF_STACK_CHAIN), Gp::R10);
    a.mov_rm(true, Gp::R10, Mem::base_disp(Gp::Rsp, chain_off + 8));
    a.mov_mr(true, Mem::base_disp(Gp::Rbx, GC_OFF_WALK_STOP_RSP), Gp::R10);

    a.test_rr(false, Gp::R11, Gp::R11);
    a.jcc(Cond::Ne, lbl_trapped);

    // 5. Unpack flat results from `[rsp + rets_off..]` into Wasm ABI return regs & overflow buffer.
    if ret_layout.stack_bytes > 0 {
        a.mov_rm(true, Gp::Rbp, Mem::base_disp(Gp::Rsp, ret_spill_save_off));
    }
    for ((&ty, &slot), &foff) in result_tys.iter().zip(&ret_layout.slots).zip(&ret_flat_offs) {
        let src = Mem::base_disp(Gp::Rsp, rets_off + foff);
        match slot {
            AbiSlot::Gp(r) => a.mov_rm(ty == Ty::I64, r, src),
            AbiSlot::Xmm(x) => match ty {
                Ty::F32 => a.movss_xm(x, src),
                Ty::F64 => a.movsd_xm(x, src),
                Ty::V128 => a.movdqu_xm(x, src),
                _ => unreachable!(),
            },
            AbiSlot::Stack(boff) => {
                let dst = Mem::base_disp(Gp::Rbp, boff);
                match ty {
                    Ty::I32 | Ty::F32 => {
                        a.mov_rm(false, Gp::R10, src);
                        a.mov_mr(false, dst, Gp::R10);
                    }
                    Ty::I64 | Ty::F64 => {
                        a.mov_rm(true, Gp::R10, src);
                        a.mov_mr(true, dst, Gp::R10);
                    }
                    Ty::V128 => {
                        a.movdqu_xm(Xmm::Xmm15, src);
                        a.movdqu_mx(dst, Xmm::Xmm15);
                    }
                }
            }
        }
    }

    a.bind(lbl_trapped);
    // Restore Wasm MXCSR and callee-saved `Xmm8..Xmm13` on BOTH normal return and trap/exn paths!
    a.ldmxcsr(Mem::base_disp(Gp::Rsp, mxcsr_save_off));
    for (i, xmm) in [
        Xmm::Xmm8,
        Xmm::Xmm9,
        Xmm::Xmm10,
        Xmm::Xmm11,
        Xmm::Xmm12,
        Xmm::Xmm13,
    ]
    .into_iter()
    .enumerate()
    {
        a.movdqu_xm(xmm, Mem::base_disp(Gp::Rsp, xmm_save_off + (i as i32) * 16));
    }
    a.add_ri(true, Gp::Rsp, frame_sz);
    a.pop(Gp::R15);
    a.pop(Gp::R14);
    a.pop(Gp::R13);
    a.pop(Gp::R12);
    a.pop(Gp::Rbp);
    a.pop(Gp::Rbx);
    a.ret();
    a.finish_labels()?;
    Ok(a)
}

/// Emits a `StoreCdylib` export-entry trampoline for machine signature `(param_tys, result_tys)`.
///
/// SysV C ABI:
/// `extern "C" fn(ctx: *mut u8 [Rdi], fn_code: *const u8 [Rsi], args_ptr: *const u64 [Rdx], rets_ptr: *mut u64 [Rcx]) -> u32 [Eax]`
pub fn emit_store_export_entry(
    param_tys: &[Ty],
    result_tys: &[Ty],
    ro: &RodataConsts,
) -> Result<Asm, String> {
    let arg_layout = layout_args(param_tys)?;
    let ret_layout = layout_rets(result_tys)?;
    let (arg_flat_offs, _) = flat_buf_layout(param_tys);
    let (ret_flat_offs, _) = flat_buf_layout(result_tys);

    // Frame layout below 6 callee-saved pushes (48B, `rsp % 16 == 8` after pushes):
    //   +0 .. outgoing_arg_bytes : Wasm outgoing stack arguments
    //   +ret_buf_off ..          : Wasm overflow return buffer (16B aligned)
    //   +mxcsr_off               : saved host MXCSR (8B)
    //   +prev_stop_off           : saved `store->gc_walk_stop_rsp` (8B)
    let ret_buf_off = (arg_layout.stack_bytes + 15) & !15;
    let mxcsr_off = (ret_buf_off + ret_layout.stack_bytes + 15) & !15;
    let prev_stop_off = mxcsr_off + 8;
    let raw_frame = prev_stop_off + 8;
    // Need `(56 + frame_sz) % 16 == 0` so `rsp % 16 == 0` before `call fn_code`!
    let mut frame_sz = (raw_frame + 7) & !7;
    if ((56 + frame_sz) & 15) != 0 {
        frame_sz += 8;
    }

    let mut a = Asm::with_capacity(256);
    let lbl_done = a.new_label();

    a.push(Gp::Rbx);
    a.push(Gp::Rbp);
    a.push(Gp::R12);
    a.push(Gp::R13);
    a.push(Gp::R14);
    a.push(Gp::R15);
    a.sub_ri(true, Gp::Rsp, frame_sz);

    // Save host MXCSR and load Wasm default MXCSR.
    a.stmxcsr(Mem::base_disp(Gp::Rsp, mxcsr_off));
    a.ldmxcsr_rip(RelocTarget::Rodata(ro.mxcsr_wasm));

    // Preserve `rets_ptr` in `Rbx`, `store_ptr` in `Rbp`, `fn_code` in `R12`, `args_ptr` in `R10`.
    a.mov_rr(true, Gp::R15, Gp::Rdi); // ctx
    a.mov_rr(true, Gp::R12, Gp::Rsi); // fn_code
    a.mov_rr(true, Gp::R10, Gp::Rdx); // args_ptr
    a.mov_rr(true, Gp::Rbx, Gp::Rcx); // rets_ptr
    a.mov_rm(true, Gp::Rbp, Mem::base_disp(Gp::R15, OFF_STORE_PTR));

    // Load `R14 = mem0_base`, `R13 = mem0_desc->len`.
    a.mov_rm(true, Gp::R14, Mem::base_disp(Gp::R15, OFF_MEM_BASE));
    a.mov_rm(
        true,
        Gp::R11,
        Mem::base_disp(Gp::R15, OFF_MEM_RESERVE_BYTES),
    );
    a.mov_rm(true, Gp::R13, Mem::base_disp(Gp::R11, 8));

    // Publish overflow return buffer pointer if needed.
    if ret_layout.stack_bytes > 0 {
        a.lea(true, Gp::R11, Mem::base_disp(Gp::Rsp, ret_buf_off));
        a.mov_mr(
            true,
            Mem::base_disp(Gp::Rbp, STORE_OFF_RET_SPILL_PTR),
            Gp::R11,
        );
    }

    // Unpack flat `args_ptr` (`R10`) into Wasm ABI argument regs & outgoing stack area.
    for ((&ty, &slot), &foff) in param_tys.iter().zip(&arg_layout.slots).zip(&arg_flat_offs) {
        let src = Mem::base_disp(Gp::R10, foff);
        match slot {
            AbiSlot::Gp(r) => a.mov_rm(ty == Ty::I64, r, src),
            AbiSlot::Xmm(x) => match ty {
                Ty::F32 => a.movss_xm(x, src),
                Ty::F64 => a.movsd_xm(x, src),
                Ty::V128 => a.movdqu_xm(x, src),
                _ => unreachable!(),
            },
            AbiSlot::Stack(soff) => {
                let dst = Mem::base_disp(Gp::Rsp, soff);
                match ty {
                    Ty::I32 | Ty::F32 => {
                        a.mov_rm(false, Gp::R11, src);
                        a.mov_mr(false, dst, Gp::R11);
                    }
                    Ty::I64 | Ty::F64 => {
                        a.mov_rm(true, Gp::R11, src);
                        a.mov_mr(true, dst, Gp::R11);
                    }
                    Ty::V128 => {
                        a.movdqu_xm(Xmm::Xmm15, src);
                        a.movdqu_mx(dst, Xmm::Xmm15);
                    }
                }
            }
        }
    }

    // Save previous `gc_walk_stop_rsp` and set `gc_walk_stop_rsp = RSP - 8` (just before `call r12`).
    a.mov_rm(true, Gp::Rax, Mem::base_disp(Gp::Rbp, GC_OFF_WALK_STOP_RSP));
    a.mov_mr(true, Mem::base_disp(Gp::Rsp, prev_stop_off), Gp::Rax);
    a.lea(true, Gp::Rax, Mem::base_disp(Gp::Rsp, -8));
    a.mov_mr(true, Mem::base_disp(Gp::Rbp, GC_OFF_WALK_STOP_RSP), Gp::Rax);

    a.call_reg(Gp::R12);

    // Restore `gc_walk_stop_rsp` and host MXCSR (preserving `R11d` and all Wasm return regs!).
    a.mov_rm(true, Gp::R10, Mem::base_disp(Gp::Rsp, prev_stop_off));
    a.mov_mr(true, Mem::base_disp(Gp::Rbp, GC_OFF_WALK_STOP_RSP), Gp::R10);
    a.ldmxcsr(Mem::base_disp(Gp::Rsp, mxcsr_off));

    a.test_rr(false, Gp::R11, Gp::R11);
    a.jcc(Cond::Ne, lbl_done);

    // Pack Wasm return regs & overflow buffer into flat `rets_ptr` (`Rbx`).
    for ((&ty, &slot), &foff) in result_tys.iter().zip(&ret_layout.slots).zip(&ret_flat_offs) {
        let dst = Mem::base_disp(Gp::Rbx, foff);
        match slot {
            AbiSlot::Gp(r) => {
                if ty == Ty::I32 {
                    a.mov_rr(false, Gp::R10, r);
                    a.mov_mr(true, dst, Gp::R10);
                } else {
                    a.mov_mr(true, dst, r);
                }
            }
            AbiSlot::Xmm(x) => match ty {
                Ty::F32 => {
                    a.xor_rr(false, Gp::R10, Gp::R10);
                    a.mov_mr(true, dst, Gp::R10);
                    a.movss_mx(dst, x);
                }
                Ty::F64 => a.movsd_mx(dst, x),
                Ty::V128 => a.movdqu_mx(dst, x),
                _ => unreachable!(),
            },
            AbiSlot::Stack(boff) => {
                let src = Mem::base_disp(Gp::Rsp, ret_buf_off + boff);
                match ty {
                    Ty::I32 | Ty::F32 => {
                        a.mov_rm(false, Gp::R10, src);
                        a.mov_mr(true, dst, Gp::R10);
                    }
                    Ty::I64 | Ty::F64 => {
                        a.mov_rm(true, Gp::R10, src);
                        a.mov_mr(true, dst, Gp::R10);
                    }
                    Ty::V128 => {
                        a.movdqu_xm(Xmm::Xmm15, src);
                        a.movdqu_mx(dst, Xmm::Xmm15);
                    }
                }
            }
        }
    }

    a.bind(lbl_done);
    a.mov_rr(false, Gp::Rax, Gp::R11);
    a.add_ri(true, Gp::Rsp, frame_sz);
    a.pop(Gp::R15);
    a.pop(Gp::R14);
    a.pop(Gp::R13);
    a.pop(Gp::R12);
    a.pop(Gp::Rbp);
    a.pop(Gp::Rbx);
    a.ret();
    a.finish_labels()?;
    Ok(a)
}
