//! `Module`: compiles Wasm to `StoreCdylib` ELF64 `.so` (or deserializes precompiled `.so` bytes),
//! loads it via `memfd_create` + `dlopen`, decodes `WTST0001` metadata, and registers canonical types.

use std::collections::HashMap;
use std::ffi::CString;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_arch = "aarch64")]
use wtime_aot::aarch64::{A64CompileOptions, compile_wasm_to_aarch64_so};
#[cfg(target_arch = "x86_64")]
use wtime_aot::x64::lower::TargetIsa;
use wtime_aot::x64::module::X64Target;
use wtime_aot::x64::store_rt::{
    STORE_ABI_VERSION, StoreModuleMeta, decode_store_metadata, ty_to_code, val_type_to_machine_ty,
};
#[cfg(target_arch = "x86_64")]
use wtime_aot::x64::{X64CompileOptions, compile_wasm_to_x64_so};

use crate::abi::{StoreModuleDesc, WtStoreExportTrampFn, WtStoreGetDescFn, WtStoreInitInstanceFn};
use crate::config::{Engine, OptLevel, RegisteredCodeRange};
use crate::error::{Error, Result};
use crate::types::{
    ExportKind, ExportType, ExternType, FuncType, GlobalType, ImportType, MemoryType, ModuleExport,
    Mutability, TableType, TagType, ValType,
};

static NEXT_MODULE_ID: AtomicU64 = AtomicU64::new(1);

/// Precompiled artifact envelope magic (`"WTMS0001"`) wrapping a `StoreCdylib` ELF64 `.so` + version tag.
const SERIALIZED_MAGIC: [u8; 8] = *b"WTMS0001";

pub(crate) struct LoadedSo {
    dl_handle: *mut libc::c_void,
    /// Optional temp file path if `memfd_create` was unavailable.
    temp_path: Option<std::path::PathBuf>,
    pub(crate) desc: StoreModuleDesc,
    pub(crate) engine: Engine,
}

// SAFETY: Loaded ELF `.so` code and `.rodata` are immutable once mapped; `dlclose` runs on final drop.
unsafe impl Send for LoadedSo {}
unsafe impl Sync for LoadedSo {}

