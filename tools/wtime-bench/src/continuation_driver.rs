//! P012 continuation-driver draft for review; do not copy without integration.
//!
//! This module expects the surrounding protocol module to provide
//! `configured_engine(Config, mode)` and `quote(&str)`. All guest work uses the
//! public Store/Instance API. Accounting and collection checks are outside
//! capture/resume timers.

use std::{hint::black_box, time::Instant};
use wtime_runtime::{Config, Engine, ErrorKind, External, Linker, RunOutcome, Store, Value};
use wtime_types::{FeatureSet, ResourceLimits};

const MAX_MODULE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Copy)]
struct LimitsOverride {
    fuel: Option<u64>,
    continuation_bytes: Option<usize>,
}

fn engine(mode: &str, profile: &str, limits: LimitsOverride) -> Engine {
    let features = match profile {
        "mvp" => FeatureSet::mvp(),
        "stable-core" => FeatureSet::stable_core(),
        "all-phase3-core" => FeatureSet::all_phase3_core(),
        _ => panic!("unknown continuation feature profile"),
    };
    let mut resource_limits = ResourceLimits::default();
    resource_limits.fuel = limits.fuel;
    if let Some(limit) = limits.continuation_bytes {
        resource_limits.max_continuation_bytes = limit;
    }
    super::configured_engine(
        Config {
            features,
            limits: resource_limits,
        },
        mode,
    )
}

fn read_module(path: &str) -> Vec<u8> {
    assert!(path.len() <= 4096, "module path is too long");
    let metadata = std::fs::metadata(path).expect("cannot stat continuation module");
    assert!(
        metadata.len() <= MAX_MODULE_BYTES,
        "continuation module exceeds 4 MiB"
    );
    std::fs::read(path).expect("cannot read continuation module")
}

fn exported(instance: wtime_runtime::Instance, store: &Store, name: &str) -> External {
    instance
        .export(store, name)
        .unwrap_or_else(|_| panic!("missing continuation export {name}"))
}

fn exported_func(
    instance: wtime_runtime::Instance,
    store: &Store,
    name: &str,
) -> wtime_runtime::Func {
    match exported(instance, store, name) {
        External::Func(func) => func,
        other => panic!("continuation export {name} is not a function: {other:?}"),
    }
}

fn wire(value: Value) -> String {
    match value {
        Value::I32(n) => format!("i32:{n}"),
        Value::I64(n) => format!("i64:{n}"),
        Value::F32(bits) => format!("f32_bits:{bits:08x}"),
        Value::F64(bits) => format!("f64_bits:{bits:016x}"),
        Value::V128(bits) => format!("v128_bits:{:032x}", bits.to_u128()),
        Value::Ref(_) => "reference".to_owned(),
    }
}

fn wire_values(values: &[Value]) -> Vec<String> {
    values.iter().copied().map(wire).collect()
}

fn json_strings(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| super::quote(value))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn ns_since(start: Instant) -> f64 {
    start.elapsed().as_nanos().max(1) as f64
}

