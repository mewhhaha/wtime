//! Stable single-observation protocol shared with the frozen-core adapter.
#[cfg(not(feature = "legacy-api"))]
#[path = "entry_batch_driver.rs"]
mod entry_batch_driver;
#[path = "fuel_policy.rs"]
mod fuel_policy;
#[cfg(not(feature = "legacy-api"))]
#[path = "integration_driver.rs"]
mod integration_driver;
#[path = "representative_driver.rs"]
mod representative_driver;
#[cfg(not(feature = "legacy-api"))]
#[path = "continuation_driver.rs"]
mod continuation_driver;
#[cfg(not(feature = "legacy-api"))]
#[path = "gc_exception_driver.rs"]
mod gc_exception_driver;
#[cfg(not(feature = "legacy-api"))]
#[path = "gc_warm_driver.rs"]
mod gc_warm_driver;
#[cfg(not(feature = "legacy-api"))]
#[path = "gc_roots_driver.rs"]
mod gc_roots_driver;
#[path = "memory_driver.rs"]
mod memory_driver;
#[path = "scalar_driver.rs"]
mod scalar_driver;
#[cfg(not(feature = "legacy-api"))]
#[path = "thread_driver.rs"]
mod thread_driver;
#[cfg(not(feature = "legacy-api"))]
#[path = "tier_state_driver.rs"]
mod tier_state_driver;
use std::{hint::black_box, time::Instant};
use wtime_runtime::{Config, Engine, External, Linker, RunOutcome, Store, Value};
use wtime_types::FeatureSet;

const TYPED_NUMERIC_MAX_BATCH: usize = 1_000_000;
const TYPED_MEMORY_MAX_BATCH: usize = 100_000;

