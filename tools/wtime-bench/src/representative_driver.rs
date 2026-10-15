//! Import-free P015 runner. Include from protocol.rs under !legacy-api.
//!
//! CLI: --representative-case FILE CASE SIZE EXPECTED OUTPUT_SHA256 STAGE MODE PROFILE
//! The one measured stage is `run_after_setup`: compilation, instantiation, and
//! the exported `prepare(size)` reset are outside the timer. Full memory
//! readback/hex formatting is also outside the timer.

use std::{hint::black_box, time::Instant};
use wtime_runtime::{External, Linker, RunOutcome, Store, Value};

const MAX_MEMORY_BYTES: usize = 1 << 20;
const MAX_BATCH_WORK_UNITS: usize = 50_000_000;
const HEX: &[u8; 16] = b"0123456789abcdef";

struct Observation {
    result: i32,
    elapsed_ns: f64,
    pages: u64,
    page_size_log2: u8,
    memory_hex: String,
}

fn export_func(
    store: &Store,
    instance: wtime_runtime::Instance,
    name: &str,
) -> wtime_runtime::Func {
    match instance
        .export(store, name)
        .unwrap_or_else(|_| panic!("missing {name} export"))
    {
        External::Func(func) => func,
        other => panic!("{name} export has unexpected kind: {other:?}"),
    }
}

fn invoke_i32(store: &mut Store, func: wtime_runtime::Func, size: i32) -> i32 {
    match store.invoke(func, black_box(&[Value::I32(size)])).unwrap() {
        RunOutcome::Returned(values) => match values.as_slice() {
            [Value::I32(value)] => *value,
            other => panic!("unexpected i32 call result {other:?}"),
        },
        other => panic!("unexpected representative outcome {other:?}"),
    }
}

fn invoke_prepare(store: &mut Store, func: wtime_runtime::Func, size: i32) {
    match store.invoke(func, black_box(&[Value::I32(size)])).unwrap() {
        RunOutcome::Returned(values) if values.is_empty() => {}
        other => panic!("prepare(size) must return no values, got {other:?}"),
    }
}

fn snapshot(store: &Store, instance: wtime_runtime::Instance) -> (u64, u8, String) {
    let memory = match instance
        .export(store, "memory")
        .expect("missing memory export")
    {
        External::Memory(memory) => memory,
        other => panic!("memory export has unexpected kind: {other:?}"),
    };
    let wtime_runtime::ExternalType::Memory(ty) = store
        .external_type(External::Memory(memory))
        .expect("memory type query failed")
    else {
        unreachable!()
    };
    let page_bytes = 1u64
        .checked_shl(ty.page_size_log2 as u32)
        .expect("memory page size overflow");
    let byte_count = ty
        .limits
        .min
        .checked_mul(page_bytes)
        .expect("memory byte count overflow");
    let byte_count = usize::try_from(byte_count).expect("memory does not fit host usize");
    assert!(
        byte_count <= MAX_MEMORY_BYTES,
        "P015 snapshot exceeds one-MiB bound"
    );
    let bytes = store
        .read_memory(memory, 0, byte_count)
        .expect("memory readback failed");
    assert_eq!(bytes.len(), byte_count);
    let mut memory_hex = String::new();
    memory_hex
        .try_reserve_exact(byte_count.checked_mul(2).expect("hex length overflow"))
        .expect("bounded hex allocation failed");
    for byte in bytes {
        memory_hex.push(HEX[(byte >> 4) as usize] as char);
        memory_hex.push(HEX[(byte & 0x0f) as usize] as char);
    }
    (ty.limits.min, ty.page_size_log2, memory_hex)
}