fn capture_resume(
    bytes: &[u8],
    case: &str,
    driver: &str,
    mode: &str,
    profile: &str,
    expected_payload: &[Value],
    expected_result: &[Value],
    logical_frames: usize,
    numeric_locals: usize,
) -> String {
    let engine = engine(
        mode,
        profile,
        LimitsOverride {
            fuel: None,
            continuation_bytes: None,
        },
    );
    let setup_start = Instant::now();
    let module = engine
        .compile(bytes)
        .expect("continuation module compile failed");
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .expect("continuation module instantiate failed");
    let run = exported_func(instance, &store, "run");
    let resume = exported_func(instance, &store, "resume");
    let setup_ns = ns_since(setup_start);

    let before_capture_bytes = store.continuation_bytes();
    let capture_start = Instant::now();
    let captured = store
        .invoke(run, black_box(&[]))
        .expect("guest capture failed");
    let capture_ns = ns_since(capture_start);
    let RunOutcome::Suspended {
        payload,
        continuation,
        ..
    } = captured
    else {
        panic!("run must suspend; got {captured:?}")
    };
    assert_eq!(
        payload, expected_payload,
        "suspension payload oracle mismatch"
    );
    let payload_wire = wire_values(&payload);
    let after_capture_bytes = store.continuation_bytes();
    assert!(
        after_capture_bytes > before_capture_bytes,
        "capture retained no continuation bytes"
    );

    // Transfer ownership from Store-published outcome roots to an explicit
    // root. The collection is an observation/control, not part of resume time.
    let root = store.root(continuation).expect("continuation root failed");
    assert_eq!(root.reference(), continuation);
    store.release_published(&payload);
    store.release_published(&[Value::Ref(wtime_runtime::RefValue::Gc(continuation))]);
    let after_root_transfer_bytes = store.continuation_bytes();
    assert_eq!(after_root_transfer_bytes, after_capture_bytes);
    let collect_with_root = store
        .collect()
        .expect("collection with rooted continuation failed");
    assert_eq!(
        collect_with_root.live,
        if driver == "root" { 2 } else { 1 },
        "saved continuation graph must remain live"
    );

    let resume_start = Instant::now();
    let resumed = store
        .invoke(
            resume,
            black_box(&[Value::Ref(wtime_runtime::RefValue::Gc(root.reference()))]),
        )
        .expect("guest resume failed");
    let resume_ns = ns_since(resume_start);
    let RunOutcome::Returned(results) = resumed else {
        panic!("resume must return; got {resumed:?}")
    };
    assert_eq!(results, expected_result, "resumed result oracle mismatch");
    let result_wire = wire_values(&results);
    store.release_published(&results);
    let after_resume_bytes = store.continuation_bytes();

    drop(root);
    let collect_after_drop = store.collect().expect("collection after root drop failed");
    let after_drop_and_collect_bytes = store.continuation_bytes();
    assert_eq!(
        after_drop_and_collect_bytes, 0,
        "consumed continuation storage was retained"
    );
    assert_eq!(collect_after_drop.live, 0);

    format!(
        "{{\"schema_version\":2,\"case\":{},\"driver\":{},\"mode\":{},\"profile\":{},\"instrumented\":false,\"setup_ns\":{setup_ns:.3},\"capture_ns\":{capture_ns:.3},\"resume_ns\":{resume_ns:.3},\"payload\":{},\"result\":{},\"logical_frames\":{logical_frames},\"bytes_per_captured_frame\":{},\"numeric_locals\":{numeric_locals},\"continuation_bytes\":{{\"before_capture\":{before_capture_bytes},\"after_capture\":{after_capture_bytes},\"after_root_transfer\":{after_root_transfer_bytes},\"after_resume\":{after_resume_bytes},\"after_drop_and_collect\":{after_drop_and_collect_bytes}}},\"collection_with_root\":{{\"live\":{},\"reclaimed\":{}}},\"collection_after_drop\":{{\"live\":{},\"reclaimed\":{}}}}}",
        super::quote(case),
        super::quote(driver),
        super::quote(mode),
        super::quote(profile),
        json_strings(&payload_wire),
        json_strings(&result_wire),
        after_capture_bytes as f64 / logical_frames as f64,
        collect_with_root.live,
        collect_with_root.reclaimed,
        collect_after_drop.live,
        collect_after_drop.reclaimed,
    )
}

fn run_depth(bytes: &[u8], case: &str, mode: &str, profile: &str) -> String {
    let (depth, expected) = match case {
        "continuation_depth_0" => (0, 1009),
        "continuation_depth_4" => (4, 1019),
        "continuation_depth_32" => (32, 1537),
        _ => panic!("depth driver only accepts depth_0, depth_4, or depth_32"),
    };
    let payload = [Value::I32(7)];
    let result = [Value::I32(expected)];
    capture_resume(
        bytes,
        case,
        "depth",
        mode,
        profile,
        &payload,
        &result,
        depth + 1,
        depth + 1,
    )
}