impl Drop for LoadedSo {
    fn drop(&mut self) {
        self.engine
            .unregister_code_range(self.desc.code_start, self.desc.code_end);
        if !self.dl_handle.is_null() {
            // SAFETY: `dl_handle` was opened by `libc::dlopen` and no live `Store` retains a reference
            // because every `Instance` holds an `Arc<ModuleInner>`.
            unsafe {
                let _ = libc::dlclose(self.dl_handle);
            }
        }
        if let Some(ref path) = self.temp_path {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub(crate) struct ModuleInner {
    pub(crate) id: u64,
    pub(crate) engine: Engine,
    pub(crate) so: LoadedSo,
    /// Retained raw ELF64 `.so` image so `Module::serialize()` works in O(1) without recompiling.
    pub(crate) so_bytes: Vec<u8>,
    pub(crate) meta: StoreModuleMeta,
    /// Maps module-local `type_idx` -> global `Engine` canonical type ID.
    pub(crate) local_type_to_global_canon: Box<[u32]>,
    /// Maps module-local `canon_id` (`0..meta.n_local_canon`) -> global `Engine` canonical type ID.
    /// Pointed to directly by `Ctx + OFF_STORE_CANON_MAP`.
    pub(crate) local_canon_to_global: Box<[u32]>,
    /// Maps `(param_ty_codes, result_ty_codes)` -> index into `desc.host_tramp_table`.
    pub(crate) host_tramp_by_sig: HashMap<(Vec<u8>, Vec<u8>), usize>,
    /// Maps `(param_ty_codes, result_ty_codes)` -> index into `desc.export_tramp_table`.
    pub(crate) export_tramp_by_sig: HashMap<(Vec<u8>, Vec<u8>), usize>,
    /// Export name -> `(ExportKind, index within exported_*)` for fast O(1) lookup.
    pub(crate) export_index: HashMap<String, (ExportKind, usize)>,
    /// Pre-materialized imports in exact Wasm declaration order.
    pub(crate) imports_cache: Vec<(String, String, ExternType)>,
    /// Pre-materialized exports.
    pub(crate) exports_cache: Vec<(String, ExternType)>,
}

/// Function index descriptor yielded by [`Module::functions`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModuleFuncIndex(pub u32);

impl ModuleFuncIndex {
    pub fn as_u32(self) -> u32 {
        self.0
    }
}

/// Native function entry yielded by [`Module::functions`].
#[derive(Clone, Debug)]
pub struct ModuleFunction {
    pub index: ModuleFuncIndex,
    pub name: Option<String>,
    pub offset: usize,
    pub len: usize,
}

/// Compiled WebAssembly module backed by a native x86-64 `StoreCdylib` ELF64 `.so`.
#[derive(Clone)]
pub struct Module {
    pub(crate) inner: Arc<ModuleInner>,
}

impl std::fmt::Debug for Module {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Module")
            .field("id", &self.inner.id)
            .finish()
    }
}

impl Module {
    pub fn same(a: &Self, b: &Self) -> bool {
        Arc::ptr_eq(&a.inner, &b.inner)
    }

    /// Validates Wasm binary bytes against the Engine's feature set without compiling.
    pub fn validate(_engine: &Engine, wasm_bytes: impl AsRef<[u8]>) -> Result<()> {
        wtime_validate::validate(
            wasm_bytes.as_ref(),
            &wtime_aot::x64::module::x64_validate_features(),
            &wtime_aot::x64::module::x64_validate_limits(),
        )
        .map(|_| ())
        .map_err(|e| Error::msg(format!("wasm validation failed: {e}")))
    }

    /// Compiles Wasm binary bytes into a native x86-64 `StoreCdylib` module and loads it.
    pub fn new(engine: &Engine, wasm_bytes: impl AsRef<[u8]>) -> Result<Self> {
        if !engine.config().compiler_enabled {
            return Err(Error::msg(
                "cannot compile Wasm module: compiler is disabled on this Engine",
            ));
        }
        let so_bytes = compile_wasm_so(engine, wasm_bytes.as_ref())?;
        Self::from_so_bytes(engine, so_bytes)
    }

    /// Loads a raw `StoreCdylib` ELF64 `.so` image into `engine`.
    pub fn from_so_bytes(engine: &Engine, so_bytes: Vec<u8>) -> Result<Self> {
        let so = load_so_image(engine, &so_bytes)?;
        // SAFETY: `metadata_ptr..metadata_ptr + metadata_len` lives inside the loaded ELF `.rodata`.
        let meta_slice =
            unsafe { std::slice::from_raw_parts(so.desc.metadata_ptr, so.desc.metadata_len) };
        let meta = decode_store_metadata(meta_slice)
            .map_err(|e| Error::msg(format!("StoreCdylib metadata decode failed: {e}")))?;
        if meta.abi_version != STORE_ABI_VERSION {
            return Err(Error::msg(format!(
                "StoreCdylib ABI version mismatch: got {}, expected {STORE_ABI_VERSION}",
                meta.abi_version
            )));
        }

        // Register module's iso-recursive type groups into the Engine's global `CanonicalTypeRegistry`.
        let local_type_to_global_canon: Box<[u32]> = {
            let mut reg = engine.inner.types.lock().unwrap_or_else(|e| e.into_inner());
            reg.register_groups(&meta.raw_type_groups)?
                .into_boxed_slice()
        };

        let mut local_canon_to_global = vec![0u32; meta.n_local_canon as usize];
        for (ti, &local_cid) in meta.type_canon.iter().enumerate() {
            if (local_cid as usize) < local_canon_to_global.len() {
                local_canon_to_global[local_cid as usize] = local_type_to_global_canon[ti];
            }
        }
        let local_canon_to_global = local_canon_to_global.into_boxed_slice();

        let mut host_tramp_by_sig = HashMap::with_capacity(meta.host_tramp_sigs.len());
        for (i, sig) in meta.host_tramp_sigs.iter().enumerate() {
            host_tramp_by_sig.insert(sig.clone(), i);
        }
        let mut export_tramp_by_sig = HashMap::with_capacity(meta.export_tramp_sigs.len());
        for (i, sig) in meta.export_tramp_sigs.iter().enumerate() {
            export_tramp_by_sig.insert(sig.clone(), i);
        }

        // Pre-materialize imports in exact Wasm declaration order.
        let mut imports_cache = Vec::with_capacity(meta.import_order.len());
        for &(kind, idx) in &meta.import_order {
            let idx = idx as usize;
            match kind {
                0 => {
                    let (ref m, ref n, ti) = meta.imported_funcs[idx];
                    let gcid = local_type_to_global_canon[ti as usize];
                    let ft = FuncType::from_canon(Arc::clone(&engine.inner), gcid);
                    imports_cache.push((m.clone(), n.clone(), ExternType::Func(ft)));
                }
                1 => {
                    let (ref m, ref n, tt) = meta.imported_tables[idx];
                    let table_ty =
                        TableType::from_module_wasm(&engine.inner, tt, &local_type_to_global_canon);
                    imports_cache.push((m.clone(), n.clone(), ExternType::Table(table_ty)));
                }
                2 => {
                    let (ref m, ref n, min_p, _eff_max, declared_max, is64, shared) =
                        meta.imported_mems[idx];
                    let mem_ty = MemoryType::from_raw(min_p, declared_max, is64, shared);
                    imports_cache.push((m.clone(), n.clone(), ExternType::Memory(mem_ty)));
                }
                3 => {
                    let (ref m, ref n, vt, mutable, _off) = meta.imported_globals[idx];
                    let content =
                        ValType::from_module_wasm(&engine.inner, vt, &local_type_to_global_canon);
                    let gt = GlobalType::new(
                        content,
                        if mutable {
                            Mutability::Var
                        } else {
                            Mutability::Const
                        },
                    );
                    imports_cache.push((m.clone(), n.clone(), ExternType::Global(gt)));
                }
                4 => {
                    let (ref m, ref n, ti) = meta.imported_tags[idx];
                    let gcid = local_type_to_global_canon[ti as usize];
                    let ft = FuncType::from_canon(Arc::clone(&engine.inner), gcid);
                    imports_cache.push((m.clone(), n.clone(), ExternType::Tag(TagType::new(ft))));
                }
                other => {
                    return Err(Error::msg(format!(
                        "StoreCdylib metadata: invalid import_order kind {other}"
                    )));
                }
            }
        }

        // Pre-materialize exports + lookup map.
        let mut export_index = HashMap::new();
        let mut exports_cache = Vec::new();
        for (i, (name, fi, _tramp)) in meta.exported_funcs.iter().enumerate() {
            let ti = meta.func_type_idx[*fi as usize];
            let gcid = local_type_to_global_canon[ti as usize];
            let ft = FuncType::from_canon(Arc::clone(&engine.inner), gcid);
            export_index.insert(name.clone(), (ExportKind::Func, i));
            exports_cache.push((name.clone(), ExternType::Func(ft)));
        }
        for (i, (name, _gi, vt, mutable, _off)) in meta.exported_globals.iter().enumerate() {
            let content =
                ValType::from_module_wasm(&engine.inner, *vt, &local_type_to_global_canon);
            let gt = GlobalType::new(
                content,
                if *mutable {
                    Mutability::Var
                } else {
                    Mutability::Const
                },
            );
            export_index.insert(name.clone(), (ExportKind::Global, i));
            exports_cache.push((name.clone(), ExternType::Global(gt)));
        }
        for (i, (name, mi)) in meta.exported_mems.iter().enumerate() {
            let mi = *mi as usize;
            let mem_ty = if mi < meta.imported_mems.len() {
                let (_, _, min_p, _, dmax, is64, shared) = &meta.imported_mems[mi];
                MemoryType::from_raw(*min_p, *dmax, *is64, *shared)
            } else {
                let (_, min_p, _, dmax, is64, shared) =
                    &meta.defined_mems[mi - meta.imported_mems.len()];
                MemoryType::from_raw(*min_p, *dmax, *is64, *shared)
            };
            export_index.insert(name.clone(), (ExportKind::Memory, i));
            exports_cache.push((name.clone(), ExternType::Memory(mem_ty)));
        }
        for (i, (name, ti)) in meta.exported_tables.iter().enumerate() {
            let ti = *ti as usize;
            let raw_tt = if ti < meta.imported_tables.len() {
                meta.imported_tables[ti].2
            } else {
                meta.defined_tables[ti - meta.imported_tables.len()].6
            };
            let table_ty =
                TableType::from_module_wasm(&engine.inner, raw_tt, &local_type_to_global_canon);
            export_index.insert(name.clone(), (ExportKind::Table, i));
            exports_cache.push((name.clone(), ExternType::Table(table_ty)));
        }
        for (i, (name, tag_i)) in meta.exported_tags.iter().enumerate() {
            let tag_i = *tag_i as usize;
            let type_idx = if tag_i < meta.imported_tags.len() {
                meta.imported_tags[tag_i].2
            } else {
                meta.defined_tags[tag_i - meta.imported_tags.len()].1
            };
            let gcid = local_type_to_global_canon[type_idx as usize];
            let ft = FuncType::from_canon(Arc::clone(&engine.inner), gcid);
            export_index.insert(name.clone(), (ExportKind::Tag, i));
            exports_cache.push((name.clone(), ExternType::Tag(TagType::new(ft))));
        }

        let id = NEXT_MODULE_ID.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            inner: Arc::new(ModuleInner {
                id,
                engine: engine.clone(),
                so,
                so_bytes,
                meta,
                local_type_to_global_canon,
                local_canon_to_global,
                host_tramp_by_sig,
                export_tramp_by_sig,
                export_index,
                imports_cache,
                exports_cache,
            }),
        })
    }

