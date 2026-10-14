use wtime_types::*;
#[test]
fn t004_case_03() {
    // Valid binary grammar, invalid body: i32.add has no input operands.
    let invalid = [
        0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 3, 2, 1, 0, 10, 5, 1, 3, 0, 106, 11,
    ];
    for stack in [1, 100, 100_000] {
        let limits = ResourceLimits {
            max_stack_height: stack,
            ..ResourceLimits::default()
        };
        assert_eq!(
            wtime_validate::validate(&invalid, &FeatureSet::default(), &limits)
                .unwrap_err()
                .kind,
            ErrorKind::Validation
        );
    }
}
#[test]
fn t004_contradictory_profile_rejected_before_decoding() {
    let mut features = FeatureSet::mvp();
    features.custom_descriptors = true;
    assert_eq!(
        wtime_validate::validate(&[], &features, &ResourceLimits::default())
            .unwrap_err()
            .kind,
        ErrorKind::DisabledFeature
    );
}