fn run_reference(bytes: &[u8], case: &str, mode: &str, profile: &str) -> String {
    assert_eq!(
        case, "continuation_captured_reference",
        "root driver requires captured-reference fixture"
    );
    capture_resume(
        bytes,
        case,
        "root",
        mode,
        profile,
        &[Value::I32(5)],
        &[Value::I32(86)],
        1,
        1,
    )
}

fn plain(store: &mut Store, func: wtime_runtime::Func, expected: &[Value]) -> Vec<Value> {
    let RunOutcome::Returned(values) = store.invoke(func, black_box(&[])).expect("stage failed")
    else {
        panic!("expected returned stage")
    };
    assert_eq!(values, expected);
    store.release_published(&values);
    values
}
fn run_bound(bytes: &[u8], case: &str, mode: &str, profile: &str) -> String {
    assert_eq!(case, "continuation_bound_nested");
    let engine = engine(
        mode,
        profile,
        LimitsOverride {
            fuel: None,
            continuation_bytes: None,
        },
    );
    let start = Instant::now();
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let bind = exported_func(instance, &store, "bind");
    let resume = exported_func(instance, &store, "resume_bound");
    let calls = exported_func(instance, &store, "calls");
    let clear = exported_func(instance, &store, "clear");
    let setup_ns = ns_since(start);
    let before = store.continuation_bytes();
    let start = Instant::now();
    plain(&mut store, bind, &[Value::I32(0)]);
    let bind_ns = ns_since(start);
    let after_bind = store.continuation_bytes();
    assert!(after_bind > 0);
    plain(&mut store, calls, &[Value::I32(0)]);
    let start = Instant::now();
    let result = plain(&mut store, resume, &[Value::I32(0), Value::I32(1122)]);
    let resume_ns = ns_since(start);
    plain(&mut store, calls, &[Value::I32(1)]);
    let after_run = store.continuation_bytes();
    plain(&mut store, clear, &[Value::I32(0)]);
    let collect = store.collect().unwrap();
    let after_collect = store.continuation_bytes();
    assert_eq!(after_collect, 0);
    assert_eq!(collect.live, 0);
    format!(
        "{{\"schema_version\":2,\"case\":{},\"driver\":\"bind\",\"mode\":{},\"profile\":{},\"instrumented\":false,\"setup_ns\":{setup_ns},\"bind_ns\":{bind_ns},\"resume_ns\":{resume_ns},\"bind_result\":\"i32:0\",\"result\":{},\"worker_calls\":1,\"continuation_bytes\":{{\"before\":{before},\"after_bind\":{after_bind},\"after_run\":{after_run},\"after_collect\":{after_collect}}},\"collection\":{{\"live\":{},\"reclaimed\":{}}}}}",
        super::quote(case),
        super::quote(mode),
        super::quote(profile),
        json_strings(&wire_values(&result)),
        collect.live,
        collect.reclaimed
    )
}
fn run_one_shot(bytes: &[u8], case: &str, mode: &str, profile: &str) -> String {
    assert_eq!(case, "continuation_one_shot");
    let engine = engine(
        mode,
        profile,
        LimitsOverride {
            fuel: None,
            continuation_bytes: None,
        },
    );
    let start = Instant::now();
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let capture = exported_func(instance, &store, "capture");
    let first = exported_func(instance, &store, "resume_once");
    let first_result = exported_func(instance, &store, "first_result");
    let reuse = exported_func(instance, &store, "resume_reuse");
    let clear = exported_func(instance, &store, "clear");
    let setup_ns = ns_since(start);
    let before = store.continuation_bytes();
    let start = Instant::now();
    plain(&mut store, capture, &[Value::I32(0)]);
    let capture_ns = ns_since(start);
    let after_capture = store.continuation_bytes();
    assert!(after_capture > 0);
    let start = Instant::now();
    plain(&mut store, first, &[Value::I32(1)]);
    let resume_once_ns = ns_since(start);
    plain(&mut store, first_result, &[Value::I32(1)]);
    let start = Instant::now();
    let error = store
        .invoke(reuse, &[])
        .expect_err("second resume must trap");
    let resume_reuse_ns = ns_since(start);
    assert_eq!(error.kind, ErrorKind::Trap);
    assert!(error.message.contains("continuation"));
    let after_trap = store.continuation_bytes();
    plain(&mut store, clear, &[Value::I32(0)]);
    let collect = store.collect().unwrap();
    let after_collect = store.continuation_bytes();
    assert_eq!(after_collect, 0);
    assert_eq!(collect.live, 0);
    format!(
        "{{\"schema_version\":2,\"case\":{},\"driver\":\"oneshot\",\"mode\":{},\"profile\":{},\"instrumented\":false,\"setup_ns\":{setup_ns},\"capture_ns\":{capture_ns},\"resume_once_ns\":{resume_once_ns},\"resume_reuse_ns\":{resume_reuse_ns},\"first_result\":\"i32:1\",\"result\":\"trap\",\"error_kind\":\"Trap\",\"error_message\":{},\"continuation_bytes\":{{\"before\":{before},\"after_capture\":{after_capture},\"after_trap\":{after_trap},\"after_collect\":{after_collect}}},\"collection\":{{\"live\":{},\"reclaimed\":{}}}}}",
        super::quote(case),
        super::quote(mode),
        super::quote(profile),
        super::quote(&error.message),
        collect.live,
        collect.reclaimed
    )
}

