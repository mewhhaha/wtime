use wtime_types::*;
use wtime_validate::*;
#[test]
fn t121_rejects_invalid_exception_types_and_targets() {
    for bytes in [
        include_bytes!("fixtures/exceptions/bad_label.wasm").as_slice(),
        include_bytes!("fixtures/exceptions/bad_payload.wasm").as_slice(),
        include_bytes!("fixtures/exceptions/bad_throw.wasm").as_slice(),
        include_bytes!("fixtures/exceptions/bad_ref.wasm").as_slice(),
    ] {
        assert_eq!(
            validate(bytes, &FeatureSet::default(), &ResourceLimits::default())
                .unwrap_err()
                .kind,
            ErrorKind::Validation
        );
    }
}
#[test]
fn t121_exception_feature_gate() {
    let f = FeatureSet {
        exception_handling: false,
        stack_switching: false,
        ..FeatureSet::default()
    };
    assert!(
        validate(
            include_bytes!("../../wtime-runtime/tests/fixtures/exceptions/cross_call.wasm"),
            &f,
            &ResourceLimits::default()
        )
        .is_err()
    );
}
