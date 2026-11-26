//! Explicit feature policy, separate from binary validity.
/// WebAssembly proposal flags controlling which language features are accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeatureSet {
    /// Mutable imported and exported globals.
    pub mutable_globals: bool,
    /// Non-trapping saturating float-to-integer conversions.
    pub saturating_float_to_int: bool,
    /// Integer sign-extension operators.
    pub sign_extension: bool,
    /// Multi-value function and block signatures.
    pub multi_value: bool,
    /// Reference types (`funcref`, `externref`), multiple tables, and table instructions.
    pub reference_types: bool,
    /// Bulk memory and table operations and passive segments.
    pub bulk_memory: bool,
    /// Fixed-width 128-bit SIMD (`v128`).
    pub simd: bool,
    /// Tail-call instructions (`return_call`, `return_call_indirect`, `return_call_ref`).
    pub tail_call: bool,
    /// Extended constant expressions (`i32`/`i64` add, sub, and mul).
    pub extended_const: bool,
    /// Typed function references (`call_ref`, non-null references, table initializers).
    pub function_references: bool,
    /// Garbage-collected structs, arrays, `i31`, and recursive type groups.
    pub gc: bool,
    /// Multiple linear memories per module.
    pub multi_memory: bool,
    /// Relaxed SIMD instructions.
    pub relaxed_simd: bool,
    /// Exception handling (`try_table`, `throw`, `throw_ref`, and tags).
    pub exception_handling: bool,
    /// 64-bit linear memories and tables (`memory64`).
    pub memory64: bool,
    /// Shared linear memories and atomic memory instructions.
    pub threads: bool,
    /// Compact import section encoding.
    pub compact_imports: bool,
    /// Wide integer arithmetic instructions.
    pub wide_arithmetic: bool,
    /// Typed continuations and stack switching.
    pub stack_switching: bool,
    /// Custom linear memory page sizes.
    pub custom_page_sizes: bool,
    /// Custom GC descriptors and exact reference types.
    pub custom_descriptors: bool,
    /// Branch-hinting custom section metadata.
    pub branch_hinting: bool,
}
impl FeatureSet {
    /// Returns the minimal WebAssembly 1.0 (MVP) feature profile with all post-MVP proposals disabled.
    pub const fn mvp() -> Self {
        Self {
            mutable_globals: false,
            saturating_float_to_int: false,
            sign_extension: false,
            multi_value: false,
            reference_types: false,
            bulk_memory: false,
            simd: false,
            tail_call: false,
            extended_const: false,
            function_references: false,
            gc: false,
            multi_memory: false,
            relaxed_simd: false,
            exception_handling: false,
            memory64: false,
            threads: false,
            compact_imports: false,
            wide_arithmetic: false,
            stack_switching: false,
            custom_page_sizes: false,
            custom_descriptors: false,
            branch_hinting: false,
        }
    }
    /// Returns the profile enabling standardized core WebAssembly features up through memory64, GC, and exceptions.
    pub const fn stable_core() -> Self {
        Self {
            mutable_globals: true,
            saturating_float_to_int: true,
            sign_extension: true,
            multi_value: true,
            reference_types: true,
            bulk_memory: true,
            simd: true,
            tail_call: true,
            extended_const: true,
            function_references: true,
            gc: true,
            multi_memory: true,
            relaxed_simd: true,
            exception_handling: true,
            memory64: true,
            threads: false,
            compact_imports: false,
            wide_arithmetic: false,
            stack_switching: false,
            custom_page_sizes: false,
            custom_descriptors: false,
            branch_hinting: false,
        }
    }
    /// Returns the profile enabling all supported core and Phase 3 proposals.
    pub const fn all_phase3_core() -> Self {
        Self {
            mutable_globals: true,
            saturating_float_to_int: true,
            sign_extension: true,
            multi_value: true,
            reference_types: true,
            bulk_memory: true,
            simd: true,
            tail_call: true,
            extended_const: true,
            function_references: true,
            gc: true,
            multi_memory: true,
            relaxed_simd: true,
            exception_handling: true,
            memory64: true,
            threads: true,
            compact_imports: true,
            wide_arithmetic: true,
            stack_switching: true,
            custom_page_sizes: true,
            custom_descriptors: true,
            branch_hinting: true,
        }
    }
    /// Returns a conservative feature profile for untrusted multi-tenant workers.
    ///
    /// Keeps standardized Wasm Core 3.0 features ([`Self::stable_core`]) while keeping
    /// `threads` (shared memory and atomics) disabled so untrusted guests cannot construct
    /// concurrent shared-memory timer channels, and keeping experimental Phase 3 proposals off.
    pub const fn untrusted_worker() -> Self {
        Self::stable_core()
    }
}
impl Default for FeatureSet {
    fn default() -> Self {
        Self::all_phase3_core()
    }
}