fn run_cross_module(
    provider_bytes: &[u8],
    consumer_bytes: &[u8],
    mode: &str,
    profile: &str,
) -> String {
    let engine = engine(
        mode,
        profile,
        LimitsOverride {
            fuel: None,
            continuation_bytes: None,
        },
    );
    let setup_start = Instant::now();
    let provider_module = engine
        .compile(provider_bytes)
        .expect("provider module compile failed");
    let consumer_module = engine
        .compile(consumer_bytes)
        .expect("consumer module compile failed");
    let mut store = Store::new(&engine);
    let provider = Linker::new(&engine)
        .instantiate(&mut store, &provider_module)
        .expect("provider instantiate failed");
    let tag = match exported(provider, &store, "yield") {
        External::Tag(tag) => tag,
        other => panic!("provider yield export is not a tag: {other:?}"),
    };
    let mut linker = Linker::new(&engine);
    linker
        .define("provider", "yield", External::Tag(tag))
        .expect("provider tag link failed");
    let consumer = linker
        .instantiate(&mut store, &consumer_module)
        .expect("consumer instantiate failed");
    let run = exported_func(consumer, &store, "run");
    let resume = exported_func(consumer, &store, "resume");
    let setup_ns = ns_since(setup_start);

    let before = store.continuation_bytes();
    let start = Instant::now();
    let captured = store
        .invoke(run, black_box(&[]))
        .expect("cross-module capture failed");
    let capture_ns = ns_since(start);
    let RunOutcome::Suspended {
        payload,
        continuation,
        ..
    } = captured
    else {
        panic!("consumer run must suspend: {captured:?}")
    };
    assert!(
        payload.is_empty(),
        "cross-module suspension payload must be empty"
    );
    let after_capture = store.continuation_bytes();
    assert!(after_capture > before);
    let root = store.root(continuation).expect("cross-module root failed");
    store.release_published(&[Value::Ref(wtime_runtime::RefValue::Gc(continuation))]);
    let after_transfer = store.continuation_bytes();
    assert_eq!(after_transfer, after_capture);
    let collection = store
        .collect()
        .expect("cross-module retained collection failed");
    assert!(collection.live > 0);

    let start = Instant::now();
    let resumed = store
        .invoke(
            resume,
            black_box(&[Value::Ref(wtime_runtime::RefValue::Gc(root.reference()))]),
        )
        .expect("cross-module guest resume failed");
    let resume_ns = ns_since(start);
    assert_eq!(
        resumed,
        RunOutcome::Returned(vec![Value::I32(77)]),
        "cross-module nominal type/result mismatch"
    );
    if let RunOutcome::Returned(ref values) = resumed {
        store.release_published(values);
    }
    let after_resume = store.continuation_bytes();
    drop(root);
    let cleanup = store
        .collect()
        .expect("cross-module cleanup collection failed");
    let after_cleanup = store.continuation_bytes();
    assert_eq!(after_cleanup, 0);
    let mut negative_store = Store::new(&engine);
    let negative_provider = Linker::new(&engine)
        .instantiate(&mut negative_store, &provider_module)
        .unwrap();
    let mut negative_linker = Linker::new(&engine);
    negative_linker
        .define(
            "provider",
            "yield",
            exported(negative_provider, &negative_store, "yield"),
        )
        .unwrap();
    let negative_consumer = negative_linker
        .instantiate(&mut negative_store, &consumer_module)
        .unwrap();
    let negative_run = exported_func(negative_consumer, &negative_store, "run");
    let host = negative_store
        .create_typed_host_func(
            negative_consumer,
            wtime_types::TypeIdx(2),
            move |caller, _| caller.invoke(negative_run, &[]),
        )
        .unwrap();
    let rejected = negative_store
        .invoke(host, &[])
        .expect_err("exact host result must reject incompatible suspension");
    assert_eq!(rejected.kind, ErrorKind::Host);
    negative_store.collect().unwrap();
    format!(
        "{{\"schema_version\":2,\"case\":\"continuation_cross_module_consumer\",\"driver\":\"cross_module\",\"exact_host_result_rejected\":true,\"exact_host_error_kind\":\"Host\",\"mode\":{},\"profile\":{},\"instrumented\":false,\"setup_ns\":{setup_ns:.3},\"capture_ns\":{capture_ns:.3},\"resume_ns\":{resume_ns:.3},\"payload\":[],\"result\":[\"i32:77\"],\"continuation_bytes\":{{\"before_capture\":{before},\"after_capture\":{after_capture},\"after_root_transfer\":{after_transfer},\"after_resume\":{after_resume},\"after_drop_and_collect\":{after_cleanup}}},\"collection_with_root\":{{\"live\":{},\"reclaimed\":{}}},\"collection_after_drop\":{{\"live\":{},\"reclaimed\":{}}}}}",
        super::quote(mode),
        super::quote(profile),
        collection.live,
        collection.reclaimed,
        cleanup.live,
        cleanup.reclaimed,
    )
}