#[cfg(not(feature = "runtime-only"))]
fn execute_wasmtime(bytes: &[u8], size: i32) -> Observation {
    let mut config = wasmtime::Config::new();
    config
        .strategy(wasmtime::Strategy::Cranelift)
        .cranelift_opt_level(wasmtime::OptLevel::Speed)
        .consume_fuel(false)
        .epoch_interruption(false)
        .wasm_gc(false);
    let engine = wasmtime::Engine::new(&config).expect("Wasmtime configuration failed");
    let module =
        wasmtime::Module::new(&engine, bytes).expect("Wasmtime representative compile failed");
    let mut store = wasmtime::Store::new(&engine, ());
    let instance = wasmtime::Instance::new(&mut store, &module, &[])
        .expect("Wasmtime representative instantiate failed");
    let prepare = instance
        .get_typed_func::<i32, ()>(&mut store, "prepare")
        .unwrap();
    let run = instance
        .get_typed_func::<i32, i32>(&mut store, "run")
        .unwrap();
    prepare
        .call(&mut store, size)
        .expect("Wasmtime prepare failed");
    let start = Instant::now();
    let result = run
        .call(&mut store, black_box(size))
        .expect("Wasmtime run failed");
    let elapsed_ns = start.elapsed().as_nanos().max(1) as f64;
    let memory = instance
        .get_memory(&mut store, "memory")
        .expect("Wasmtime memory export missing");
    let bytes = memory.data(&store);
    assert_eq!(
        bytes.len(),
        65536,
        "representative memory must be one fixed page"
    );
    let mut memory_hex = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        memory_hex.push(HEX[(byte >> 4) as usize] as char);
        memory_hex.push(HEX[(byte & 15) as usize] as char);
    }
    Observation {
        result,
        elapsed_ns,
        pages: 1,
        page_size_log2: 16,
        memory_hex,
    }
}

fn execute_once(bytes: &[u8], size: i32, mode: &str, profile: &str) -> Observation {
    let engine = super::typed_engine(mode, profile);
    let module = engine
        .compile(bytes)
        .expect("representative module compile failed");
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .expect("representative module instantiation failed");
    let prepare = export_func(&store, instance, "prepare");
    let run = export_func(&store, instance, "run");

    // Every process starts from a fresh Store/Instance, and prepare is outside
    // the timer so the measured stage covers only the representative kernel.
    invoke_prepare(&mut store, prepare, size);
    let start = Instant::now();
    let result = invoke_i32(&mut store, run, size);
    let elapsed_ns = start.elapsed().as_nanos().max(1) as f64;

    // Public API readback and hex formatting are excluded from guest timing.
    let (pages, page_size_log2, memory_hex) = snapshot(&store, instance);
    Observation {
        result,
        elapsed_ns,
        pages,
        page_size_log2,
        memory_hex,
    }
}

/// Measure a setup-aware batch.  Each call gets a fresh Store and Instance,
/// matching the frozen fresh_instance_per_invocation reset policy.  The module
/// is compiled once per process; instance construction and prepare are outside
/// each call's Instant interval.  The complete memory snapshot happens after
/// the final call, outside every interval.
fn execute_staged(
    bytes: &[u8],
    size: i32,
    expected: i32,
    mode: &str,
    profile: &str,
    batch: usize,
    fuel: super::fuel_policy::FuelArgument,
) -> (Observation, usize, f64) {
    let engine = super::typed_engine_with_fuel(mode, profile, fuel);
    let module = engine
        .compile(bytes)
        .expect("representative module compile failed");
    let mut guest_ns = 0.0f64;
    let mut calls_checked = 0usize;
    let mut result = expected;
    let mut final_snapshot = None;
    for _ in 0..batch {
        let mut store = Store::new(&engine);
        let instance = Linker::new(&engine)
            .instantiate(&mut store, &module)
            .expect("representative module instantiation failed");
        let prepare = export_func(&store, instance, "prepare");
        let run = export_func(&store, instance, "run");
        // Instance setup and deterministic reset are deliberately outside timing.
        invoke_prepare(&mut store, prepare, size);
        let start = Instant::now();
        let current = invoke_i32(&mut store, run, size);
        guest_ns += start.elapsed().as_nanos().max(1) as f64;
        assert_eq!(current, expected, "representative result oracle mismatch");
        calls_checked += 1;
        result = current;
        if calls_checked == batch {
            final_snapshot = Some(snapshot(&store, instance));
        }
    }

    // Readback, memory digest validation, and serialization are host work.
    let (pages, page_size_log2, memory_hex) = final_snapshot.expect("batch has at least one call");
    (
        Observation {
            result,
            elapsed_ns: guest_ns / batch as f64,
            pages,
            page_size_log2,
            memory_hex,
        },
        calls_checked,
        guest_ns,
    )
}