    /// Serializes this compiled module into a self-contained precompiled binary artifact.
    pub fn serialize(&self) -> Result<Vec<u8>> {
        Ok(serialize_so_bytes(&self.inner.engine, &self.inner.so_bytes))
    }

    /// Deserializes a precompiled artifact produced by `Module::serialize`.
    ///
    /// # Safety
    ///
    /// The caller guarantees `bytes` contains a trusted native `StoreCdylib` image produced by
    /// a compatible version of `wtime-aot-host`.
    pub unsafe fn deserialize(engine: &Engine, bytes: impl AsRef<[u8]>) -> Result<Self> {
        let bytes = bytes.as_ref();
        if bytes.len() < 20 || bytes[..8] != SERIALIZED_MAGIC {
            return Err(Error::msg("invalid serialized wtime-aot-host module magic"));
        }
        let ver_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let hdr_end = 12usize
            .checked_add(ver_len)
            .and_then(|v| v.checked_add(8))
            .ok_or_else(|| Error::msg("corrupt serialized module header"))?;
        if bytes.len() < hdr_end {
            return Err(Error::msg("truncated serialized module header"));
        }
        let expected_ver = engine_version_bytes(engine);
        if &bytes[12..12 + ver_len] != expected_ver.as_slice() {
            return Err(Error::msg("serialized module version mismatch with Engine"));
        }
        let so_len =
            u64::from_le_bytes(bytes[12 + ver_len..12 + ver_len + 8].try_into().unwrap()) as usize;
        if bytes.len() != hdr_end + so_len {
            return Err(Error::msg("serialized module payload length mismatch"));
        }
        Self::from_so_bytes(engine, bytes[hdr_end..].to_vec())
    }

