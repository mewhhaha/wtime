//! `Module`: compiles Wasm to `StoreCdylib` ELF64 `.so` (or deserializes precompiled `.so` bytes),
//! maps it zero-copy via `mmap` (or `dlopen` fallback), decodes `WTST0001` metadata, and registers
//! canonical types.

use std::collections::HashMap;
use std::ffi::CString;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
#[cfg(unix)]
use std::os::unix::fs::FileExt;
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
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

/// Legacy precompiled artifact envelope magic (`"WTMS0001"`, leading header before ELF `.so`).
const SERIALIZED_MAGIC_V1: [u8; 8] = *b"WTMS0001";
/// Page-aligned precompiled artifact envelope magic (`"WTMS0002"`, raw ELF64 `.so` at offset `0`
/// followed by a compact trailer so embedded/file-backed images map zero-copy via `mmap`).
const SERIALIZED_MAGIC_V2: [u8; 8] = *b"WTMS0002";

pub(crate) enum SoMapping {
    Mmap {
        base: *mut u8,
        map_len: usize,
        /// Reported byte length for `Module::image_range()` (matches serialized artifact length
        /// when loaded via `deserialize*`, or `map_len` for in-memory compiled modules).
        image_len: usize,
        /// Optional file range holding the raw ELF64 `.so` bytes (`(file, so_file_offset, so_len)`)
        /// so `Module::serialize()` works without retaining a duplicate heap `Vec<u8>`.
        backing: Option<(Arc<File>, u64, usize)>,
    },
    Dlopen {
        dl_handle: *mut libc::c_void,
        /// Optional temp file path if `memfd_create` was unavailable.
        temp_path: Option<std::path::PathBuf>,
        backing: Option<(Arc<File>, u64, usize)>,
    },
}

pub(crate) struct LoadedSo {
    mapping: SoMapping,
    pub(crate) desc: StoreModuleDesc,
    pub(crate) engine: Engine,
}

impl LoadedSo {
    fn backing(&self) -> Option<&(Arc<File>, u64, usize)> {
        match &self.mapping {
            SoMapping::Mmap { backing, .. } | SoMapping::Dlopen { backing, .. } => backing.as_ref(),
        }
    }
}

// SAFETY: Loaded ELF `.so` code and `.rodata` are immutable once mapped; unmap/`dlclose` runs on final drop.
unsafe impl Send for LoadedSo {}
unsafe impl Sync for LoadedSo {}