#[cfg(all(feature = "performance-counters", not(feature = "execution-reference")))]
fn execute_staged_diagnostic(
    bytes: &[u8],
    size: i32,
    expected: i32,
    mode: &str,
    profile: &str,
) -> (Observation, String) {
    let engine = super::typed_engine(mode, profile);
    let module = engine
        .compile(bytes)
        .expect("representative module compile failed");
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .expect("representative module instantiation failed");
    let prepare = export_func(&store, instance, "prepare");
    let run = export_func(&store, instance, "run");
    invoke_prepare(&mut store, prepare, size);
    store.reset_performance_counters();
    let start = Instant::now();
    let result = invoke_i32(&mut store, run, size);
    let measured_guest_ns = start.elapsed().as_nanos().max(1) as f64;
    assert_eq!(
        result, expected,
        "representative diagnostic oracle mismatch"
    );
    let counters_json = store.performance_counters().to_json();
    let (pages, page_size_log2, memory_hex) = snapshot(&store, instance);
    (
        Observation {
            result,
            elapsed_ns: measured_guest_ns,
            pages,
            page_size_log2,
            memory_hex,
        },
        counters_json,
    )
}

pub fn try_main(args: &[String]) -> bool {
    let wasmtime = args.first().map(String::as_str) == Some("--representative-wasmtime-case");
    let staged = args.first().map(String::as_str) == Some("--representative-staged");
    let staged_diagnostic =
        args.first().map(String::as_str) == Some("--representative-staged-diagnostic");
    if !wasmtime
        && !staged
        && !staged_diagnostic
        && args.first().map(String::as_str) != Some("--representative-case")
    {
        return false;
    }
    if staged_diagnostic {
        assert!(
            !cfg!(feature = "execution-reference"),
            "representative diagnostics require decoded execution"
        );
        assert!(
            cfg!(feature = "performance-counters"),
            "representative diagnostics require performance-counters"
        );
    } else {
        assert!(
            !cfg!(feature = "performance-counters") && !cfg!(feature = "execution-reference"),
            "representative timing requires a noninstrumented default build"
        );
    }
    let expected_arg_count = if staged || staged_diagnostic { 10 } else { 9 };
    let fuel = if staged {
        let fuel = super::fuel_policy::FuelArgument::parse(args, expected_arg_count)
            .expect("invalid representative timing options");
        fuel.validate_mode(&args[8])
            .expect("invalid register entry setting");
        assert!(
            matches!(args[8].as_str(), "decoded" | "compact")
                || (args[8] == "register" && cfg!(feature = "register-executor")),
            "unsupported representative timing mode"
        );
        fuel
    } else {
        assert_eq!(
            args.len(),
            expected_arg_count,
            "invalid representative argument count"
        );
        super::fuel_policy::FuelArgument::OMITTED
    };
    let bytes = std::fs::read(&args[1]).expect("cannot read representative fixture");
    assert!(
        bytes.len() <= MAX_MEMORY_BYTES,
        "representative fixture exceeds one MiB"
    );
    let case = &args[2];
    let size: i32 = args[3].parse().expect("invalid i32 workload size");
    let allowed = if case.starts_with("representative_parser_hash_") {
        &[16, 1024, 257][..]
    } else if case.starts_with("representative_image_filter_") {
        &[16, 256, 73][..]
    } else if case.starts_with("representative_graph_bfs_") {
        &[12, 64, 23][..]
    } else {
        panic!("unknown representative case")
    };
    assert!(
        allowed.contains(&size),
        "representative input is not a frozen size"
    );
    let expected: i32 = args[4]
        .strip_prefix("i32:")
        .expect("expected typed i32")
        .parse()
        .expect("invalid expected i32 result");
    let output_sha256 = &args[5];
    assert_eq!(
        output_sha256.len(),
        64,
        "expected output SHA-256 must be 64 hex chars"
    );
    assert!(output_sha256.bytes().all(|b| b.is_ascii_hexdigit()));
    let stage = &args[6];
    assert_eq!(stage, "run_after_setup", "unsupported representative stage");
    let batch = if staged || staged_diagnostic {
        let value = args[7]
            .parse::<usize>()
            .expect("invalid representative batch");
        assert!(
            (1..=100_000).contains(&value),
            "representative batch is out of bounds"
        );
        let work = usize::try_from(size)
            .expect("representative size must be positive")
            .checked_mul(value)
            .expect("representative batch work overflow");
        assert!(
            work <= MAX_BATCH_WORK_UNITS,
            "representative batch exceeds the frozen work bound"
        );
        value
    } else {
        1
    };
    let mode_index = if staged || staged_diagnostic { 8 } else { 7 };
    let mode = &args[mode_index];
    let profile = &args[mode_index + 1];

    assert_eq!(profile, "mvp", "representative profile must be MVP");
    if staged_diagnostic {
        assert_eq!(batch, 1, "representative counter diagnostics use one call");
        assert!(
            matches!(mode.as_str(), "decoded" | "compact"),
            "staged representative diagnostics support decoded or compact execution"
        );
        #[cfg(all(feature = "performance-counters", not(feature = "execution-reference")))]
        {
            let (actual, counters_json) =
                execute_staged_diagnostic(&bytes, size, expected, mode, profile);
            println!(
                "{{\"schema_version\":2,\"diagnostic\":true,\"instrumented\":true,\"case\":{},\"stage\":{},\"input\":{},\"expected\":{},\"result\":{},\"batch\":1,\"calls_checked\":1,\"report_complete\":true,\"setup_reset_before_each_call\":true,\"setup_excluded_from_timing\":true,\"measured_guest_ns\":{:.3},\"diagnostic_elapsed_ns\":{:.3},\"mode\":{},\"profile\":{},\"memory_pages\":{},\"memory_page_size_log2\":{},\"memory_hex\":{},\"expected_output_sha256\":{},\"counters\":{}}}",
                super::quote(case),
                super::quote(stage),
                super::quote(&size.to_string()),
                super::quote(&format!("i32:{expected}")),
                super::quote(&format!("i32:{}", actual.result)),
                actual.elapsed_ns,
                actual.elapsed_ns,
                super::quote(mode),
                super::quote(profile),
                actual.pages,
                actual.page_size_log2,
                super::quote(&actual.memory_hex),
                super::quote(output_sha256),
                counters_json,
            );
            return true;
        }
        #[cfg(any(not(feature = "performance-counters"), feature = "execution-reference"))]
        panic!("representative diagnostics require decoded performance-counters build");
    }
    let (actual, calls_checked, measured_guest_ns) = if staged {
        assert!(
            matches!(mode.as_str(), "decoded" | "compact" | "register"),
            "staged representative timing supports decoded, compact or register execution"
        );
        execute_staged(&bytes, size, expected, mode, profile, batch, fuel)
    } else if wasmtime {
        assert_eq!(mode, "wasmtime");
        #[cfg(not(feature = "runtime-only"))]
        {
            let actual = execute_wasmtime(&bytes, size);
            let elapsed_ns = actual.elapsed_ns;
            (actual, 1, elapsed_ns)
        }
        #[cfg(feature = "runtime-only")]
        {
            panic!("Wasmtime comparator is absent from runtime-only build")
        }
    } else {
        assert_eq!(mode, "decoded");
        let actual = execute_once(&bytes, size, mode, profile);
        let elapsed_ns = actual.elapsed_ns;
        (actual, 1, elapsed_ns)
    };
    assert_eq!(
        actual.result, expected,
        "representative result oracle mismatch"
    );
    if staged {
        println!(
            "{{\"schema_version\":2,\"case\":{},\"stage\":{},\"input\":{},\"expected\":{},\"result\":{},\"batch\":{},\"calls_checked\":{},\"report_complete\":true,\"setup_reset_before_each_call\":true,\"setup_excluded_from_timing\":true,\"instrumented\":false,\"measured_guest_ns\":{:.3},\"ns_per_operation\":{:.3},\"mode\":{},\"profile\":{},\"memory_pages\":{},\"memory_page_size_log2\":{},\"memory_hex\":{},\"expected_output_sha256\":{}{}}}",
            super::quote(case),
            super::quote(stage),
            super::quote(&size.to_string()),
            super::quote(&format!("i32:{expected}")),
            super::quote(&format!("i32:{}", actual.result)),
            batch,
            calls_checked,
            measured_guest_ns,
            actual.elapsed_ns,
            super::quote(mode),
            super::quote(profile),
            actual.pages,
            actual.page_size_log2,
            super::quote(&actual.memory_hex),
            super::quote(output_sha256),
            fuel.json_suffix(),
        );
    } else {
        println!(
            "{{\"schema_version\":2,\"case\":{},\"stage\":{},\"input\":{},\"expected\":{},\"result\":{},\"batch\":1,\"ns_per_operation\":{:.3},\"mode\":{},\"profile\":{},\"memory_pages\":{},\"memory_page_size_log2\":{},\"memory_hex\":{},\"expected_output_sha256\":{}}}",
            super::quote(case),
            super::quote(stage),
            super::quote(&size.to_string()),
            super::quote(&format!("i32:{expected}")),
            super::quote(&format!("i32:{}", actual.result)),
            actual.elapsed_ns,
            super::quote(mode),
            super::quote(profile),
            actual.pages,
            actual.page_size_log2,
            super::quote(&actual.memory_hex),
            super::quote(output_sha256),
        );
    }
    true
}