    /// Deserializes a precompiled artifact from an open file.
    ///
    /// # Safety
    ///
    /// Same trust contract as `Module::deserialize`.
    pub unsafe fn deserialize_open_file(engine: &Engine, mut file: File) -> Result<Self> {
        file.seek(SeekFrom::Start(0))?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        unsafe { Self::deserialize(engine, buf) }
    }

    /// Deserializes a precompiled artifact from `[offset..offset+len]` of an open file.
    ///
    /// # Safety
    ///
    /// Same trust contract as `Module::deserialize`.
    pub unsafe fn deserialize_open_file_range(
        engine: &Engine,
        file: impl Into<Arc<File>>,
        offset: u64,
        len: usize,
    ) -> Result<Self> {
        use std::os::unix::fs::FileExt;
        let file: Arc<File> = file.into();
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(4096) as u64;
        if !offset.is_multiple_of(page_size) {
            return Err(Error::msg(
                "deserialize_open_file_range offset is not page-aligned",
            ));
        }
        if len == 0 {
            return Err(Error::msg("deserialize_open_file_range length is zero"));
        }
        let file_len = file.metadata()?.len();
        let end = offset
            .checked_add(len as u64)
            .ok_or_else(|| Error::msg("deserialize_open_file_range offset overflow"))?;
        if end > file_len {
            return Err(Error::msg(
                "deserialize_open_file_range range exceeds file size",
            ));
        }
        let mut buf = vec![0u8; len];
        file.read_exact_at(&mut buf, offset)?;
        unsafe { Self::deserialize(engine, buf) }
    }