/// Host policy budgets. Exceeding these is a resource error, not invalid Wasm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceLimits {
    /// Maximum accepted WebAssembly module binary size in bytes.
    pub max_module_bytes: usize,
    /// Maximum number of defined types or type groups in a module.
    pub max_types: usize,
    /// Maximum number of functions (imported and defined) in a module.
    pub max_functions: usize,
    /// Maximum number of linear memories in a module.
    pub max_memories: usize,
    /// Maximum number of tables in a module.
    pub max_tables: usize,
    /// Maximum number of globals in a module.
    pub max_globals: usize,
    /// Maximum number of tags in a module.
    pub max_tags: usize,
    /// Maximum number of locals (parameters plus declared locals) in a single function.
    pub max_locals: usize,
    /// Maximum aggregate number of locals across all functions in a module;
    /// at runtime also bounds aggregate active frame local slots and active
    /// control labels across live call frames in a `Store`.
    pub max_total_locals: usize,
    /// Maximum operand stack height during validation and execution.
    pub max_stack_height: usize,
    /// Maximum active call frame depth during execution.
    pub max_call_depth: usize,
    /// Maximum page count permitted for a single linear memory.
    pub max_memory_pages: u64,
    /// Per-memory byte budget, independent of the module's custom page size.
    pub max_memory_bytes: u64,
    /// Aggregate live memory bytes charged once per backing within a Store.
    pub max_store_memory_bytes: u64,
    /// Aggregate live table element budget across all tables in a `Store`.
    pub max_store_table_elements: u64,
    /// Aggregate live object and handle budget within a `Store`.
    pub max_store_objects: usize,
    /// Maximum number of elements permitted in a single table.
    pub max_table_elements: u64,
    /// Aggregate canonical identities retained by an Engine.
    pub max_canonical_types: usize,
    /// Aggregate retained byte budget for canonical type definitions in an `Engine`.
    pub max_canonical_type_bytes: usize,
    /// Aggregate allocated GC object headers and typed field storage.
    pub max_gc_bytes: usize,
    /// Aggregate retained byte budget for captured continuations in a `Store`.
    pub max_continuation_bytes: usize,
    /// Maximum roots/edges examined by a collection.
    pub max_gc_mark_work: usize,
    /// Maximum work budget for type validation, subtyping, and canonical interning.
    pub max_type_work: usize,
    /// Maximum number of types permitted in a single recursive type group.
    pub max_rec_group_size: usize,
    /// Maximum number of element segments in a module.
    pub max_elements: usize,
    /// Maximum number of data segments in a module.
    pub max_data_segments: usize,
    /// Maximum decoded instructions and variable-length immediate elements in a module.
    pub max_instructions: usize,
    /// Optional per-invocation instruction fuel limit (`None` disables fuel metering).
    pub fuel: Option<u64>,
    /// Host threads that decoding and validation may use for function
    /// bodies: 1 keeps them on the calling thread; 0, the default, uses the
    /// host's available parallelism (at most 8) for code sections of at
    /// least 512 KiB. The result, including which error is reported, does
    /// not depend on it.
    pub compile_threads: usize,
}
impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_module_bytes: 64 * 1024 * 1024,
            max_types: 100_000,
            max_functions: 100_000,
            max_memories: 100,
            max_tables: 100,
            max_globals: 100_000,
            max_tags: 1_000,
            max_locals: 50_000,
            max_total_locals: 1_000_000,
            max_stack_height: 100_000,
            max_call_depth: 10_000,
            max_memory_pages: 65_536,
            max_memory_bytes: 4 * 1024 * 1024 * 1024,
            max_store_memory_bytes: 4 * 1024 * 1024 * 1024,
            max_store_table_elements: 10_000_000,
            max_store_objects: 1_000_000,
            max_table_elements: 10_000_000,
            max_canonical_types: 1_000_000,
            max_canonical_type_bytes: 256 * 1024 * 1024,
            max_gc_bytes: 256 * 1024 * 1024,
            max_continuation_bytes: 256 * 1024 * 1024,
            max_gc_mark_work: 10_000_000,
            max_type_work: 10_000_000,
            max_rec_group_size: 10_000,
            max_elements: 100_000,
            max_data_segments: 100_000,
            max_instructions: 10_000_000,
            fuel: None,
            compile_threads: 0,
        }
    }
}
impl ResourceLimits {
    /// Returns tighter resource budgets suitable for untrusted multi-tenant workers.
    ///
    /// Bounds module size (16 MiB), linear memory (64 MiB), GC heap (32 MiB), continuations
    /// (16 MiB), tables (65,536 elements), call depth (1,024), stack height (16,384),
    /// per-invocation fuel (10,000,000 instructions), and single-threaded compilation
    /// (`compile_threads = 1`) so untrusted inputs cannot fan out host worker threads.
    pub const fn untrusted_worker() -> Self {
        Self {
            max_module_bytes: 16 * 1024 * 1024,
            max_types: 10_000,
            max_functions: 10_000,
            max_memories: 4,
            max_tables: 8,
            max_globals: 1_000,
            max_tags: 64,
            max_locals: 4_096,
            max_total_locals: 65_536,
            max_stack_height: 16_384,
            max_call_depth: 1_024,
            max_memory_pages: 1_024,
            max_memory_bytes: 64 * 1024 * 1024,
            max_store_memory_bytes: 64 * 1024 * 1024,
            max_store_table_elements: 65_536,
            max_store_objects: 65_536,
            max_table_elements: 65_536,
            max_canonical_types: 65_536,
            max_canonical_type_bytes: 16 * 1024 * 1024,
            max_gc_bytes: 32 * 1024 * 1024,
            max_continuation_bytes: 16 * 1024 * 1024,
            max_gc_mark_work: 1_000_000,
            max_type_work: 1_000_000,
            max_rec_group_size: 1_024,
            max_elements: 10_000,
            max_data_segments: 10_000,
            max_instructions: 1_000_000,
            fuel: Some(10_000_000),
            compile_threads: 1,
        }
    }
}