impl Drop for LoadedSo {
    fn drop(&mut self) {
        self.engine
            .unregister_code_range(self.desc.code_start, self.desc.code_end);
        match &mut self.mapping {
            SoMapping::Mmap { base, map_len, .. } => {
                if !base.is_null() && *map_len > 0 {
                    // SAFETY: `[base, base + map_len)` was reserved by `libc::mmap` and no live
                    // `Store` retains a reference because every `Instance` holds an `Arc<ModuleInner>`.
                    unsafe {
                        let _ = libc::munmap((*base).cast(), *map_len);
                    }
                }
            }
            SoMapping::Dlopen {
                dl_handle,
                temp_path,
                ..
            } => {
                if !dl_handle.is_null() {
                    // SAFETY: `dl_handle` was opened by `libc::dlopen` and no live `Store` retains a
                    // reference because every `Instance` holds an `Arc<ModuleInner>`.
                    unsafe {
                        let _ = libc::dlclose(*dl_handle);
                    }
                }
                if let Some(path) = temp_path {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    }
}

pub(crate) struct ModuleInner {
    pub(crate) id: u64,
    pub(crate) fingerprint: [u8; 32],
    pub(crate) engine: Engine,
    pub(crate) so: LoadedSo,
    /// Retained raw ELF64 `.so` image for in-memory compiled modules (`None` for file-backed
    /// deserialized modules, which read from `LoadedSo::backing` on demand in `Module::serialize`).
    pub(crate) so_bytes: Option<Vec<u8>>,
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

    /// Returns a deterministic SHA-256 fingerprint of this compiled module's metadata,
    /// ABI version, target architecture, and native code image.
    pub fn fingerprint(&self) -> [u8; 32] {
        self.inner.fingerprint
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
        let so = load_so_from_slice(engine, &so_bytes, None, None)?;
        Self::from_loaded_so(engine, so, Some(so_bytes))
    }

    fn from_loaded_so(engine: &Engine, so: LoadedSo, so_bytes: Option<Vec<u8>>) -> Result<Self> {
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
            let mut reg = engine
                .inner
                .types
                .write()
                .unwrap_or_else(|e| e.into_inner());
            let out = reg
                .register_groups(&meta.raw_type_groups)?
                .into_boxed_slice();
            engine.inner.publish_types_snapshot(&reg);
            out
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

        let fingerprint = {
            let mut h = wtime_aot::sha256::Sha256::new();
            h.update(b"WTMODFP1");
            h.update(&STORE_ABI_VERSION.to_le_bytes());
            let arch = std::env::consts::ARCH.as_bytes();
            h.update(&(arch.len() as u32).to_le_bytes());
            h.update(arch);
            let code_len = so.desc.code_end.saturating_sub(so.desc.code_start);
            h.update(&(code_len as u64).to_le_bytes());
            if so.desc.code_start != 0 && code_len > 0 {
                // SAFETY: `code_start..code_end` is the immutable RX `.text` segment of `so`.
                let text = unsafe {
                    std::slice::from_raw_parts(so.desc.code_start as *const u8, code_len)
                };
                if text.len() <= 65536 {
                    h.update(text);
                } else {
                    const SAMPLE: usize = 16384;
                    let mid = (text.len() - SAMPLE) / 2;
                    h.update(&text[..SAMPLE]);
                    h.update(&text[mid..mid + SAMPLE]);
                    h.update(&text[text.len() - SAMPLE..]);
                }
            }
            h.update(&(meta_slice.len() as u64).to_le_bytes());
            h.update(meta_slice);
            h.finish()
        };

        let id = NEXT_MODULE_ID.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            inner: Arc::new(ModuleInner {
                id,
                fingerprint,
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
        if let Some(ref bytes) = self.inner.so_bytes {
            return Ok(serialize_so_bytes(&self.inner.engine, bytes));
        }
        #[cfg(unix)]
        if let Some((file, offset, len)) = self.inner.so.backing() {
            let mut buf = vec![0u8; *len];
            file.read_exact_at(&mut buf, *offset)?;
            return Ok(serialize_so_bytes(&self.inner.engine, &buf));
        }
        Err(Error::msg(
            "cannot serialize module: backing image unavailable",
        ))
    }

    /// Deserializes a precompiled artifact produced by `Module::serialize`.
    ///
    /// # Safety
    ///
    /// The caller guarantees `bytes` contains a trusted native `StoreCdylib` image produced by
    /// a compatible version of `wtime-aot-host`.
    pub unsafe fn deserialize(engine: &Engine, bytes: impl AsRef<[u8]>) -> Result<Self> {
        let bytes = bytes.as_ref();
        // Fast path: if `bytes` is a page-aligned static slice mapped read-only from `/proc/self/exe`
        // (e.g. `include_bytes!` embedded precompiled modules), map directly from the executable file
        // with zero heap copy and zero `/dev/shm` tmpfs duplication.
        #[cfg(target_os = "linux")]
        if let Some((exe_file, file_offset)) = locate_in_proc_self_exe(bytes) {
            return unsafe {
                Self::deserialize_open_file_range(engine, exe_file, file_offset, bytes.len())
            };
        }
        let (so_rel_off, so_len) = parse_serialized_envelope_slice(engine, bytes)?;
        let so_slice = &bytes[so_rel_off..so_rel_off + so_len];
        let so = load_so_from_slice(engine, so_slice, None, Some(bytes.len()))?;
        Self::from_loaded_so(engine, so, Some(so_slice.to_vec()))
    }

    /// Deserializes a precompiled artifact from an open file.
    ///
    /// # Safety
    ///
    /// Same trust contract as `Module::deserialize`.
    pub unsafe fn deserialize_open_file(engine: &Engine, mut file: File) -> Result<Self> {
        #[cfg(unix)]
        {
            let file_len = file.metadata()?.len();
            if file_len == 0 {
                return Err(Error::msg("invalid serialized wtime-aot-host module magic"));
            }
            if let Ok(len) = usize::try_from(file_len) {
                return unsafe {
                    Self::deserialize_open_file_range(engine, Arc::new(file), 0, len)
                };
            }
        }
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
        let file: Arc<File> = file.into();
        let page_size = host_page_size();
        if !offset.is_multiple_of(page_size as u64) {
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
        let (so_rel_off, so_len) = parse_serialized_envelope_file(engine, &file, offset, len)?;
        let so_file_offset = offset
            .checked_add(so_rel_off as u64)
            .ok_or_else(|| Error::msg("serialized module payload offset overflow"))?;
        // Zero-copy direct `mmap` path when the ELF payload starts on a host page boundary
        // (always true for `WTMS0002`, where `so_rel_off == 0`).
        #[cfg(target_os = "linux")]
        if so_file_offset.is_multiple_of(page_size as u64)
            && let Some(so) =
                load_so_mmap_file(engine, Arc::clone(&file), so_file_offset, so_len, Some(len))?
        {
            return Self::from_loaded_so(engine, so, None);
        }
        // Fallback (e.g. legacy `WTMS0001` whose payload starts after a variable-length header):
        // map via temporary buffer without retaining `so_bytes` because `file` backs `serialize()`.
        let mut buf = vec![0u8; so_len];
        file.read_exact_at(&mut buf, so_file_offset)?;
        let so = load_so_from_slice(
            engine,
            &buf,
            Some((file, so_file_offset, so_len)),
            Some(len),
        )?;
        Self::from_loaded_so(engine, so, None)
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
        match &self.inner.so.mapping {
            SoMapping::Mmap {
                base, image_len, ..
            } if !base.is_null() && *image_len > 0 => {
                let start = *base as *const u8;
                // SAFETY: `start..start.wrapping_add(image_len)` describes the module's mapped image span.
                let end = start.wrapping_add(*image_len);
                start..end
            }
            _ => {
                (self.inner.so.desc.code_start as *const u8)
                    ..(self.inner.so.desc.code_end as *const u8)
            }
        }
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

#[cfg(target_arch = "x86_64")]
fn host_cache_salt() -> [u8; 32] {
    static SALT: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();
    *SALT.get_or_init(|| {
        let mut h = wtime_aot::sha256::Sha256::new();
        h.update(b"WTIME-AOT-HOST-INC-V1\0");
        h.update(&STORE_ABI_VERSION.to_le_bytes());
        h.update(env!("CARGO_PKG_VERSION").as_bytes());
        if let Ok(exe_bytes) = std::fs::read("/proc/self/exe") {
            h.update(&(exe_bytes.len() as u64).to_le_bytes());
            h.update(&exe_bytes);
        } else if let Ok(exe_path) = std::env::current_exe()
            && let Ok(exe_bytes) = std::fs::read(&exe_path)
        {
            h.update(&(exe_bytes.len() as u64).to_le_bytes());
            h.update(&exe_bytes);
        } else {
            // Fallback salt tied to this process's binary address if exe is unreadable.
            h.update(&(host_cache_salt as *const () as usize).to_le_bytes());
        }
        h.finish()
    })
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
        let inc_cache_dir = cfg
            .cache
            .as_ref()
            .and_then(|c| c.config().directory().map(|p| p.to_path_buf()));
        let cache_key_salt = inc_cache_dir.as_ref().map(|_| host_cache_salt());
        let opts = X64CompileOptions {
            target: X64Target::StoreCdylib,
            isa,
            opt: cfg.opt_level != OptLevel::None,
            verify: false,
            jobs: if cfg.parallel_compilation { 0 } else { 1 },
            inc_cache_dir,
            cache_key_salt,
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
    let mut out = Vec::with_capacity(so_bytes.len() + ver.len() + 20);
    out.extend_from_slice(so_bytes);
    out.extend_from_slice(&ver);
    out.extend_from_slice(&(ver.len() as u32).to_le_bytes());
    out.extend_from_slice(&(so_bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(&SERIALIZED_MAGIC_V2);
    out
}

fn parse_serialized_envelope_slice(engine: &Engine, bytes: &[u8]) -> Result<(usize, usize)> {
    const MAX_VER_LEN: usize = 1024;
    if bytes.len() < 20 {
        return Err(Error::msg("invalid serialized wtime-aot-host module magic"));
    }
    let expected_ver = engine_version_bytes(engine);
    let tail = &bytes[bytes.len() - 20..];
    if tail[12..20] == SERIALIZED_MAGIC_V2 {
        let ver_len = u32::from_le_bytes(tail[0..4].try_into().unwrap()) as usize;
        let so_len = u64::from_le_bytes(tail[4..12].try_into().unwrap()) as usize;
        if ver_len > MAX_VER_LEN {
            return Err(Error::msg("corrupt serialized module version length"));
        }
        let total = so_len
            .checked_add(ver_len)
            .and_then(|v| v.checked_add(20))
            .ok_or_else(|| Error::msg("corrupt serialized module trailer"))?;
        if total != bytes.len() {
            return Err(Error::msg("serialized module payload length mismatch"));
        }
        if &bytes[so_len..so_len + ver_len] != expected_ver.as_slice() {
            return Err(Error::msg("serialized module version mismatch with Engine"));
        }
        return Ok((0, so_len));
    }
    if bytes[..8] == SERIALIZED_MAGIC_V1 {
        let ver_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        if ver_len > MAX_VER_LEN {
            return Err(Error::msg("corrupt serialized module version length"));
        }
        let hdr_end = 12usize
            .checked_add(ver_len)
            .and_then(|v| v.checked_add(8))
            .ok_or_else(|| Error::msg("corrupt serialized module header"))?;
        if bytes.len() < hdr_end {
            return Err(Error::msg("truncated serialized module header"));
        }
        if &bytes[12..12 + ver_len] != expected_ver.as_slice() {
            return Err(Error::msg("serialized module version mismatch with Engine"));
        }
        let so_len =
            u64::from_le_bytes(bytes[12 + ver_len..12 + ver_len + 8].try_into().unwrap()) as usize;
        if bytes.len() != hdr_end.saturating_add(so_len) {
            return Err(Error::msg("serialized module payload length mismatch"));
        }
        return Ok((hdr_end, so_len));
    }
    Err(Error::msg("invalid serialized wtime-aot-host module magic"))
}

#[cfg(unix)]
fn parse_serialized_envelope_file(
    engine: &Engine,
    file: &File,
    offset: u64,
    len: usize,
) -> Result<(usize, usize)> {
    const MAX_VER_LEN: usize = 1024;
    if len < 20 {
        return Err(Error::msg("invalid serialized wtime-aot-host module magic"));
    }
    let expected_ver = engine_version_bytes(engine);
    let mut tail = [0u8; 20];
    file.read_exact_at(&mut tail, offset + (len - 20) as u64)?;
    if tail[12..20] == SERIALIZED_MAGIC_V2 {
        let ver_len = u32::from_le_bytes(tail[0..4].try_into().unwrap()) as usize;
        let so_len = u64::from_le_bytes(tail[4..12].try_into().unwrap()) as usize;
        if ver_len > MAX_VER_LEN {
            return Err(Error::msg("corrupt serialized module version length"));
        }
        let total = so_len
            .checked_add(ver_len)
            .and_then(|v| v.checked_add(20))
            .ok_or_else(|| Error::msg("corrupt serialized module trailer"))?;
        if total != len {
            return Err(Error::msg("serialized module payload length mismatch"));
        }
        let mut ver = vec![0u8; ver_len];
        file.read_exact_at(&mut ver, offset + so_len as u64)?;
        if ver.as_slice() != expected_ver.as_slice() {
            return Err(Error::msg("serialized module version mismatch with Engine"));
        }
        return Ok((0, so_len));
    }
    let mut head = [0u8; 12];
    file.read_exact_at(&mut head, offset)?;
    if head[..8] == SERIALIZED_MAGIC_V1 {
        let ver_len = u32::from_le_bytes(head[8..12].try_into().unwrap()) as usize;
        if ver_len > MAX_VER_LEN {
            return Err(Error::msg("corrupt serialized module version length"));
        }
        let hdr_end = 12usize
            .checked_add(ver_len)
            .and_then(|v| v.checked_add(8))
            .ok_or_else(|| Error::msg("corrupt serialized module header"))?;
        if len < hdr_end {
            return Err(Error::msg("truncated serialized module header"));
        }
        let mut ver_and_len = vec![0u8; ver_len + 8];
        file.read_exact_at(&mut ver_and_len, offset + 12)?;
        if &ver_and_len[..ver_len] != expected_ver.as_slice() {
            return Err(Error::msg("serialized module version mismatch with Engine"));
        }
        let so_len =
            u64::from_le_bytes(ver_and_len[ver_len..ver_len + 8].try_into().unwrap()) as usize;
        if len != hdr_end.saturating_add(so_len) {
            return Err(Error::msg("serialized module payload length mismatch"));
        }
        return Ok((hdr_end, so_len));
    }
    Err(Error::msg("invalid serialized wtime-aot-host module magic"))
}

#[inline]
fn host_page_size() -> usize {
    unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(4096) as usize
}

#[inline]
fn align_up(n: usize, align: usize) -> Option<usize> {
    let mask = align - 1;
    n.checked_add(mask).map(|v| v & !mask)
}

#[inline]
fn read_u16_le(buf: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(buf.get(off..off + 2)?.try_into().ok()?))
}

#[inline]
fn read_u32_le(buf: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(buf.get(off..off + 4)?.try_into().ok()?))
}

#[inline]
fn read_u64_le(buf: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes(buf.get(off..off + 8)?.try_into().ok()?))
}

#[cfg(target_os = "linux")]
fn locate_in_proc_self_exe(bytes: &[u8]) -> Option<(Arc<File>, u64)> {
    use std::os::unix::fs::MetadataExt;
    const MIN_STATIC_BYTES: usize = 64 * 1024;
    const MAX_MAPS_BYTES: u64 = 2 * 1024 * 1024;

    let page_size = host_page_size();
    let addr = bytes.as_ptr() as usize;
    let len = bytes.len();
    if len < MIN_STATIC_BYTES || !addr.is_multiple_of(page_size) {
        return None;
    }
    let file = File::open("/proc/self/exe").ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    let exe_dev = meta.dev();
    let exe_ino = meta.ino();
    let exe_len = meta.len();
    let expected_major = (exe_dev >> 8 & 0xfff) | (exe_dev >> 32 & 0xffff_f000);
    let expected_minor = (exe_dev & 0xff) | (exe_dev >> 12 & 0xffff_ff00);

    let mut maps = String::new();
    File::open("/proc/self/maps")
        .ok()?
        .take(MAX_MAPS_BYTES + 1)
        .read_to_string(&mut maps)
        .ok()?;
    if maps.len() as u64 > MAX_MAPS_BYTES {
        return None;
    }
    let addr_end = addr.checked_add(len)?;
    let mut found: Option<(u64, bool)> = None;
    for line in maps.lines() {
        let mut fields = line.split_whitespace();
        let (start_s, end_s) = fields.next()?.split_once('-')?;
        let start = usize::from_str_radix(start_s, 16).ok()?;
        let end = usize::from_str_radix(end_s, 16).ok()?;
        if addr < start || addr_end > end {
            continue;
        }
        if fields.next()? != "r--p" {
            return None;
        }
        let map_off = u64::from_str_radix(fields.next()?, 16).ok()?;
        let (maj_s, min_s) = fields.next()?.split_once(':')?;
        let major = u64::from_str_radix(maj_s, 16).ok()?;
        let minor = u64::from_str_radix(min_s, 16).ok()?;
        let ino: u64 = fields.next()?.parse().ok()?;
        if ino == 0 || ino != exe_ino {
            return None;
        }
        let file_off = map_off.checked_add((addr - start) as u64)?;
        if !file_off.is_multiple_of(page_size as u64) || file_off.checked_add(len as u64)? > exe_len
        {
            return None;
        }
        let dev_matches = major == expected_major && minor == expected_minor;
        if found.replace((file_off, dev_matches)).is_some() {
            return None;
        }
    }
    let (file_off, dev_matches) = found?;
    if !dev_matches {
        // Container/overlayfs/btrfs filesystems can report different device numbers in stat vs maps.
        // Confirm identity by comparing the boundary 64-byte header and trailer (zero bulk fault).
        let mut hdr = [0u8; 64];
        let mut trl = [0u8; 64];
        file.read_exact_at(&mut hdr, file_off).ok()?;
        file.read_exact_at(&mut trl, file_off + (len - 64) as u64)
            .ok()?;
        if hdr != bytes[..64] || trl != bytes[len - 64..] {
            return None;
        }
    }
    Some((Arc::new(file), file_off))
}

#[derive(Clone, Copy)]
struct ElfLoadSeg {
    flags: u32,
    offset: usize,
    vaddr: usize,
    filesz: usize,
}

struct ParsedElfSo {
    loads: Vec<ElfLoadSeg>,
    dyn_seg: Option<(usize, usize)>,
    map_len: usize,
}

fn host_elf_machine() -> Option<u16> {
    #[cfg(target_arch = "x86_64")]
    {
        Some(62)
    }
    #[cfg(target_arch = "aarch64")]
    {
        Some(183)
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        None
    }
}

fn parse_store_elf_headers(header_and_phdrs: &[u8], so_len: usize) -> Option<ParsedElfSo> {
    const PT_LOAD: u32 = 1;
    const PT_DYNAMIC: u32 = 2;

    if header_and_phdrs.len() < 64 || &header_and_phdrs[0..4] != b"\x7fELF" {
        return None;
    }
    if header_and_phdrs[4] != 2 || header_and_phdrs[5] != 1 || header_and_phdrs[6] != 1 {
        return None;
    }
    if read_u16_le(header_and_phdrs, 16)? != 3 // ET_DYN
        || read_u16_le(header_and_phdrs, 18)? != host_elf_machine()?
        || read_u16_le(header_and_phdrs, 54)? != 56
    {
        return None;
    }
    let e_phoff = usize::try_from(read_u64_le(header_and_phdrs, 32)?).ok()?;
    let e_phnum = usize::from(read_u16_le(header_and_phdrs, 56)?);
    if !(2..=16).contains(&e_phnum) {
        return None;
    }
    let ph_end = e_phoff.checked_add(e_phnum.checked_mul(56)?)?;
    if ph_end > header_and_phdrs.len() || ph_end > so_len {
        return None;
    }

    let page_size = host_page_size();
    let mut loads = Vec::with_capacity(e_phnum);
    let mut dyn_seg = None;
    let mut max_end = 0usize;
    for i in 0..e_phnum {
        let base = e_phoff + i * 56;
        let p_type = read_u32_le(header_and_phdrs, base)?;
        let p_flags = read_u32_le(header_and_phdrs, base + 4)?;
        let p_offset = usize::try_from(read_u64_le(header_and_phdrs, base + 8)?).ok()?;
        let p_vaddr = usize::try_from(read_u64_le(header_and_phdrs, base + 16)?).ok()?;
        let p_filesz = usize::try_from(read_u64_le(header_and_phdrs, base + 32)?).ok()?;
        let p_memsz = usize::try_from(read_u64_le(header_and_phdrs, base + 40)?).ok()?;
        if p_type == PT_LOAD {
            if p_filesz != p_memsz
                || p_offset != p_vaddr
                || !p_vaddr.is_multiple_of(page_size)
                || p_offset.checked_add(p_filesz)? > so_len
            {
                return None;
            }
            let seg_end = p_vaddr.checked_add(p_memsz)?;
            max_end = max_end.max(seg_end);
            loads.push(ElfLoadSeg {
                flags: p_flags,
                offset: p_offset,
                vaddr: p_vaddr,
                filesz: p_filesz,
            });
        } else if p_type == PT_DYNAMIC {
            dyn_seg = Some((p_vaddr, p_filesz));
        }
    }
    if loads.len() < 2 || max_end == 0 {
        return None;
    }
    // Verify `PT_LOAD` segments are strictly ordered and page-disjoint, and segment 0 starts at 0.
    loads.sort_by_key(|s| s.vaddr);
    if loads[0].vaddr != 0 {
        return None;
    }
    for pair in loads.windows(2) {
        let prev_end = align_up(pair[0].vaddr.checked_add(pair[0].filesz)?, page_size)?;
        if prev_end > pair[1].vaddr {
            return None;
        }
    }
    let map_len = align_up(max_end, page_size)?;
    Some(ParsedElfSo {
        loads,
        dyn_seg,
        map_len,
    })
}

struct MmapGuard {
    base: *mut u8,
    len: usize,
}

impl MmapGuard {
    fn disarm(mut self) -> *mut u8 {
        let ptr = self.base;
        self.base = std::ptr::null_mut();
        ptr
    }
}

impl Drop for MmapGuard {
    fn drop(&mut self) {
        if !self.base.is_null() && self.len > 0 {
            unsafe {
                let _ = libc::munmap(self.base.cast(), self.len);
            }
        }
    }
}

#[inline]
fn elf_flags_to_prot(flags: u32) -> libc::c_int {
    let mut prot = 0;
    if (flags & 4) != 0 {
        prot |= libc::PROT_READ;
    }
    if (flags & 2) != 0 {
        prot |= libc::PROT_WRITE;
    }
    if (flags & 1) != 0 {
        prot |= libc::PROT_EXEC;
    }
    prot
}

/// Resolves `wt_store_get_desc` from an already-mapped zero-relocation `StoreCdylib` ELF64 image.
/// Returns `Ok(None)` if the image uses dynamic relocations or external dependencies (caller falls
/// back to `dlopen`), or `Ok(Some(ptr))` pointing into an executable `PT_LOAD` segment.
unsafe fn resolve_mapped_get_desc(
    base: *const u8,
    parsed: &ParsedElfSo,
) -> Option<WtStoreGetDescFn> {
    const DT_NULL: u64 = 0;
    const DT_NEEDED: u64 = 1;
    const DT_PLTRELSZ: u64 = 2;
    const DT_HASH: u64 = 4;
    const DT_STRTAB: u64 = 5;
    const DT_SYMTAB: u64 = 6;
    const DT_RELA: u64 = 7;
    const DT_STRSZ: u64 = 10;
    const DT_SYMENT: u64 = 11;
    const DT_INIT: u64 = 12;
    const DT_REL: u64 = 17;
    const DT_JMPREL: u64 = 23;
    const DT_INIT_ARRAY: u64 = 25;
    const DT_RELR: u64 = 36;

    let (dyn_vaddr, dyn_len) = parsed.dyn_seg?;
    // Ensure `.dynamic` lies completely inside a readable mapped `PT_LOAD` segment.
    let dyn_in_load = parsed.loads.iter().any(|s| {
        (s.flags & 4) != 0
            && dyn_vaddr >= s.vaddr
            && dyn_vaddr
                .checked_add(dyn_len)
                .is_some_and(|e| e <= s.vaddr + s.filesz)
    });
    if !dyn_in_load || dyn_len < 16 {
        return None;
    }
    let dyn_slice = unsafe { std::slice::from_raw_parts(base.add(dyn_vaddr), dyn_len) };

    let mut hash_off = None;
    let mut strtab_off = None;
    let mut symtab_off = None;
    let mut strsz = None;
    let mut syment = None;
    for chunk in dyn_slice.as_chunks::<16>().0 {
        let tag = u64::from_le_bytes(chunk[0..8].try_into().ok()?);
        let val = u64::from_le_bytes(chunk[8..16].try_into().ok()?);
        match tag {
            DT_NULL => break,
            DT_NEEDED | DT_PLTRELSZ | DT_RELA | DT_REL | DT_JMPREL | DT_RELR | DT_INIT
            | DT_INIT_ARRAY => return None,
            DT_HASH => hash_off = usize::try_from(val).ok(),
            DT_STRTAB => strtab_off = usize::try_from(val).ok(),
            DT_SYMTAB => symtab_off = usize::try_from(val).ok(),
            DT_STRSZ => strsz = usize::try_from(val).ok(),
            DT_SYMENT => syment = usize::try_from(val).ok(),
            _ => {}
        }
    }
    if syment? != 24 {
        return None;
    }
    let seg0_end = parsed.loads[0].filesz;
    let hash_off = hash_off?;
    let strtab_off = strtab_off?;
    let symtab_off = symtab_off?;
    let strsz = strsz?;
    if hash_off.checked_add(8)? > seg0_end || strtab_off.checked_add(strsz)? > seg0_end {
        return None;
    }
    let seg0 = unsafe { std::slice::from_raw_parts(base, seg0_end) };
    let nsyms = read_u32_le(seg0, hash_off + 4)? as usize;
    let symtab_end = symtab_off.checked_add(nsyms.checked_mul(24)?)?;
    if symtab_end > seg0_end {
        return None;
    }
    let strtab = &seg0[strtab_off..strtab_off + strsz];
    for sym_idx in 1..nsyms {
        let off = symtab_off + sym_idx * 24;
        let st_name = read_u32_le(seg0, off)? as usize;
        if st_name >= strtab.len() {
            continue;
        }
        let rest = &strtab[st_name..];
        let nul = rest.iter().position(|&b| b == 0)?;
        if &rest[..nul] == b"wt_store_get_desc" {
            let st_value = usize::try_from(read_u64_le(seg0, off + 8)?).ok()?;
            let in_exec = parsed.loads.iter().any(|s| {
                (s.flags & 1) != 0 && st_value >= s.vaddr && st_value < s.vaddr + s.filesz
            });
            if !in_exec {
                return None;
            }
            let fn_ptr = unsafe { base.add(st_value) };
            return Some(unsafe { std::mem::transmute::<*const u8, WtStoreGetDescFn>(fn_ptr) });
        }
    }
    None
}

#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn flush_icache_range(start: *const u8, len: usize) {
    if len == 0 {
        return;
    }
    let ctr: u64;
    unsafe {
        std::arch::asm!("mrs {}, ctr_el0", out(reg) ctr, options(nomem, nostack, preserves_flags));
    }
    let dmin = 4usize << ((ctr >> 16) & 0xf);
    let imin = 4usize << (ctr & 0xf);
    let beg = start as usize;
    let end = beg.saturating_add(len);
    let mut p = beg & !(dmin - 1);
    while p < end {
        unsafe {
            std::arch::asm!("dc cvau, {}", in(reg) p, options(nostack, preserves_flags));
        }
        p += dmin;
    }
    unsafe {
        std::arch::asm!("dsb ish", options(nostack, preserves_flags));
    }
    let mut p = beg & !(imin - 1);
    while p < end {
        unsafe {
            std::arch::asm!("ic ivau, {}", in(reg) p, options(nostack, preserves_flags));
        }
        p += imin;
    }
    unsafe {
        std::arch::asm!("dsb ish", "isb", options(nostack, preserves_flags));
    }
}

/// Zero-copy `mmap` loader for a page-aligned `StoreCdylib` ELF64 `.so` inside an open file.
/// Returns `Ok(None)` if the ELF layout is ineligible (caller falls back to `load_so_from_slice`).
#[cfg(target_os = "linux")]
fn load_so_mmap_file(
    engine: &Engine,
    file: Arc<File>,
    so_file_offset: u64,
    so_len: usize,
    reported_image_len: Option<usize>,
) -> Result<Option<LoadedSo>> {
    if so_len < 64 {
        return Ok(None);
    }
    let probe_len = so_len.min(64 + 16 * 56);
    let mut probe = vec![0u8; probe_len];
    file.read_exact_at(&mut probe, so_file_offset)?;
    let Some(parsed) = parse_store_elf_headers(&probe, so_len) else {
        return Ok(None);
    };

    let raw_base = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            parsed.map_len,
            libc::PROT_NONE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if raw_base == libc::MAP_FAILED {
        return Err(Error::msg("mmap reserve for StoreCdylib failed"));
    }
    let guard = MmapGuard {
        base: raw_base.cast::<u8>(),
        len: parsed.map_len,
    };

    let fd = file.as_raw_fd();
    for seg in &parsed.loads {
        if seg.filesz == 0 {
            continue;
        }
        let dst = unsafe { guard.base.add(seg.vaddr) };
        let foff = libc::off_t::try_from(so_file_offset + seg.offset as u64)
            .map_err(|_| Error::msg("StoreCdylib mmap file offset overflow"))?;
        let ptr = unsafe {
            libc::mmap(
                dst.cast(),
                seg.filesz,
                elf_flags_to_prot(seg.flags),
                libc::MAP_PRIVATE | libc::MAP_FIXED,
                fd,
                foff,
            )
        };
        if ptr == libc::MAP_FAILED || ptr != dst.cast() {
            return Err(Error::msg("mmap PT_LOAD segment for StoreCdylib failed"));
        }
    }

    let Some(get_desc) = (unsafe { resolve_mapped_get_desc(guard.base, &parsed) }) else {
        return Ok(None);
    };
    let mut desc = StoreModuleDesc::default();
    unsafe {
        get_desc(&mut desc);
    }
    if desc.abi_version != STORE_ABI_VERSION {
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
    let base = guard.disarm();
    Ok(Some(LoadedSo {
        mapping: SoMapping::Mmap {
            base,
            map_len: parsed.map_len,
            image_len: reported_image_len.unwrap_or(parsed.map_len),
            backing: Some((file, so_file_offset, so_len)),
        },
        desc,
        engine: engine.clone(),
    }))
}

/// Direct anonymous `mmap` loader for an in-memory `StoreCdylib` ELF64 `.so` slice (avoids
/// `/dev/shm` tmpfs copies and glibc `dlopen` global lock contention).
#[cfg(target_os = "linux")]
fn load_so_mmap_slice(
    engine: &Engine,
    so_bytes: &[u8],
    backing: Option<(Arc<File>, u64, usize)>,
    reported_image_len: Option<usize>,
) -> Result<Option<LoadedSo>> {
    let Some(parsed) = parse_store_elf_headers(so_bytes, so_bytes.len()) else {
        return Ok(None);
    };
    let page_size = host_page_size();
    let raw_base = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            parsed.map_len,
            libc::PROT_NONE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if raw_base == libc::MAP_FAILED {
        return Err(Error::msg("mmap reserve for StoreCdylib slice failed"));
    }
    let guard = MmapGuard {
        base: raw_base.cast::<u8>(),
        len: parsed.map_len,
    };

    for seg in &parsed.loads {
        if seg.filesz == 0 {
            continue;
        }
        let seg_map_len = align_up(seg.filesz, page_size)
            .ok_or_else(|| Error::msg("StoreCdylib segment size overflow"))?;
        let dst = unsafe { guard.base.add(seg.vaddr) };
        let rc =
            unsafe { libc::mprotect(dst.cast(), seg_map_len, libc::PROT_READ | libc::PROT_WRITE) };
        if rc != 0 {
            return Err(Error::msg("mprotect RW for StoreCdylib segment failed"));
        }
        unsafe {
            std::ptr::copy_nonoverlapping(so_bytes.as_ptr().add(seg.offset), dst, seg.filesz);
        }
        #[cfg(target_arch = "aarch64")]
        if (seg.flags & 1) != 0 {
            unsafe { flush_icache_range(dst, seg.filesz) };
        }
        let rc = unsafe { libc::mprotect(dst.cast(), seg_map_len, elf_flags_to_prot(seg.flags)) };
        if rc != 0 {
            return Err(Error::msg("mprotect final prot for StoreCdylib failed"));
        }
    }

    let Some(get_desc) = (unsafe { resolve_mapped_get_desc(guard.base, &parsed) }) else {
        return Ok(None);
    };
    let mut desc = StoreModuleDesc::default();
    unsafe {
        get_desc(&mut desc);
    }
    if desc.abi_version != STORE_ABI_VERSION {
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
    let base = guard.disarm();
    Ok(Some(LoadedSo {
        mapping: SoMapping::Mmap {
            base,
            map_len: parsed.map_len,
            image_len: reported_image_len.unwrap_or(parsed.map_len),
            backing,
        },
        desc,
        engine: engine.clone(),
    }))
}

fn load_so_from_slice(
    engine: &Engine,
    so_bytes: &[u8],
    backing: Option<(Arc<File>, u64, usize)>,
    reported_image_len: Option<usize>,
) -> Result<LoadedSo> {
    #[cfg(target_os = "linux")]
    {
        let backing_clone = backing.clone();
        if let Some(loaded) =
            load_so_mmap_slice(engine, so_bytes, backing_clone, reported_image_len)?
        {
            return Ok(loaded);
        }
    }
    load_so_dlopen(engine, so_bytes, backing)
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

/// Fallback ELF64 `.so` loader using a globally unique tmpfs (`/dev/shm`, fallback `temp_dir`)
/// path + `dlopen`, then immediately unlinking the path once mapped.
fn load_so_dlopen(
    engine: &Engine,
    so_bytes: &[u8],
    backing: Option<(Arc<File>, u64, usize)>,
) -> Result<LoadedSo> {
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
        mapping: SoMapping::Dlopen {
            dl_handle,
            temp_path,
            backing,
        },
        desc,
        engine: engine.clone(),
    })
}
