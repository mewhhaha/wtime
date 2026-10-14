//! T162 fixtures are assembled separately with wasm-tools 1.261.0.
//! Readable .wat and exact .wasm inputs live in fixtures/feature_profiles.
use wtime_runtime::*;

fn engine(features: FeatureSet) -> Engine {
    Engine::new(Config {
        features,
        ..Config::default()
    })
    .unwrap()
}
fn disabled(name: &str) -> FeatureSet {
    let mut p = FeatureSet::all_phase3_core();
    match name {
        "mutable_globals" => p.mutable_globals = false,
        "saturating_float_to_int" => p.saturating_float_to_int = false,
        "sign_extension" => p.sign_extension = false,
        "multi_value" => p.multi_value = false,
        "reference_types" => {
            p.reference_types = false;
            p.function_references = false;
            p.gc = false;
            p.custom_descriptors = false;
            p.stack_switching = false;
        }
        "bulk_memory" => p.bulk_memory = false,
        "simd" => {
            p.simd = false;
            p.relaxed_simd = false;
        }
        "tail_call" => p.tail_call = false,
        "extended_const" => p.extended_const = false,
        "function_references" => {
            p.function_references = false;
            p.gc = false;
            p.custom_descriptors = false;
            p.stack_switching = false;
        }
        "gc" => {
            p.gc = false;
            p.custom_descriptors = false;
        }
        "multi_memory" => p.multi_memory = false,
        "relaxed_simd" => p.relaxed_simd = false,
        "exception_handling" => {
            p.exception_handling = false;
            p.stack_switching = false;
        }
        "memory64" => p.memory64 = false,
        "threads" => p.threads = false,
        "compact_imports" => p.compact_imports = false,
        "wide_arithmetic" => p.wide_arithmetic = false,
        "stack_switching" => p.stack_switching = false,
        "custom_page_sizes" => p.custom_page_sizes = false,
        "custom_descriptors" => p.custom_descriptors = false,
        _ => panic!("unknown fixture feature {name}"),
    }
    p.validate().unwrap();
    p
}
fn gate(name: &str, bytes: &[u8]) {
    engine(FeatureSet::all_phase3_core())
        .compile(bytes)
        .unwrap();
    let e = engine(disabled(name));
    assert_eq!(
        e.compile(bytes).unwrap_err().kind,
        ErrorKind::DisabledFeature,
        "{name}"
    );
    // Turning off one proposal must not disable ordinary calls/imports.
    e.compile(include_bytes!(
        "fixtures/feature_profiles/baseline_host.wasm"
    ))
    .unwrap();
}
macro_rules! feature_case {
    ($test:ident,$feature:literal,$file:literal) => {
        #[test]
        fn $test() {
            gate(
                $feature,
                include_bytes!(concat!("fixtures/feature_profiles/", $file, ".wasm")),
            );
        }
    };
}
feature_case!(t162_case_01, "custom_descriptors", "custom_descriptors");
#[test]
fn t162_case_02() {
    gate(
        "custom_page_sizes",
        include_bytes!("fixtures/feature_profiles/custom_page_sizes.wasm"),
    );
    gate(
        "custom_descriptors",
        include_bytes!("fixtures/feature_profiles/custom_descriptors_type.wasm"),
    );
}
feature_case!(t162_case_03, "stack_switching", "stack_switching");
feature_case!(t162_case_04, "mutable_globals", "mutable_globals");
feature_case!(
    t162_case_05,
    "saturating_float_to_int",
    "saturating_float_to_int"
);
feature_case!(t162_case_06, "sign_extension", "sign_extension");
feature_case!(t162_case_07, "multi_value", "multi_value");
feature_case!(t162_case_08, "reference_types", "reference_types");
feature_case!(t162_case_09, "bulk_memory", "bulk_memory");
feature_case!(t162_case_10, "simd", "simd");
feature_case!(t162_case_11, "tail_call", "tail_call");
feature_case!(t162_case_12, "extended_const", "extended_const");
feature_case!(t162_case_13, "function_references", "function_references");
feature_case!(t162_case_14, "gc", "gc");
feature_case!(t162_case_15, "multi_memory", "multi_memory");
feature_case!(t162_case_16, "relaxed_simd", "relaxed_simd");
feature_case!(t162_case_17, "exception_handling", "exception_handling");
feature_case!(t162_case_18, "memory64", "memory64");
feature_case!(t162_case_19, "threads", "threads");
feature_case!(t162_case_20, "compact_imports", "compact_imports");
feature_case!(t162_case_21, "wide_arithmetic", "wide_arithmetic");
#[test]
fn t162_case_22_initializer_and_encoding_paths() {
    gate(
        "gc",
        include_bytes!("fixtures/feature_profiles/gc_initializer.wasm"),
    );
    gate(
        "function_references",
        include_bytes!("fixtures/feature_profiles/function_reference_initializer.wasm"),
    );
    gate(
        "bulk_memory",
        include_bytes!("fixtures/feature_profiles/bulk_memory_passive.wasm"),
    );
    gate(
        "threads",
        include_bytes!("fixtures/feature_profiles/atomic_opcode.wasm"),
    );
    gate(
        "simd",
        include_bytes!("fixtures/feature_profiles/simd_opcode.wasm"),
    );
    gate(
        "memory64",
        include_bytes!("fixtures/feature_profiles/memory64_table.wasm"),
    );
}
#[test]
fn t162_case_23_stable_and_all_phase3() {
    let stable = engine(FeatureSet::stable_core());
    let all = engine(FeatureSet::all_phase3_core());
    let phase3: [&[u8]; 7] = [
        include_bytes!("fixtures/feature_profiles/threads.wasm"),
        include_bytes!("fixtures/feature_profiles/compact_imports.wasm"),
        include_bytes!("fixtures/feature_profiles/wide_arithmetic.wasm"),
        include_bytes!("fixtures/feature_profiles/stack_switching.wasm"),
        include_bytes!("fixtures/feature_profiles/custom_page_sizes.wasm"),
        include_bytes!("fixtures/feature_profiles/custom_descriptors.wasm"),
        include_bytes!("fixtures/feature_profiles/custom_descriptors_type.wasm"),
    ];
    for bytes in phase3 {
        all.compile(bytes).unwrap();
        assert_eq!(
            stable.compile(bytes).unwrap_err().kind,
            ErrorKind::DisabledFeature
        );
    }
    stable
        .compile(include_bytes!("fixtures/feature_profiles/simd.wasm"))
        .unwrap();
    stable
        .compile(include_bytes!("fixtures/feature_profiles/gc.wasm"))
        .unwrap();
}
#[test]
fn t162_case_24_dependency_closure() {
    for p in [
        FeatureSet {
            reference_types: false,
            ..FeatureSet::all_phase3_core()
        },
        FeatureSet {
            function_references: false,
            ..FeatureSet::all_phase3_core()
        },
        FeatureSet {
            gc: false,
            ..FeatureSet::all_phase3_core()
        },
        FeatureSet {
            simd: false,
            ..FeatureSet::all_phase3_core()
        },
        FeatureSet {
            exception_handling: false,
            ..FeatureSet::all_phase3_core()
        },
    ] {
        assert_eq!(
            Engine::new(Config {
                features: p,
                ..Config::default()
            })
            .unwrap_err()
            .kind,
            ErrorKind::DisabledFeature
        );
    }
    for p in [
        FeatureSet::mvp(),
        FeatureSet::stable_core(),
        FeatureSet::all_phase3_core(),
    ] {
        p.validate().unwrap();
    }
}
fn host_roundtrip(features: FeatureSet) {
    let e = engine(features);
    let m = e
        .compile(include_bytes!(
            "fixtures/feature_profiles/baseline_host.wasm"
        ))
        .unwrap();
    let mut s = Store::new(&e);
    let f = s
        .create_host_func(
            FuncType::new(vec![ValType::I32], vec![ValType::I32]),
            |args| match args {
                [Value::I32(n)] => Ok(vec![Value::I32(n + 1)]),
                _ => Err(Error::new(ErrorKind::Host, "unexpected host arguments")),
            },
        )
        .unwrap();
    let mut l = Linker::new(&e);
    l.define("m", "f", External::Func(f)).unwrap();
    let i = l.instantiate(&mut s, &m).unwrap();
    let External::Func(run) = i.export(&s, "run").unwrap() else {
        panic!("expected function")
    };
    assert_eq!(
        s.invoke(run, &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(7)])
    );
}
#[test]
fn t162_case_25_unrelated_host_calls() {
    host_roundtrip(FeatureSet::mvp());
    host_roundtrip(FeatureSet::stable_core());
    host_roundtrip(FeatureSet::all_phase3_core());
    host_roundtrip(disabled("simd"));
    host_roundtrip(disabled("gc"));
}
#[test]
fn t162_case_26_ignorable_branch_hints() {
    // Advisory custom metadata has no guest semantics. A disabled optimizer must
    // ignore it rather than reject an otherwise ordinary program.
    let b = include_bytes!("fixtures/feature_profiles/branch_hinting.wasm");
    let mut p = FeatureSet::all_phase3_core();
    p.branch_hinting = false;
    engine(p).compile(b).unwrap();
    engine(FeatureSet::all_phase3_core()).compile(b).unwrap();
}
#[test]
fn t162_case_27_syntax_only_feature_paths() {
    gate(
        "mutable_globals",
        include_bytes!("fixtures/feature_profiles/mutable_global_import.wasm"),
    );
    gate(
        "custom_descriptors",
        include_bytes!("fixtures/feature_profiles/custom_exact_heap.wasm"),
    );
    gate(
        "multi_value",
        include_bytes!("fixtures/feature_profiles/multi_value_block.wasm"),
    );
    gate(
        "custom_page_sizes",
        include_bytes!("fixtures/feature_profiles/custom_default_page_syntax.wasm"),
    );
}
#[test]
fn t162_case_28_typed_indirect_calls() {
    let b = include_bytes!("fixtures/feature_profiles/typed_call_indirect.wasm");
    gate("function_references", b);
    let e = engine(FeatureSet::stable_core());
    let m = e.compile(b).unwrap();
    let mut s = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
    let External::Func(f) = i.export(&s, "run").unwrap() else {
        panic!("expected function")
    };
    assert_eq!(
        s.invoke(f, &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(7)])
    );
}