    pub fn engine(&self) -> &Engine {
        &self.inner.engine
    }

    pub fn imports(&self) -> impl ExactSizeIterator<Item = ImportType<'_>> + '_ {
        self.inner
            .imports_cache
            .iter()
            .map(|(m, n, ty)| ImportType {
                module: m.as_str(),
                name: n.as_str(),
                ty: ty.clone(),
            })
    }

    pub fn exports(&self) -> impl ExactSizeIterator<Item = ExportType<'_>> + '_ {
        self.inner.exports_cache.iter().map(|(n, ty)| ExportType {
            name: n.as_str(),
            ty: ty.clone(),
        })
    }

    pub fn get_export(&self, name: &str) -> Option<ExternType> {
        self.inner
            .exports_cache
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, ty)| ty.clone())
    }

    pub fn get_export_index(&self, name: &str) -> Option<ModuleExport> {
        let &(kind, index) = self.inner.export_index.get(name)?;
        Some(ModuleExport {
            module_id: self.inner.id,
            kind,
            index,
        })
    }

    pub fn image_range(&self) -> std::ops::Range<*const u8> {
        (self.inner.so.desc.code_start as *const u8)..(self.inner.so.desc.code_end as *const u8)
    }

    pub fn text(&self) -> &[u8] {
        let start = self.inner.so.desc.code_start;
        let end = self.inner.so.desc.code_end;
        if start == 0 || end <= start {
            return &[];
        }
        // SAFETY: `code_start..code_end` is the immutable RX `.text` segment of the loaded `.so`.
        unsafe { std::slice::from_raw_parts(start as *const u8, end - start) }
    }

    pub fn functions(&self) -> impl ExactSizeIterator<Item = ModuleFunction> + '_ {
        let n_imp = self.inner.meta.n_imports;
        let n_defs = self.inner.meta.n_defs as usize;
        let base = self.inner.so.desc.code_start;
        let end = self.inner.so.desc.code_end;
        let mut addrs: Vec<usize> = (0..n_defs).map(|i| self.def_fn_ptr(i) as usize).collect();
        addrs.push(end);
        let mut out = Vec::with_capacity(n_defs);
        for i in 0..n_defs {
            let start = addrs[i];
            let next = addrs[i + 1].max(start);
            out.push(ModuleFunction {
                index: ModuleFuncIndex(n_imp + i as u32),
                name: None,
                offset: start.saturating_sub(base),
                len: next.saturating_sub(start),
            });
        }
        out.into_iter()
    }

    // ---- Internal low-level accessors into the loaded ELF `.so` ----

    #[inline]
    pub(crate) fn def_fn_ptr(&self, def_idx: usize) -> *const u8 {
        debug_assert!(def_idx < self.inner.meta.n_defs as usize);
        let tbl = self.inner.so.desc.def_fn_table;
        // SAFETY: `def_idx < n_defs`; each table slot holds `rel32` from `tbl` base.
        let rel = unsafe { *tbl.add(def_idx) };
        unsafe { tbl.cast::<u8>().offset(rel as isize) }
    }

    #[inline]
    pub(crate) fn host_tramp_ptr(&self, tramp_idx: usize) -> *const u8 {
        debug_assert!(tramp_idx < self.inner.meta.host_tramp_sigs.len());
        let tbl = self.inner.so.desc.host_tramp_table;
        let rel = unsafe { *tbl.add(tramp_idx) };
        unsafe { tbl.cast::<u8>().offset(rel as isize) }
    }

    #[inline]
    pub(crate) fn export_tramp_fn(&self, tramp_idx: usize) -> WtStoreExportTrampFn {
        debug_assert!(tramp_idx < self.inner.meta.export_tramp_sigs.len());
        let tbl = self.inner.so.desc.export_tramp_table;
        let rel = unsafe { *tbl.add(tramp_idx) };
        let ptr = unsafe { tbl.cast::<u8>().offset(rel as isize) };
        // SAFETY: Compiled by `emit_store_export_entry` with SysV C ABI `WtStoreExportTrampFn`.
        unsafe { std::mem::transmute::<*const u8, WtStoreExportTrampFn>(ptr) }
    }

    #[inline]
    pub(crate) fn init_instance_fn(&self) -> WtStoreInitInstanceFn {
        // SAFETY: Compiled by `emit_store_init_instance` with SysV C ABI `WtStoreInitInstanceFn`.
        unsafe {
            std::mem::transmute::<*const u8, WtStoreInitInstanceFn>(
                self.inner.so.desc.init_instance_fn,
            )
        }
    }

    /// Finds the host trampoline code pointer matching a canonical `FuncType` in this module.
    pub(crate) fn host_tramp_for_func_type(&self, ft: &FuncType) -> Option<*const u8> {
        let (ps, rs) = ft.wasm_params_results();
        let ps_c: Vec<u8> = ps
            .iter()
            .map(|&v| ty_to_code(val_type_to_machine_ty(v)))
            .collect();
        let rs_c: Vec<u8> = rs
            .iter()
            .map(|&v| ty_to_code(val_type_to_machine_ty(v)))
            .collect();
        let &idx = self.inner.host_tramp_by_sig.get(&(ps_c, rs_c))?;
        Some(self.host_tramp_ptr(idx))
    }

    /// Finds the export-entry trampoline matching a canonical `FuncType` in this module.
    pub(crate) fn export_tramp_for_func_type(&self, ft: &FuncType) -> Option<WtStoreExportTrampFn> {
        let (ps, rs) = ft.wasm_params_results();
        let ps_c: Vec<u8> = ps
            .iter()
            .map(|&v| ty_to_code(val_type_to_machine_ty(v)))
            .collect();
        let rs_c: Vec<u8> = rs
            .iter()
            .map(|&v| ty_to_code(val_type_to_machine_ty(v)))
            .collect();
        let &idx = self.inner.export_tramp_by_sig.get(&(ps_c, rs_c))?;
        Some(self.export_tramp_fn(idx))
    }
}

