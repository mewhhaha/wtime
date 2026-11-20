use super::*;

fn leb(mut n: usize, out: &mut Vec<u8>) {
    loop {
        let b = (n & 127) as u8;
        n >>= 7;
        out.push(b | if n != 0 { 128 } else { 0 });
        if n == 0 {
            break;
        }
    }
}
fn section(module: &mut Vec<u8>, id: u8, bytes: &[u8]) {
    module.push(id);
    leb(bytes.len(), module);
    module.extend(bytes);
}
fn function(params: &[u8], results: &[u8], instructions: &[u8]) -> Vec<u8> {
    let mut m = b"\0asm\x01\0\0\0".to_vec();
    let mut ty = vec![1, 0x60];
    leb(params.len(), &mut ty);
    ty.extend(params);
    leb(results.len(), &mut ty);
    ty.extend(results);
    section(&mut m, 1, &ty);
    section(&mut m, 3, &[1, 0]);
    let mut body = vec![0];
    body.extend(instructions);
    body.push(0x0b);
    let mut code = vec![1];
    leb(body.len(), &mut code);
    code.extend(body);
    section(&mut m, 10, &code);
    m
}
fn valid(m: &[u8]) -> ValidatedModule {
    validate(
        m,
        &FeatureSet::all_phase3_core(),
        &ResourceLimits::default(),
    )
    .unwrap()
}
fn invalid_module(m: &[u8]) {
    assert_eq!(
        validate(
            m,
            &FeatureSet::all_phase3_core(),
            &ResourceLimits::default()
        )
        .unwrap_err()
        .kind,
        ErrorKind::Validation
    );
}

#[test]
fn t025_case01_unknown_function_type() {
    let mut m = b"\0asm\x01\0\0\0".to_vec();
    section(&mut m, 3, &[1, 0]);
    section(&mut m, 10, &[1, 2, 0, 0x0b]);
    invalid_module(&m);
}
#[test]
fn t026_case01_duplicate_export_names() {
    let mut m = function(&[], &[], &[]);
    let code = m.split_off(18);
    section(&mut m, 7, &[2, 1, b'f', 0, 0, 1, b'f', 0, 0]);
    m.extend(code);
    invalid_module(&m);
}
#[test]
fn t026_case02_start_requires_empty_signature() {
    let mut m = function(&[0x7f], &[], &[]);
    let code = m.split_off(19);
    section(&mut m, 8, &[0]);
    m.extend(code);
    invalid_module(&m);
}
#[test]
fn t027_case02_initializer_requires_one_value() {
    let mut m = b"\0asm\x01\0\0\0".to_vec();
    section(&mut m, 6, &[1, 0x7f, 0, 0x41, 0, 0x41, 1, 0x0b]);
    invalid_module(&m);
}
#[test]
fn t027_case03_float_initializer_preserves_raw_bits() {
    let mut m = b"\0asm\x01\0\0\0".to_vec();
    section(&mut m, 6, &[1, 0x7d, 0, 0x43, 1, 0, 0xc0, 0x7f, 0x0b]);
    valid(&m);
}
#[test]
fn t028_case02_block_cannot_pop_below_frame() {
    invalid_module(&function(
        &[],
        &[],
        &[0x41, 0, 0x02, 0x40, 0x1a, 0x0b, 0x1a],
    ));
}
#[test]
fn t029_case02_br_if_preserves_branch_operands() {
    valid(&function(
        &[],
        &[0x7f],
        &[0x02, 0x7f, 0x41, 7, 0x41, 0, 0x0d, 0, 0x0b],
    ));
}
#[test]
fn t030_case02_call_requires_arguments() {
    invalid_module(&function(&[0x7f], &[], &[0x10, 0]));
}
#[test]
fn t031_case01_numeric_operand_width() {
    invalid_module(&function(&[], &[0x7e], &[0x41, 0, 0x42, 0, 0x7c]));
}
#[test]
fn t031_case02_conversion_result_width() {
    valid(&function(&[], &[0x7e], &[0x41, 0, 0xac]));
    invalid_module(&function(&[], &[0x7f], &[0x41, 0, 0xac]));
}
#[test]
fn t033_case01_failed_final_body_returns_no_module() {
    let mut m = b"\0asm\x01\0\0\0".to_vec();
    section(&mut m, 1, &[1, 0x60, 0, 0]);
    section(&mut m, 3, &[2, 0, 0]);
    section(&mut m, 10, &[2, 2, 0, 0x0b, 3, 0, 0x1a, 0x0b]);
    invalid_module(&m);
}
#[test]
fn t033_case02_caller_bytes_cannot_mutate_certificate() {
    let mut bytes = function(&[], &[], &[]);
    let module = valid(&bytes);
    let original = module.decoded().bytes.clone();
    bytes.fill(0);
    assert_eq!(module.decoded().bytes.as_ref(), original.as_ref());
}
#[test]
fn t033_case03_policy_provenance_is_retained() {
    let mut features = FeatureSet::all_phase3_core();
    let mut limits = ResourceLimits::default();
    let module = validate(b"\0asm\x01\0\0\0", &features, &limits).unwrap();
    features.simd = false;
    limits.max_types = 1;
    assert!(module.features().simd);
    assert_ne!(module.limits().max_types, limits.max_types);
    assert!(!features.simd);
}