fn run_probe(bytes: &[u8], case: &str, mode: &str, profile: &str, probe: &str) -> String {
    assert_eq!(case, "continuation_depth_0");
    let limits = match probe {
        "fuel" => LimitsOverride {
            fuel: Some(4),
            continuation_bytes: None,
        },
        "fuel-positive" => LimitsOverride {
            fuel: Some(5),
            continuation_bytes: None,
        },
        "budget" => LimitsOverride {
            fuel: None,
            continuation_bytes: Some(0),
        },
        _ => panic!("unknown probe"),
    };
    let engine = engine(mode, profile, limits);
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let run = exported_func(instance, &store, "run");
    let gc_before = store.gc_payload_bytes();
    let outcome = store.invoke(run, &[]);
    let (kind, message, payload, successful) = if probe == "fuel-positive" {
        let RunOutcome::Suspended {
            payload,
            continuation,
            ..
        } = outcome.expect("five fuel must capture")
        else {
            panic!("five fuel must suspend")
        };
        assert_eq!(payload, [Value::I32(7)]);
        let wire = json_strings(&wire_values(&payload));
        store.release_published(&payload);
        store.release_published(&[Value::Ref(wtime_runtime::RefValue::Gc(continuation))]);
        ("none".to_owned(), String::new(), wire, true)
    } else {
        let error = outcome.expect_err("probe must fail");
        assert_eq!(error.kind, ErrorKind::ResourceLimit);
        assert_eq!(
            error.message,
            if probe == "fuel" {
                "fuel exhausted"
            } else {
                "continuation byte budget"
            }
        );
        (
            "ResourceLimit".to_owned(),
            error.message,
            "[]".to_owned(),
            false,
        )
    };
    let bytes_after = store.continuation_bytes();
    let gc_after = store.gc_payload_bytes();
    if successful {
        assert!(bytes_after > 0);
    } else {
        assert_eq!(bytes_after, 0);
    }
    let collection = store.collect().unwrap();
    let bytes_cleanup = store.continuation_bytes();
    let gc_cleanup = store.gc_payload_bytes();
    assert_eq!((collection.live, bytes_cleanup, gc_cleanup), (0, 0, 0));
    format!(
        "{{\"schema_version\":2,\"case\":{},\"driver\":{},\"mode\":{},\"profile\":{},\"instrumented\":false,\"outcome\":{},\"payload\":{},\"fuel_budget\":{},\"error_kind\":{},\"error_message\":{},\"continuation_bytes\":{bytes_after},\"continuation_bytes_after_collect\":{bytes_cleanup},\"gc_payload_bytes_before\":{gc_before},\"gc_payload_bytes_after_capture\":{gc_after},\"gc_payload_bytes_after_collect\":{gc_cleanup}}}",
        super::quote(case),
        super::quote(&format!("probe_{probe}")),
        super::quote(mode),
        super::quote(profile),
        super::quote(if successful { "suspended" } else { "error" }),
        payload,
        limits.fuel.map(|f| f.to_string()).unwrap_or("null".into()),
        super::quote(&kind),
        super::quote(&message)
    )
}