fn configured_engine(config: Config, mode: &str) -> Engine {
    #[cfg(feature = "legacy-api")]
    {
        assert_eq!(mode, "decoded", "unavailable legacy execution policy");
        Engine::new(config).unwrap()
    }
    #[cfg(not(feature = "legacy-api"))]
    {
        use wtime_runtime::ExecutionPolicy;
        let policy = match mode {
            "decoded" => ExecutionPolicy::Decoded,
            "portable" => ExecutionPolicy::Portable,
            "reference" => ExecutionPolicy::Reference,
            "compact" => ExecutionPolicy::Compact,
            "register" => ExecutionPolicy::Register,
            "experimental" => ExecutionPolicy::Experimental,
            "native" => ExecutionPolicy::Native,
            _ => panic!("unknown execution policy"),
        };
        Engine::with_execution_policy(config, policy).unwrap()
    }
}
fn engine(mode: &str) -> Engine {
    engine_with_fuel(mode, fuel_policy::FuelArgument::OMITTED)
}
fn engine_with_fuel(mode: &str, fuel: fuel_policy::FuelArgument) -> Engine {
    let mut config = Config {
        features: FeatureSet::mvp(),
        ..Config::default()
    };
    fuel.configure(&mut config);
    configured_engine(
        config,
        mode,
    )
}
fn available_modes() -> String {
    #[cfg(feature = "legacy-api")]
    { "\"decoded\"".to_owned() }
    #[cfg(not(feature = "legacy-api"))]
    {
        #[allow(unused_mut)]
        let mut modes = vec!["\"decoded\"", "\"portable\""];
        #[cfg(feature = "execution-reference")]
        modes.push("\"reference\"");
        #[cfg(feature = "compact-executor")]
        modes.push("\"compact\"");
        #[cfg(feature = "register-executor")]
        modes.push("\"register\"");
        modes.join(",")
    }
}
fn available_types() -> &'static str {
    #[cfg(feature = "legacy-api")]
    { "\"i32\",\"i64\",\"f32_bits\",\"f64_bits\",\"v128_bits\"" }
    #[cfg(not(feature = "legacy-api"))]
    { "\"i32\",\"i64\",\"f32_bits\",\"f64_bits\",\"v128_bits\",\"reference\"" }
}
fn call(store: &mut Store, func: wtime_runtime::Func, input: i32) -> i32 {
    match store.invoke(func, black_box(&[Value::I32(input)])).unwrap() {
        RunOutcome::Returned(values) => match values.as_slice() {
            [Value::I32(value)] => *value,
            other => panic!("unexpected results {other:?}"),
        },
        other => panic!("unexpected outcome {other:?}"),
    }
}
fn exported(store: &Store, instance: wtime_runtime::Instance) -> wtime_runtime::Func {
    match instance.export(store, "run").unwrap() {
        External::Func(func) => func,
        _ => panic!("run must be a function"),
    }
}
fn measured(batch: usize, mut f: impl FnMut() -> i32, expected: i32) -> f64 {
    let start = Instant::now();
    let mut result = 0;
    for _ in 0..batch {
        result = black_box(f());
    }
    let ns = start.elapsed().as_nanos() as f64 / batch as f64;
    assert_eq!(result, expected, "oracle mismatch");
    ns.max(1.0)
}
pub fn try_main(args: &[String]) -> bool {
    #[cfg(not(feature = "legacy-api"))]
    if entry_batch_driver::try_main(args) { return true; }
    #[cfg(not(feature = "legacy-api"))]
    if integration_driver::try_main(args) { return true; }
    if representative_driver::try_main(args) { return true; }
    #[cfg(not(feature = "legacy-api"))]
    if thread_driver::try_main(args) {
        return true;
    }
    #[cfg(not(feature = "legacy-api"))]
    if continuation_driver::try_main(args) {
        return true;
    }
    #[cfg(not(feature = "legacy-api"))]
    if tier_state_driver::try_main(args) {
        return true;
    }
    #[cfg(not(feature = "legacy-api"))]
    if gc_exception_driver::try_main(args) {
        return true;
    }
    #[cfg(not(feature = "legacy-api"))]
    if gc_warm_driver::try_main(args) {
        return true;
    }
    #[cfg(not(feature = "legacy-api"))]
    if gc_roots_driver::try_main(args) {
        return true;
    }
    if memory_driver::try_main(args) {
        return true;
    }
    if scalar_driver::try_main(args) {
        return true;
    }
    #[cfg(all(not(feature = "legacy-api"), feature = "execution-reference"))]
    if snapshot(args) {
        return true;
    }
    #[cfg(all(not(feature = "legacy-api"), feature = "performance-counters"))]
    if diagnostic(args) {
        return true;
    }
    if typed_main(args) {
        return true;
    }
    #[cfg(all(not(feature = "legacy-api"), not(feature = "runtime-only")))]
    if capability(args) {
        return true;
    }
    if args.first().map(String::as_str) == Some("--protocol") {
        assert_eq!(args.len(), 1);
        println!(
            "{{\"schema_version\":2,\"engine\":\"wtime\",\"modes\":[{}],\"types\":[{}],\"features\":[\"mvp\",\"stable-core\",\"all-phase3-core\",\"mvp-saturating\",\"simd\",\"simd-relaxed\"],\"thread_driver\":{},\"instrumented\":{},\"source_fuel_contract\":{},\"batch_limits\":{{\"typed_numeric\":{},\"typed_memory\":{}}}}}",
            available_modes(),
            available_types(),
            cfg!(not(feature = "legacy-api")),
            cfg!(all(
                not(feature = "legacy-api"),
                any(
                    feature = "performance-counters",
                    feature = "execution-reference"
                )
            )),
            fuel_policy::CONTRACT,
            TYPED_NUMERIC_MAX_BATCH,
            TYPED_MEMORY_MAX_BATCH,
        );
        return true;
    }
    if args.first().map(String::as_str) != Some("--single-case") {
        return false;
    }
    let fuel = fuel_policy::FuelArgument::parse(args, 8)
        .expect("--single-case FILE CASE INPUT EXPECTED STAGE BATCH MODE [--source-fuel none|50000000]");
    let bytes = std::fs::read(&args[1]).unwrap();
    let case = &args[2];
    assert!(
        !case.is_empty()
            && case
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    );
    let input = args[3].parse::<i32>().unwrap();
    let expected = args[4].parse::<i32>().unwrap();
    let stage = args[5].as_str();
    let requested_batch = args[6].parse::<usize>().unwrap();
    assert!(requested_batch <= 1_000_000,
            "{case}: requested single-case batch {requested_batch} exceeds 1000000");
    let mode = args[7].as_str();
    assert!(
        case != "prepare_large" || stage == "prepare",
        "prepare-only fixture"
    );
    assert!(
        [
            "prepare",
            "instantiate",
            "first_call",
            "warm_call",
            "load_and_call"
        ]
        .contains(&stage)
    );
    // Preflight is outside each timing boundary; prepare-only has no call oracle.
    let e = engine_with_fuel(mode, fuel);
    let module = e.compile(&bytes).unwrap();
    if case != "prepare_large" {
        let mut store = Store::new(&e);
        let instance = Linker::new(&e).instantiate(&mut store, &module).unwrap();
        let func = exported(&store, instance);
        assert_eq!(call(&mut store, func, input), expected, "oracle mismatch");
    }
    let mut batch = 1;
    let ns = match stage {
        "prepare" => {
            let e = engine_with_fuel(mode, fuel);
            let start = Instant::now();
            let module = e.compile(black_box(&bytes)).unwrap();
            let ns = start.elapsed().as_nanos() as f64;
            black_box(module);
            ns
        }
        "load_and_call" => {
            let e = engine_with_fuel(mode, fuel);
            let start = Instant::now();
            let module = e.compile(&bytes).unwrap();
            let mut store = Store::new(&e);
            let instance = Linker::new(&e).instantiate(&mut store, &module).unwrap();
            let func = exported(&store, instance);
            let result = call(&mut store, func, input);
            let ns = start.elapsed().as_nanos() as f64;
            assert_eq!(result, expected, "oracle mismatch");
            ns
        }
        "instantiate" => {
            let linker = Linker::new(&e);
            let mut store = Store::new(&e);
            let start = Instant::now();
            let instance = linker.instantiate(&mut store, &module).unwrap();
            let ns = start.elapsed().as_nanos() as f64;
            let func = exported(&store, instance);
            assert_eq!(call(&mut store, func, input), expected, "oracle mismatch");
            ns
        }
        "first_call" | "warm_call" => {
            let mut store = Store::new(&e);
            let instance = Linker::new(&e).instantiate(&mut store, &module).unwrap();
            let func = exported(&store, instance);
            if stage == "warm_call" {
                for _ in 0..5 {
                    assert_eq!(call(&mut store, func, input), expected);
                }
                batch = if requested_batch == 0 {
                    let one = measured(1, || call(&mut store, func, input), expected);
                    (20_000_000.0 / one).ceil().clamp(1.0, 1_000_000.0) as usize
                } else {
                    requested_batch
                };
            }
            measured(batch, || call(&mut store, func, input), expected)
        }
        _ => unreachable!(),
    }
    .max(1.0);
    let result = if case == "prepare_large" {
        "null".to_owned()
    } else {
        expected.to_string()
    };
    println!(
        "{{\"schema_version\":2,\"case\":\"{case}\",\"stage\":\"{stage}\",\"input\":{input},\"expected\":{expected},\"result\":{result},\"batch\":{batch},\"ns_per_operation\":{ns:.3},\"mode\":\"{mode}\"{}}}",
        fuel.json_suffix(),
    );
    true
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(all(test, feature = "execution-reference", not(feature = "legacy-api")))]
mod p127_tests {
    use super::*;

