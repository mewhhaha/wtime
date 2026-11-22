//! Explicit feature policy, separate from binary validity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeatureSet {
    pub mutable_globals: bool,
    pub saturating_float_to_int: bool,
    pub sign_extension: bool,
    pub multi_value: bool,
    pub reference_types: bool,
    pub bulk_memory: bool,
    pub simd: bool,
    pub tail_call: bool,
    pub extended_const: bool,
    pub function_references: bool,
    pub gc: bool,
    pub multi_memory: bool,
    pub relaxed_simd: bool,
    pub exception_handling: bool,
    pub memory64: bool,
    pub threads: bool,
    pub compact_imports: bool,
    pub wide_arithmetic: bool,
    pub stack_switching: bool,
    pub custom_page_sizes: bool,
    pub custom_descriptors: bool,
    pub branch_hinting: bool,
}
impl FeatureSet {
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
}
impl Default for FeatureSet {
    fn default() -> Self {
        Self::all_phase3_core()
    }
}

/// Host policy budgets. Exceeding these is a resource error, not invalid Wasm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceLimits {
    pub max_module_bytes: usize,
    pub max_types: usize,
    pub max_functions: usize,
    pub max_memories: usize,
    pub max_tables: usize,
    pub max_globals: usize,
    pub max_tags: usize,
    pub max_locals: usize,
    pub max_total_locals: usize,
    pub max_stack_height: usize,
    pub max_call_depth: usize,
    pub max_memory_pages: u64,
    /// Per-memory byte budget, independent of the module's custom page size.
    pub max_memory_bytes: u64,
    /// Aggregate live memory bytes charged once per backing within a Store.
    pub max_store_memory_bytes: u64,
    pub max_store_table_elements: u64,
    pub max_store_objects: usize,
    pub max_table_elements: u64,
    /// Aggregate canonical identities retained by an Engine.
    pub max_canonical_types: usize,
    pub max_canonical_type_bytes: usize,
    /// Aggregate allocated GC object headers and typed field storage.
    pub max_gc_bytes: usize,
    pub max_continuation_bytes: usize,
    /// Maximum roots/edges examined by a collection.
    pub max_gc_mark_work: usize,
    pub max_type_work: usize,
    pub max_rec_group_size: usize,
    pub max_instructions: usize,
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
            max_instructions: 10_000_000,
            fuel: None,
            compile_threads: 0,
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
            0 if section_bytes >= 512 * 1024 => {
                std::thread::available_parallelism().map_or(1, |n| n.get()).min(8)
            }
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