pub(crate) fn compile_wasm_so(engine: &Engine, wasm_bytes: &[u8]) -> Result<Vec<u8>> {
    let cfg = engine.config();
    let max_gc_bytes = cfg
        .gc_heap_reservation
        .min(crate::gc::SEMI_SPACE_RESERVE_BYTES as u64);
    #[cfg(target_arch = "x86_64")]
    {
        let isa = TargetIsa {
            has_abm_bmi1: std::is_x86_feature_detected!("lzcnt")
                && std::is_x86_feature_detected!("bmi1"),
            has_bmi2: std::is_x86_feature_detected!("bmi2"),
            has_avx2: std::is_x86_feature_detected!("avx2"),
            has_avx512: false,
        };
        let opts = X64CompileOptions {
            target: X64Target::StoreCdylib,
            isa,
            opt: cfg.opt_level != OptLevel::None,
            verify: false,
            jobs: if cfg.parallel_compilation { 0 } else { 1 },
            gc_initial_heap_bytes: cfg.gc_heap_initial_size,
            gc_max_heap_bytes: max_gc_bytes,
            ..X64CompileOptions::default()
        };
        compile_wasm_to_x64_so(wasm_bytes, &opts)
            .map_err(|e| Error::msg(format!("wtime-aot x64 compile failed: {e}")))
    }
    #[cfg(target_arch = "aarch64")]
    {
        let opts = A64CompileOptions {
            target: X64Target::StoreCdylib,
            optimize: cfg.opt_level != OptLevel::None,
            jobs: if cfg.parallel_compilation { 0 } else { 1 },
            gc_initial_heap_bytes: cfg.gc_heap_initial_size,
            gc_max_heap_bytes: max_gc_bytes,
            ..A64CompileOptions::default()
        };
        compile_wasm_to_aarch64_so(wasm_bytes, &opts)
            .map_err(|e| Error::msg(format!("wtime-aot aarch64 compile failed: {e}")))
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let _ = (wasm_bytes, max_gc_bytes);
        Err(Error::msg(format!(
            "wtime-aot-host: unsupported target architecture `{}`",
            std::env::consts::ARCH
        )))
    }
}