    #[test]
    fn p127_single_case_builder_uses_selected_fuel_at_guest_source_boundary() {
        const ADD: &[u8] = &[0,97,115,109,1,0,0,0,1,6,1,96,1,127,1,127,3,2,1,0,7,7,1,3,114,117,110,0,0,10,9,1,7,0,32,0,65,1,106,11];
        for mode in ["decoded"].into_iter().chain(cfg!(feature = "compact-executor").then_some("compact")) {
            for policy in [fuel_policy::SourceFuel::Unmetered, fuel_policy::SourceFuel::Metered] {
                let argument = fuel_policy::FuelArgument { policy, explicit: true };
                let engine = engine_with_fuel(mode, argument);
                let module = engine.compile(ADD).unwrap();
                let mut store = Store::new(&engine);
                let instance = Linker::new(&engine).instantiate(&mut store, &module).unwrap();
                let function = exported(&store, instance);
                store.request_materialization_roundtrip(function, 0).unwrap();
                assert_eq!(call(&mut store, function, 3), 4);
                let snapshot = store.execution_snapshot().unwrap();
                assert_eq!(snapshot.remaining_fuel, policy.budget().map(|n| n - 1));
                assert!(snapshot.pending_boundary_charged);
            }
        }
    }
}
fn typed_engine(mode: &str, profile: &str) -> Engine {
    let features = match profile {
        "mvp" => FeatureSet::mvp(),
        "stable-core" => FeatureSet::stable_core(),
        "all-phase3-core" => FeatureSet::all_phase3_core(),
        "mvp-saturating" => {
            let mut features = FeatureSet::mvp();
            features.saturating_float_to_int = true;
            features
        }
        "simd" | "simd-relaxed" => {
            let mut features = FeatureSet::mvp();
            features.simd = true;
            features.saturating_float_to_int = true;
            features.relaxed_simd = profile == "simd-relaxed";
            features
        }
        _ => panic!("unknown feature profile"),
    };
    configured_engine(
        Config {
            features,
            ..Config::default()
        },
        mode,
    )
}
fn parse_values(encoded: &str) -> Vec<Value> {
    if encoded == "-" {
        return vec![];
    }
    encoded
        .split(';')
        .map(|item| {
            let (ty, bits) = item.split_once(':').expect("typed value needs type:value");
            match ty {
                "i32" => Value::I32(bits.parse().unwrap()),
                "i64" => Value::I64(bits.parse().unwrap()),
                "f32_bits" => Value::F32(bits.parse().unwrap()),
                "f64_bits" => Value::F64(bits.parse().unwrap()),
                "v128_bits" => {
                    assert_eq!(bits.len(), 32);
                    let mut bytes = [0; 16];
                    for (i, b) in bytes.iter_mut().enumerate() {
                        *b = u8::from_str_radix(&bits[i * 2..i * 2 + 2], 16).unwrap();
                    }
                    Value::V128(wtime_types::V128(bytes))
                }
                "reference" if bits == "null" => {
                    Value::Ref(wtime_runtime::RefValue::Null(wtime_types::HeapType::None))
                }
                _ => panic!(
                    "unsupported typed input; nonnull references require a guest setup driver"
                ),
            }
        })
        .collect()
}
fn format_values(store: &Store, values: &[Value]) -> String {
    if values.is_empty() {
        return "-".into();
    }
    values
        .iter()
        .map(|value| match *value {
            Value::I32(n) => format!("i32:{n}"),
            Value::I64(n) => format!("i64:{n}"),
            Value::F32(n) => format!("f32_bits:{n}"),
            Value::F64(n) => format!("f64_bits:{n}"),
            Value::V128(v) => format!(
                "v128_bits:{}",
                v.0.iter().map(|b| format!("{b:02x}")).collect::<String>()
            ),
            Value::Ref(r) => {
                use wtime_runtime::RefValue;
                let category = match r {
                    RefValue::Null(_) => "null",
                    RefValue::Func(_) => "func",
                    RefValue::I31(_) => "i31",
                    RefValue::Exn(_) => "exception",
                    RefValue::Extern(_) | RefValue::ExternI31(_) | RefValue::ExternGc(_) => {
                        "extern"
                    }
                    RefValue::Gc(_) | RefValue::InternalExtern(_) => {
                        #[cfg(feature = "legacy-api")]
                        { panic!("P001 numeric adapter does not classify GC reference results") }
                        #[cfg(not(feature = "legacy-api"))]
                        {
                        if store
                            .reference_matches(*value, wtime_types::HeapType::Struct)
                            .unwrap()
                        {
                            "struct"
                        } else if store
                            .reference_matches(*value, wtime_types::HeapType::Array)
                            .unwrap()
                        {
                            "array"
                        } else if store
                            .reference_matches(*value, wtime_types::HeapType::Cont)
                            .unwrap()
                        {
                            "continuation"
                        } else {
                            panic!("unclassified reference")
                        }
                        }
                    }
                };
                format!("reference:{category}")
            }
        })
        .collect::<Vec<_>>()
        .join(";")
}
fn invoke_typed(store: &mut Store, func: wtime_runtime::Func, args: &[Value]) -> Vec<Value> {
    match store.invoke(func, black_box(args)).unwrap() {
        RunOutcome::Returned(values) => values,
        other => panic!("unexpected typed outcome {other:?}"),
    }
}
fn arguments(store: &Store, func: wtime_runtime::Func, spec: &str) -> Vec<Value> {
    let mut args = parse_values(spec);
    let wtime_runtime::ExternalType::Func(ty) = store.external_type(External::Func(func)).unwrap()
    else {
        unreachable!()
    };
    assert_eq!(args.len(), ty.params.len());
    for (value, ty) in args.iter_mut().zip(ty.params) {
        if let (Value::Ref(wtime_runtime::RefValue::Null(heap)), wtime_types::ValType::Ref(r)) =
            (value, ty)
        {
            *heap = r.heap;
        }
    }
    args
}
fn typed_memory_fields(store: &Store, instance: wtime_runtime::Instance) -> String {
    let external = instance
        .export(store, "memory")
        .expect("missing memory export");
    let External::Memory(memory) = external else {
        panic!("memory export is not memory")
    };
    let wtime_runtime::ExternalType::Memory(ty) = store.external_type(external).unwrap() else {
        unreachable!()
    };
    let length = ty
        .limits
        .min
        .checked_shl(ty.page_size_log2 as u32)
        .and_then(|n| usize::try_from(n).ok())
        .expect("memory snapshot length overflow");
    assert!(length <= 1 << 20, "memory snapshot exceeds harness bound");
    let bytes = store.read_memory(memory, 0, length).unwrap();
    let hex = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!(
        ",\"memory_pages\":{},\"memory_page_size_log2\":{},\"memory_hex\":{}",
        ty.limits.min,
        ty.page_size_log2,
        quote(&hex)
    )
}
fn typed_main(args: &[String]) -> bool {
    let command = args.first().map(String::as_str);
    if !matches!(
        command,
        Some("--typed-case" | "--typed-allowed-case" | "--simd-memory-case")
    ) {
        return false;
    }
    let allowed = command == Some("--typed-allowed-case");
    let memory = command == Some("--simd-memory-case");
    assert_eq!(
        args.len(),
        9,
        "--typed-case FILE CASE ARGS EXPECTED STAGE BATCH MODE PROFILE"
    );
    let bytes = std::fs::read(&args[1]).unwrap();
    let case = &args[2];
    let input = &args[3];
    let expected = &args[4];
    let accepts = |result: &str| {
        if allowed {
            expected.split('|').any(|candidate| candidate == result)
        } else {
            result == expected
        }
    };
    let stage = &args[5];
    let requested = args[6].parse::<usize>().unwrap();
    let max_batch = if memory { TYPED_MEMORY_MAX_BATCH } else { TYPED_NUMERIC_MAX_BATCH };
    assert!(requested <= max_batch, "typed batch exceeds driver capacity {max_batch}");
    let mode = &args[7];
    let profile = &args[8];
    assert!(
        [
            "prepare",
            "instantiate",
            "first_call",
            "warm_call",
            "load_and_call"
        ]
        .contains(&stage.as_str())
    );
    let e = typed_engine(mode, profile);
    let module = e.compile(&bytes).unwrap();
    let linker = Linker::new(&e);
    let mut store = Store::new(&e);
    let instance = linker.instantiate(&mut store, &module).unwrap();
    let func = exported(&store, instance);
    let argv = arguments(&store, func, input);
    let values = invoke_typed(&mut store, func, &argv);
    let check = format_values(&store, &values);
    assert!(accepts(&check), "typed preflight oracle mismatch: {check}");
    // Preparation has no measured invocation; its image is explicitly preflight.
    let mut memory_fields = if memory && stage == "prepare" {
        typed_memory_fields(&store, instance)
    } else {
        String::new()
    };
    // Fresh cold state; preflight state is never reused for the measured invocation.
    let mut store = Store::new(&e);
    let mut batch = 1;
    let (ns, result) = match stage.as_str() {
        "prepare" => {
            let e = typed_engine(mode, profile);
            let start = Instant::now();
            let m = e.compile(black_box(&bytes)).unwrap();
            let ns = start.elapsed().as_nanos() as f64;
            black_box(m);
            (ns, check.clone())
        }
        "instantiate" => {
            let start = Instant::now();
            let instance = linker.instantiate(&mut store, &module).unwrap();
            let ns = start.elapsed().as_nanos() as f64;
            let func = exported(&store, instance);
            let argv = arguments(&store, func, input);
            let values = invoke_typed(&mut store, func, &argv);
            if memory {
                memory_fields = typed_memory_fields(&store, instance);
            }
            (ns, format_values(&store, &values))
        }
        "load_and_call" => {
            let e = typed_engine(mode, profile);
            let start = Instant::now();
            let m = e.compile(&bytes).unwrap();
            let mut s = Store::new(&e);
            let inst = Linker::new(&e).instantiate(&mut s, &m).unwrap();
            let f = exported(&s, inst);
            let a = arguments(&s, f, input);
            let v = invoke_typed(&mut s, f, &a);
            let ns = start.elapsed().as_nanos() as f64;
            if memory {
                memory_fields = typed_memory_fields(&s, inst);
            }
            (ns, format_values(&s, &v))
        }
        "first_call" | "warm_call" => {
            let instance = linker.instantiate(&mut store, &module).unwrap();
            let func = exported(&store, instance);
            let argv = arguments(&store, func, input);
            if stage == "warm_call" {
                for _ in 0..5 {
                    let v = invoke_typed(&mut store, func, &argv);
                    assert!(
                        accepts(&format_values(&store, &v)),
                        "typed warmup oracle mismatch"
                    );
                }
                batch = if requested == 0 {
                    let start = Instant::now();
                    let v = invoke_typed(&mut store, func, &argv);
                    let ns = start.elapsed().as_nanos() as f64;
                    assert!(
                        accepts(&format_values(&store, &v)),
                        "typed calibration oracle mismatch"
                    );
                    (20000000.0 / ns.max(1.0)).ceil().clamp(1.0, max_batch as f64) as usize
                } else {
                    requested
                };
            }
            let start = Instant::now();
            let mut values = vec![];
            for _ in 0..batch {
                values = black_box(invoke_typed(&mut store, func, &argv));
            }
            let ns = start.elapsed().as_nanos() as f64 / batch as f64;
            if memory {
                memory_fields = typed_memory_fields(&store, instance);
            }
            (ns, format_values(&store, &values))
        }
        _ => unreachable!(),
    };
    assert!(accepts(&result), "typed oracle mismatch: {result}");
    println!(
        "{{\"schema_version\":2,\"case\":{},\"stage\":{},\"input\":{},\"expected\":{},\"result\":{},\"batch\":{},\"ns_per_operation\":{:.3},\"mode\":{},\"profile\":{}{}}}",
        quote(case),
        quote(stage),
        quote(input),
        quote(expected),
        quote(&result),
        batch,
        ns.max(1.0),
        quote(mode),
        quote(profile),
        memory_fields
    );
    true
}
#[cfg(all(not(feature = "legacy-api"), not(feature = "runtime-only")))]
fn capability(args: &[String]) -> bool {
    let command = args.first().map(String::as_str);
    if !matches!(command, Some("--capability" | "--capability-allowed")) {
        return false;
    }
    let allowed = command == Some("--capability-allowed");
    assert_eq!(
        args.len(),
        5,
        "--capability FILE ARGS EXPECTED REQUIRED_FEATURES"
    );
    let probe = (|| -> Result<(), String> {
        let required = args[4].split(',').collect::<Vec<_>>();
        if required.iter().any(|f| {
            [
                "threads",
                "exception_handling",
                "gc",
                "reference_types",
                "function_references",
                "stack_switching",
                "custom_descriptors",
                "compact_imports",
            ]
            .contains(f)
        }) {
            return Err("this pinned Wasmtime oracle build lacks the required proposal/collector feature; no timing comparison is available".into());
        }
        let has = |f| required.contains(&f);
        let mut config = wasmtime::Config::new();
        config
            .strategy(wasmtime::Strategy::Cranelift)
            .cranelift_opt_level(wasmtime::OptLevel::Speed);
        config
            .consume_fuel(false)
            .epoch_interruption(false)
            .wasm_gc(false);
        config.wasm_simd(has("simd") || has("relaxed_simd"));
        config.wasm_relaxed_simd(has("relaxed_simd"));
        config.wasm_tail_call(has("tail_call"));
        config.wasm_memory64(has("memory64"));
        config.wasm_multi_memory(has("multi_memory"));
        config.wasm_bulk_memory(has("bulk_memory"));
        config.wasm_multi_value(has("multi_value"));
        config.wasm_extended_const(has("extended_const"));
        config.wasm_custom_page_sizes(has("custom_page_sizes"));
        config.wasm_wide_arithmetic(has("wide_arithmetic"));
        let e = wasmtime::Engine::new(&config).map_err(|e| e.to_string())?;
        let bytes = std::fs::read(&args[1]).map_err(|e| e.to_string())?;
        let module = wasmtime::Module::new(&e, &bytes).map_err(|e| e.to_string())?;
        let mut s = wasmtime::Store::new(&e, ());
        let inst = wasmtime::Instance::new(&mut s, &module, &[]).map_err(|e| e.to_string())?;
        let func = inst.get_func(&mut s, "run").ok_or("missing run export")?;
        let argv = parse_values(&args[2])
            .into_iter()
            .map(|v| match v {
                Value::I32(n) => Ok(wasmtime::Val::I32(n)),
                Value::I64(n) => Ok(wasmtime::Val::I64(n)),
                Value::F32(n) => Ok(wasmtime::Val::F32(n)),
                Value::F64(n) => Ok(wasmtime::Val::F64(n)),
                Value::V128(v) => Ok(wasmtime::Val::V128(v.to_u128().into())),
                Value::Ref(_) => Err(
                    "reference input comparator setup requires an explicit guest driver".to_owned(),
                ),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = vec![wasmtime::Val::I32(0); func.ty(&s).results().len()];
        func.call(&mut s, &argv, &mut out)
            .map_err(|e| e.to_string())?;
        let result = if out.is_empty() {
            "-".into()
        } else {
            out.iter()
                .map(|v| match v {
                    wasmtime::Val::I32(n) => Ok(format!("i32:{n}")),
                    wasmtime::Val::I64(n) => Ok(format!("i64:{n}")),
                    wasmtime::Val::F32(n) => Ok(format!("f32_bits:{n}")),
                    wasmtime::Val::F64(n) => Ok(format!("f64_bits:{n}")),
                    wasmtime::Val::V128(v) => Ok(format!(
                        "v128_bits:{}",
                        v.as_u128()
                            .to_le_bytes()
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<String>()
                    )),
                    _ => Err(
                        "reference result comparator setup requires a semantic oracle".to_owned(),
                    ),
                })
                .collect::<Result<Vec<_>, _>>()?
                .join(";")
        };
        if !(if allowed {
            args[3].split('|').any(|expected| expected == result)
        } else {
            result == args[3]
        }) {
            return Err(format!("comparator oracle mismatch: {result}"));
        }
        Ok(())
    })();
    match probe {
        Ok(()) => println!(
            "{{\"schema_version\":2,\"comparator\":\"wasmtime-49.0.2\",\"status\":\"supported\",\"compile_execute_checked\":true}}"
        ),
        Err(reason) if reason.starts_with("comparator oracle mismatch") => {
            eprintln!("{reason}");
            std::process::exit(3);
        }
        Err(reason) => println!(
            "{{\"schema_version\":2,\"comparator\":\"wasmtime-49.0.2\",\"status\":\"unsupported\",\"reason\":{}}}",
            quote(&reason)
        ),
    }
    true
}

#[cfg(all(not(feature = "legacy-api"), feature = "performance-counters"))]
fn diagnostic(args: &[String]) -> bool {
    if args.first().map(String::as_str) != Some("--diagnostic") {
        return false;
    }
    assert_eq!(
        args.len(),
        6,
        "--diagnostic FILE INPUT EXPECTED MODE PROFILE"
    );
    let bytes = std::fs::read(&args[1]).unwrap();
    let input = args[2].parse::<i32>().unwrap();
    let expected = args[3].parse::<i32>().unwrap();
    let engine = typed_engine(&args[4], &args[5]);
    let module = engine.compile(&bytes).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let func = exported(&store, instance);
    store.reset_performance_counters();
    let start = Instant::now();
    let result = call(&mut store, func, input);
    let diagnostic_elapsed_ns = start.elapsed().as_nanos();
    assert_eq!(result, expected);
    println!(
        "{{\"schema_version\":1,\"diagnostic\":true,\"result\":{result},\"diagnostic_elapsed_ns\":{diagnostic_elapsed_ns},\"counters\":{}}}",
        store.performance_counters().to_json()
    );
    true
}

#[cfg(all(not(feature = "legacy-api"), feature = "execution-reference"))]
fn snapshot(args: &[String]) -> bool {
    if args.first().map(String::as_str) != Some("--snapshot") {
        return false;
    }
    assert_eq!(
        args.len(),
        8,
        "--snapshot FILE ARGS EXPECTED PC MODE PROFILE CASE"
    );
    let e = typed_engine(&args[5], &args[6]);
    assert_eq!(
        e.execution_policy(),
        wtime_runtime::ExecutionPolicy::Decoded,
        "snapshot hook requires decoded policy"
    );
    let m = e.compile(&std::fs::read(&args[1]).unwrap()).unwrap();
    let mut store = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut store, &m).unwrap();
    let f = exported(&store, i);
    let argv = arguments(&store, f, &args[2]);
    store
        .request_materialization_roundtrip(f, args[4].parse().unwrap())
        .unwrap();
    let out = invoke_typed(&mut store, f, &argv);
    assert_eq!(
        format_values(&store, &out),
        args[3],
        "snapshot oracle mismatch"
    );
    let view = store
        .execution_snapshot()
        .expect("requested source boundary did not execute");
    println!(
        "{{\"schema_version\":1,\"diagnostic\":true,\"case\":{},\"source_pc\":{},\"prepared_pc\":{},\"function_body\":{},\"remaining_fuel\":{},\"pending_boundary_charged\":{},\"materialization_roundtrips\":{},\"normalized_state\":{}}}",
        quote(&args[7]),
        view.cursor.source_pc,
        view.cursor.prepared_pc,
        view.cursor.function_body,
        view.remaining_fuel.map_or("null".into(), |n| n.to_string()),
        view.pending_boundary_charged,
        store.materialization_roundtrips(),
        quote(&format!("{view:?}"))
    );
    true
}