#[test]
fn t028_case03_deep_declared_subtyping_is_iterative_and_budgeted() {
    const N: usize = 50_000;
    let mut m = b"\0asm\x01\0\0\0".to_vec();
    let mut types = Vec::new();
    leb(N + 1, &mut types);
    for i in 0..N {
        types.push(0x50);
        types.push(u8::from(i != 0));
        if i != 0 {
            leb(i - 1, &mut types);
        }
        types.extend([0x5f, 0]);
    }
    types.extend([0x60, 1, 0x63]);
    leb(N - 1, &mut types);
    types.extend([1, 0x63, 0]);
    section(&mut m, 1, &types);
    let mut funcs = vec![1];
    leb(N, &mut funcs);
    section(&mut m, 3, &funcs);
    section(&mut m, 10, &[1, 4, 0, 0x20, 0, 0x0b]);
    valid(&m);
    let limits = ResourceLimits {
        max_type_work: 1000,
        ..ResourceLimits::default()
    };
    assert_eq!(
        validate(&m, &FeatureSet::all_phase3_core(), &limits)
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
}

#[test]
fn t025_case02_declared_type_count_budget() {
    let mut m = b"\0asm\x01\0\0\0".to_vec();
    section(&mut m, 1, &[2, 0x60, 0, 0, 0x60, 0, 0]);
    let limits = ResourceLimits {
        max_types: 1,
        ..ResourceLimits::default()
    };
    assert_eq!(
        validate(&m, &FeatureSet::default(), &limits)
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
}
#[test]
fn t025_case03_disabled_type_and_malformed_type_differ() {
    let mut disabled = b"\0asm\x01\0\0\0".to_vec();
    section(&mut disabled, 1, &[1, 0x60, 1, 0x7b, 0]);
    assert_eq!(
        validate(&disabled, &FeatureSet::mvp(), &ResourceLimits::default())
            .unwrap_err()
            .kind,
        ErrorKind::DisabledFeature
    );
    let mut malformed = b"\0asm\x01\0\0\0".to_vec();
    section(&mut malformed, 1, &[1, 0x60, 1, 0x01, 0]);
    assert_eq!(
        validate(
            &malformed,
            &FeatureSet::default(),
            &ResourceLimits::default()
        )
        .unwrap_err()
        .kind,
        ErrorKind::Decode
    );
}
#[test]
fn t027_case01_mutable_global_not_constant_source() {
    let mut m = b"\0asm\x01\0\0\0".to_vec();
    section(&mut m, 2, &[1, 1, b'm', 1, b'g', 3, 0x7f, 1]);
    section(&mut m, 6, &[1, 0x7f, 0, 0x23, 0, 0x0b]);
    invalid_module(&m);
}
#[test]
fn t030_case01_immutable_global_cannot_be_set() {
    let mut m = function(&[], &[], &[0x41, 0, 0x24, 0]);
    let code = m.split_off(18);
    section(&mut m, 6, &[1, 0x7f, 0, 0x41, 0, 0x0b]);
    m.extend(code);
    invalid_module(&m);
}
#[test]
fn t030_case03_indirect_call_requires_function_table() {
    let mut m = function(&[], &[], &[0x41, 0, 0x11, 0, 0]);
    let code = m.split_off(18);
    section(&mut m, 4, &[1, 0x6f, 0, 1]);
    m.extend(code);
    invalid_module(&m);
}
#[test]
fn t032_case01_memory_alignment_limit() {
    let mut m = function(&[], &[], &[0x41, 0, 0x28, 3, 0, 0x1a]);
    let code = m.split_off(18);
    section(&mut m, 5, &[1, 0, 1]);
    m.extend(code);
    invalid_module(&m);
}
#[test]
fn t032_case02_data_segment_index_bounds() {
    let mut m = function(&[], &[], &[0xfc, 9, 0]);
    let code = m.split_off(18);
    section(&mut m, 12, &[0]);
    m.extend(code);
    invalid_module(&m);
}
#[test]
fn t032_case03_active_data_bounds_are_dynamic() {
    let mut m = b"\0asm\x01\0\0\0".to_vec();
    section(&mut m, 5, &[1, 0, 0]);
    section(&mut m, 11, &[1, 0, 0x41, 0, 0x0b, 1, 42]);
    valid(&m);
}

#[test]
fn t026_case03_enabled_mutable_import_and_export() {
    let mut m = b"\0asm\x01\0\0\0".to_vec();
    section(&mut m, 2, &[1, 1, b'm', 1, b'g', 3, 0x7f, 1]);
    section(&mut m, 7, &[1, 1, b'g', 3, 0]);
    valid(&m);
}
#[test]
fn t029_case01_loop_label_uses_inputs_not_results() {
    valid(&function(&[], &[0x7f], &[0x03, 0x7f, 0x0c, 0, 0x0b]));
    invalid_module(&function(&[], &[0x7f], &[0x02, 0x7f, 0x0c, 0, 0x0b]));
}
#[test]
fn t029_case03_branch_table_requires_compatible_labels() {
    invalid_module(&function(
        &[],
        &[0x7f],
        &[
            0x02, 0x7f, 0x02, 0x7d, 0x43, 0, 0, 0, 0, 0x41, 0, 0x0e, 1, 0, 1, 0x0b, 0x1a, 0x41, 0,
            0x0b,
        ],
    ));
}

#[test]
fn t090_custom_page_count_fits_memory_size_result() {
    let mut m = b"\0asm\x01\0\0\0".to_vec();
    let mut memory = vec![1, 8];
    leb(1usize << 32, &mut memory);
    memory.push(0);
    section(&mut m, 5, &memory);
    invalid_module(&m);
    let mut m = b"\0asm\x01\0\0\0".to_vec();
    let mut memory = vec![1, 8];
    leb(u32::MAX as usize, &mut memory);
    memory.push(0);
    section(&mut m, 5, &memory);
    valid(&m);
}

#[test]
fn t028_case03_deep_control_nesting_is_bounded() {
    let mut instructions = Vec::new();
    for _ in 0..50_000 {
        instructions.extend([0x02, 0x40]);
    }
    instructions.extend(std::iter::repeat_n(0x0b, 50_000));
    let bytes = function(&[], &[], &instructions);
    valid(&bytes);
    let limits = ResourceLimits {
        max_stack_height: 1024,
        ..ResourceLimits::default()
    };
    assert_eq!(
        validate(&bytes, &FeatureSet::all_phase3_core(), &limits)
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
}

#[test]
fn t121_try_table_accepts_abstract_reference_block_results() {
    valid(&function(&[], &[0x6e], &[0x1f, 0x6e, 0, 0xd0, 0x6e, 0x0b]));
}
#[test]
fn t121_try_table_indexed_signature_obeys_multi_value_gate() {
    let bytes = function(&[], &[], &[0x1f, 0, 0, 0x0b]);
    valid(&bytes);
    let features = FeatureSet {
        multi_value: false,
        ..FeatureSet::all_phase3_core()
    };
    assert_eq!(
        validate(&bytes, &features, &ResourceLimits::default())
            .unwrap_err()
            .kind,
        ErrorKind::DisabledFeature
    );
}

#[test]
fn t026_export_indices_use_each_external_index_space() {
    for kind in 0..=4 {
        let mut bytes = b"\0asm\x01\0\0\0".to_vec();
        section(&mut bytes, 7, &[1, 1, b'x', kind, 0]);
        invalid_module(&bytes);
    }
    let mut bytes = function(&[], &[], &[]);
    let code = bytes.split_off(18);
    section(&mut bytes, 7, &[1, 1, b'f', 0, 0]);
    bytes.extend(code);
    valid(&bytes);
}
#[test]
fn t026_start_result_and_index_boundaries() {
    let mut bytes = function(&[], &[0x7f], &[0x41, 0]);
    let code = bytes.split_off(19);
    section(&mut bytes, 8, &[0]);
    bytes.extend(code);
    invalid_module(&bytes);
    let mut bytes = b"\0asm\x01\0\0\0".to_vec();
    section(&mut bytes, 8, &[0]);
    invalid_module(&bytes);
}
#[test]
fn t027_case03_reference_initializer_must_match_global_type() {
    let mut bytes = b"\0asm\x01\0\0\0".to_vec();
    section(&mut bytes, 6, &[1, 0x6e, 0, 0x41, 0, 0x0b]);
    invalid_module(&bytes);
}

#[test]
fn t028_polymorphic_untyped_select_still_rejects_known_reference() {
    invalid_module(&function(&[], &[0x6e], &[0, 0xd0, 0x6e, 0x41, 0, 0x1b]));
    valid(&function(&[], &[0x7f], &[0, 0x41, 7, 0x41, 0, 0x1b]));
}

#[test]
fn t107_function_parameter_contravariance_and_result_covariance() {
    let module = |parent_param, parent_result, child_param, child_result| {
        let mut bytes = b"\0asm\x01\0\0\0".to_vec();
        section(
            &mut bytes,
            1,
            &[
                2,
                0x50,
                0,
                0x60,
                1,
                parent_param,
                1,
                parent_result,
                0x50,
                1,
                0,
                0x60,
                1,
                child_param,
                1,
                child_result,
            ],
        );
        bytes
    };
    valid(&module(0x6d, 0x6e, 0x6e, 0x6d));
    invalid_module(&module(0x6e, 0x6e, 0x6d, 0x6e));
    invalid_module(&module(0x6e, 0x6d, 0x6e, 0x6e));
    valid(&module(0x6e, 0x6d, 0x6e, 0x6d));
}
#[test]
fn t107_declared_array_const_covariance_and_mutable_invariance() {
    let module = |parent_element, child_element, mutable| {
        let mut bytes = b"\0asm\x01\0\0\0".to_vec();
        section(
            &mut bytes,
            1,
            &[
                2,
                0x50,
                0,
                0x5e,
                parent_element,
                mutable,
                0x50,
                1,
                0,
                0x5e,
                child_element,
                mutable,
            ],
        );
        bytes
    };
    valid(&module(0x6e, 0x6d, 0));
    invalid_module(&module(0x6d, 0x6e, 0));
    valid(&module(0x6d, 0x6d, 1));
    invalid_module(&module(0x6e, 0x6d, 1));
    invalid_module(&module(0x6d, 0x6e, 1));
}

/// Validates `m` with function bodies on one thread and on four.
fn on_threads(m: &[u8], features: &FeatureSet, limits: &ResourceLimits) -> [String; 2] {
    [1, 4].map(|threads| {
        let limits = ResourceLimits { compile_threads: threads, ..limits.clone() };
        format!("{:?}", validate(m, features, &limits).map(|v| v.function_bounds().to_vec()))
    })
}

#[test]
fn parallel_bodies_report_the_first_error_in_body_order() {
    // Five bodies of type [] -> []: two invalid (i32.add and drop on an
    // empty stack), the first of them in body 1.
    let mut m = b"\0asm\x01\0\0\0".to_vec();
    section(&mut m, 1, &[1, 0x60, 0, 0]);
    section(&mut m, 3, &[5, 0, 0, 0, 0, 0]);
    let mut code = vec![5];
    for body in [&[0x0b][..], &[0x6a, 0x0b], &[0x0b], &[0x1a, 0x0b], &[0x0b]] {
        code.push(body.len() as u8 + 1);
        code.push(0);
        code.extend(body);
    }
    section(&mut m, 10, &code);
    let [one, four] = on_threads(&m, &FeatureSet::default(), &ResourceLimits::default());
    assert!(one.starts_with("Err"), "{one}");
    assert_eq!(one, four);
    // Malformed bodies: the first is reported, before a malformed size.
    let mut m = b"\0asm\x01\0\0\0".to_vec();
    section(&mut m, 1, &[1, 0x60, 0, 0]);
    section(&mut m, 3, &[4, 0, 0, 0, 0]);
    let mut code = vec![4];
    for body in [&[0x0b][..], &[0xff, 0x0b], &[0xfe, 0x0b]] {
        code.push(body.len() as u8 + 1);
        code.push(0);
        code.extend(body);
    }
    code.push(0x7f);
    section(&mut m, 10, &code);
    let [one, four] = on_threads(&m, &FeatureSet::default(), &ResourceLimits::default());
    assert!(one.starts_with("Err"), "{one}");
    assert_eq!(one, four);
}

#[test]
fn parallel_bodies_spend_the_type_work_budget_in_body_order() {
    // A chain of declared subtypes and bodies that each check a subtyping
    // along it: one body's work fits the budget, two do not.
    const N: usize = 300;
    let mut m = b"\0asm\x01\0\0\0".to_vec();
    let mut types = Vec::new();
    leb(N + 1, &mut types);
    for i in 0..N {
        types.push(0x50);
        types.push(u8::from(i != 0));
        if i != 0 {
            leb(i - 1, &mut types);
        }
        types.extend([0x5f, 0]);
    }
    types.extend([0x60, 1, 0x63]);
    leb(N - 1, &mut types);
    types.extend([1, 0x63, 0]);
    section(&mut m, 1, &types);
    let mut funcs = vec![4];
    for _ in 0..4 {
        leb(N, &mut funcs);
    }
    section(&mut m, 3, &funcs);
    let mut code = vec![4];
    for _ in 0..4 {
        code.extend([4, 0, 0x20, 0, 0x0b]);
    }
    section(&mut m, 10, &code);
    let features = FeatureSet::all_phase3_core();
    let [one, four] = on_threads(&m, &features, &ResourceLimits::default());
    assert!(one.starts_with("Ok"), "{one}");
    assert_eq!(one, four);
    let mut exhausted = 0;
    for budget in (50..4 * N + 100).step_by(7) {
        let limits = ResourceLimits { max_type_work: budget, ..ResourceLimits::default() };
        let [one, four] = on_threads(&m, &features, &limits);
        assert_eq!(one, four, "budget {budget}");
        exhausted += usize::from(one.contains("budget"));
    }
    assert!(exhausted > 0);
}