pub(crate) fn serialize_so_bytes(engine: &Engine, so_bytes: &[u8]) -> Vec<u8> {
    let ver = engine_version_bytes(engine);
    let mut out = Vec::with_capacity(8 + 4 + ver.len() + 8 + so_bytes.len());
    out.extend_from_slice(&SERIALIZED_MAGIC);
    out.extend_from_slice(&(ver.len() as u32).to_le_bytes());
    out.extend_from_slice(&ver);
    out.extend_from_slice(&(so_bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(so_bytes);
    out
}

fn engine_version_bytes(engine: &Engine) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&engine.precompile_compatibility_hash().to_le_bytes());
    match &engine.config().module_version {
        crate::config::ModuleVersionStrategy::WasmtimeVersion => {
            out.extend_from_slice(
                format!(
                    "wtime-store-{}-v{STORE_ABI_VERSION}",
                    std::env::consts::ARCH
                )
                .as_bytes(),
            );
        }
        crate::config::ModuleVersionStrategy::Custom(s) => {
            out.extend_from_slice(s.as_bytes());
        }
        crate::config::ModuleVersionStrategy::None => {}
    }
    out
}

/// Loads an ELF64 `.so` image using a globally unique tmpfs (`/dev/shm`, fallback `temp_dir`)
/// path so glibc's `dlopen` filename cache NEVER aliases two distinct modules, then immediately
/// unlinks the path once mapped.
fn load_so_image(engine: &Engine, so_bytes: &[u8]) -> Result<LoadedSo> {
    static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let fname = format!("wtime_store_{}_{seq}.so", std::process::id());
    let shm_dir = std::path::Path::new("/dev/shm");
    let path = if shm_dir.is_dir() {
        let p = shm_dir.join(&fname);
        if std::fs::write(&p, so_bytes).is_ok() {
            p
        } else {
            let p2 = std::env::temp_dir().join(&fname);
            std::fs::write(&p2, so_bytes)?;
            p2
        }
    } else {
        let p = std::env::temp_dir().join(&fname);
        std::fs::write(&p, so_bytes)?;
        p
    };
    let cpath = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| Error::msg("invalid so path"))?;
    let dl_handle = unsafe { libc::dlopen(cpath.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    // Unlink immediately after dlopen maps the segments so no temp files ever remain on disk.
    let _ = std::fs::remove_file(&path);
    let temp_path: Option<std::path::PathBuf> = None;

    if dl_handle.is_null() {
        let err = unsafe {
            let p = libc::dlerror();
            if p.is_null() {
                "unknown dlopen error".to_string()
            } else {
                std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        return Err(Error::msg(format!("dlopen StoreCdylib failed: {err}")));
    }

    let sym_name = CString::new("wt_store_get_desc").unwrap();
    let sym = unsafe { libc::dlsym(dl_handle, sym_name.as_ptr()) };
    if sym.is_null() {
        unsafe {
            let _ = libc::dlclose(dl_handle);
        }
        return Err(Error::msg("StoreCdylib missing `wt_store_get_desc` export"));
    }
    let get_desc: WtStoreGetDescFn = unsafe { std::mem::transmute(sym) };
    let mut desc = StoreModuleDesc::default();
    unsafe {
        get_desc(&mut desc);
    }
    if desc.abi_version != STORE_ABI_VERSION {
        unsafe {
            let _ = libc::dlclose(dl_handle);
        }
        return Err(Error::msg(format!(
            "StoreCdylib descriptor ABI mismatch: got {}, expected {STORE_ABI_VERSION}",
            desc.abi_version
        )));
    }

    engine.register_code_range(RegisteredCodeRange {
        code_start: desc.code_start,
        code_end: desc.code_end,
        safepoint_header: desc.safepoint_header,
    });

    Ok(LoadedSo {
        dl_handle,
        temp_path,
        desc,
        engine: engine.clone(),
    })
}