impl FeatureSet {
    /// Reject contradictory semantic profiles before any module is decoded.
    pub fn validate(&self) -> crate::Result<()> {
        let requirements = [
            (
                self.custom_descriptors,
                self.gc && self.function_references,
                "custom descriptors require GC and function references",
            ),
            (
                self.gc,
                self.function_references && self.reference_types,
                "GC requires function references and reference types",
            ),
            (
                self.function_references,
                self.reference_types,
                "function references require reference types",
            ),
            (
                self.stack_switching,
                self.function_references && self.exception_handling,
                "stack switching requires function references and exception handling",
            ),
            (self.relaxed_simd, self.simd, "relaxed SIMD requires SIMD"),
        ];
        for (enabled, dependencies, message) in requirements {
            if enabled && !dependencies {
                return Err(crate::Error::new(
                    crate::ErrorKind::DisabledFeature,
                    message,
                ));
            }
        }
        Ok(())
    }
}
impl ResourceLimits {
    /// The threads to use for function bodies of a code section of
    /// `section_bytes` (see `compile_threads`); 1 means the calling thread.
    pub fn body_threads(&self, section_bytes: usize) -> usize {
        match self.compile_threads {
            0 if section_bytes >= 512 * 1024 => std::thread::available_parallelism()
                .map_or(1, |n| n.get())
                .min(8),
            0 => 1,
            n => n,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn t004_case_01() {
        let mut profile = FeatureSet::mvp();
        profile.custom_descriptors = true;
        assert!(profile.validate().is_err());
        assert!(FeatureSet::all_phase3_core().validate().is_ok());
    }
    #[test]
    fn t004_case_02() {
        let profile = FeatureSet::stable_core();
        assert!(profile.validate().is_ok());
        assert!(
            !profile.stack_switching
                && !profile.custom_descriptors
                && !profile.wide_arithmetic
                && !profile.threads
        );
    }
    #[test]
    fn t004_dependency_combinations() {
        let mut profile = FeatureSet::mvp();
        profile.relaxed_simd = true;
        assert!(profile.validate().is_err());
        profile.simd = true;
        assert!(profile.validate().is_ok());
        profile.stack_switching = true;
        assert!(profile.validate().is_err());
    }
}