/// CLI forms:
/// `--continuation-driver FILE CASE MODE PROFILE DRIVER`
/// `--continuation-cross-module PROVIDER CONSUMER MODE PROFILE`
/// `--continuation-probe FILE CASE MODE PROFILE fuel|budget`
pub fn try_main(args: &[String]) -> bool {
    if args
        .first()
        .is_some_and(|s| s.starts_with("--continuation-"))
    {
        assert!(
            !cfg!(any(
                feature = "performance-counters",
                feature = "execution-reference"
            )),
            "continuation timing requires counters disabled"
        );
    }
    match args.first().map(String::as_str) {
        Some("--continuation-driver") => {
            assert_eq!(
                args.len(),
                6,
                "--continuation-driver FILE CASE MODE PROFILE DRIVER"
            );
            let bytes = read_module(&args[1]);
            let row = match args[5].as_str() {
                "depth" => run_depth(&bytes, &args[2], &args[3], &args[4]),
                "root" => run_reference(&bytes, &args[2], &args[3], &args[4]),
                "bind" => run_bound(&bytes, &args[2], &args[3], &args[4]),
                "oneshot" => run_one_shot(&bytes, &args[2], &args[3], &args[4]),
                other => panic!("unsupported continuation driver {other}"),
            };
            println!("{row}");
            true
        }
        Some("--continuation-cross-module") => {
            assert_eq!(
                args.len(),
                5,
                "--continuation-cross-module PROVIDER CONSUMER MODE PROFILE"
            );
            let provider = read_module(&args[1]);
            let consumer = read_module(&args[2]);
            println!(
                "{}",
                run_cross_module(&provider, &consumer, &args[3], &args[4])
            );
            true
        }
        Some("--continuation-probe") => {
            assert_eq!(
                args.len(),
                6,
                "--continuation-probe FILE CASE MODE PROFILE fuel|budget"
            );
            let bytes = read_module(&args[1]);
            println!(
                "{}",
                run_probe(&bytes, &args[2], &args[3], &args[4], &args[5])
            );
            true
        }
        _ => false,
    }
}
