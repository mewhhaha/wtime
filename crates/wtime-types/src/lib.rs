#![forbid(unsafe_code)]
//! Shared source-level types and policy for the wtime WebAssembly engine.
//! Runtime handles and canonical type identities deliberately live elsewhere.
mod error;
mod features;
mod indices;
pub mod parallel;
mod types;
pub use error::*;
pub use features::*;
pub use indices::*;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn t005_error_retains_category_and_absolute_offset() {
        let e = Error::new(ErrorKind::Decode, "truncated section").at(17);
        assert_eq!(e.offset, Some(17));
        assert_eq!(e.to_string(), "Decode: truncated section at byte 17");
        assert_ne!(e.kind, ErrorKind::DisabledFeature);
    }
    #[test]
    fn t004_profiles_have_explicit_feature_policy() {
        assert!(!FeatureSet::mvp().multi_value);
        assert!(FeatureSet::stable_core().multi_value);
        assert!(!FeatureSet::stable_core().stack_switching);
        assert!(FeatureSet::default().stack_switching);
    }
    #[test]
    fn t006_source_types_preserve_exactness_and_recursion_groups() {
        assert_ne!(
            HeapType::Defined(TypeIdx(3)),
            HeapType::ExactDefined(TypeIdx(3))
        );
        let group = TypeGroup {
            explicit: true,
            types: vec![SubType::final_type(CompositeType::Func(FuncType::new(
                vec![ValType::I32],
                vec![ValType::I64],
            )))],
        };
        assert!(group.explicit);
        assert_eq!(group.types.len(), 1);
    }
    #[test]
    fn t006_limits_and_vectors_keep_wire_widths() {
        assert!(Limits::new(u64::MAX, Some(u64::MAX)).is_valid());
        assert!(!Limits::new(2, Some(1)).is_valid());
        assert_eq!(V128::from_u128(1).0[0], 1);
        assert_eq!(V128::from_u128(u128::MAX).to_u128(), u128::MAX);
        assert_eq!(MemoryType::default().page_size_log2, 16);
    }
}
