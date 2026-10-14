//! P011 public-API benchmark-driver draft. Staged under /tmp during P023 timing.
//!
//! Surrounding protocol module must provide `configured_engine(Config, mode)`
//! and JSON `quote(&str)`. This draft never compares raw GC/exception handles.

use std::{
    hint::black_box,
    sync::{Arc, Mutex},
    time::Instant,
};
use wtime_runtime::{
    Config, Engine, ErrorKind, ExceptionRef, External, Linker, RefValue, RunOutcome, Store, Value,
};
use wtime_types::{FeatureSet, FuncType, ResourceLimits, ValType};

const MAX_MODULE_BYTES: u64 = 4 * 1024 * 1024;
const WASMTIME_STATUS: &str = "unsupported";
const WASMTIME_REASON: &str = "pinned Wasmtime benchmark build lacks GC and exception support";

#[derive(Debug, Clone, Copy, Default)]
struct Observation {
    ns: f64,
    live: usize,
    reclaimed: usize,
    gc_payload_bytes: usize,
    continuation_bytes: usize,
}

fn engine(mode: &str, profile: &str) -> Engine {
    let features = match profile {
        "all-phase3-core" => FeatureSet::all_phase3_core(),
        _ => panic!("GC/exception driver requires all-phase3-core"),
    };
    super::configured_engine(
        Config {
            features,
            limits: ResourceLimits::default(),
        },
        mode,
    )
}

fn read_module(path: &str) -> Vec<u8> {
    assert!(path.len() <= 4096, "module path is too long");
    let metadata = std::fs::metadata(path).expect("cannot stat module");
    assert!(
        metadata.len() <= MAX_MODULE_BYTES,
        "module exceeds 4 MiB bound"
    );
    std::fs::read(path).expect("cannot read module")
}

fn export(instance: wtime_runtime::Instance, store: &Store, name: &str) -> External {
    instance
        .export(store, name)
        .unwrap_or_else(|_| panic!("missing export {name}"))
}

fn function(instance: wtime_runtime::Instance, store: &Store, name: &str) -> wtime_runtime::Func {
    match export(instance, store, name) {
        External::Func(f) => f,
        other => panic!("export {name} is not a function: {other:?}"),
    }
}

fn wire(value: Value) -> String {
    match value {
        Value::I32(n) => format!("i32:{n}"),
        Value::I64(n) => format!("i64:{n}"),
        Value::F32(n) => format!("f32_bits:{n:08x}"),
        Value::F64(n) => format!("f64_bits:{n:016x}"),
        Value::V128(v) => format!("v128_bits:{:032x}", v.to_u128()),
        Value::Ref(RefValue::Null(_)) => "null_reference".to_owned(),
        Value::Ref(RefValue::Func(_)) => "function_reference".to_owned(),
        Value::Ref(RefValue::Gc(_)) => "gc_reference".to_owned(),
        Value::Ref(RefValue::I31(_)) => "i31_reference".to_owned(),
        Value::Ref(RefValue::Extern(_) | RefValue::InternalExtern(_)) => {
            "extern_reference".to_owned()
        }
        Value::Ref(RefValue::ExternI31(_) | RefValue::ExternGc(_)) => "extern_reference".to_owned(),
        Value::Ref(RefValue::Exn(_)) => "exception_reference".to_owned(),
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
            .map(|v| super::quote(v))
            .collect::<Vec<_>>()
            .join(",")
    )
}
fn ns(start: Instant) -> f64 {
    start.elapsed().as_nanos().max(1) as f64
}
fn gc_count(case: &str, n: i32) -> usize {
    if case.starts_with("gc_packed_array_") {
        1
    } else if case.starts_with("gc_struct_graph_")
        || case.starts_with("gc_nominal_exact_casts_")
        || case.starts_with("gc_root_lifecycle_")
    {
        n as usize
    } else if case.starts_with("gc_typed_function_refs_") {
        0
    } else if case.starts_with("gc_descriptor_")
        || case.starts_with("gc_host_reentry_collect_")
        || case.starts_with("gc_continuation_root_")
    {
        3
    } else {
        0
    }
}
fn expected_i32(case: &str, n: i32) -> Option<i32> {
    let u = n as i64;
    let answer = if case.starts_with("gc_packed_array_") {
        -3 * u
    } else if case.starts_with("gc_struct_graph_") || case.starts_with("gc_root_lifecycle_") {
        u * (u - 1) / 2
    } else if case.starts_with("gc_nominal_exact_casts_") {
        2 * u
    } else if case.starts_with("gc_typed_function_refs_") {
        u * (u - 1) / 2 + u
    } else if case.starts_with("gc_descriptor_")
        || case.starts_with("gc_host_reentry_collect_")
        || case.starts_with("gc_continuation_root_")
    {
        u
    } else if case.starts_with("exception_") {
        u
    } else {
        return None;
    };
    Some(answer as i32)
}
fn validate_case(case: &str, n: i32) {
    assert!(
        (0..=0x7fff_ffff).contains(&n),
        "input must be nonnegative i32"
    );
    let known = [
        "gc_packed_array",
        "gc_struct_graph",
        "gc_nominal_exact_casts",
        "gc_typed_function_refs",
        "gc_descriptor_graph",
        "gc_descriptor_cross_module",
        "gc_root_lifecycle",
        "gc_host_reentry_collect",
        "gc_continuation_root",
        "exception_caught_throw",
        "exception_normal_path",
        "exception_cross_call_catch",
        "exception_host_throw_catch",
        "exception_throw_ref_unwind",
        "exception_uncaught_throw",
    ];
    assert!(
        known.iter().any(|family| case == format!("{family}_n{n}")),
        "unknown workload case or mismatched input"
    );
}

fn setup_host_imports(
    store: &mut Store,
    linker: &mut Linker,
    case: &str,
    input: i32,
    collector: Arc<Mutex<Option<Observation>>>,
    host_exception: Arc<Mutex<Option<ExceptionRef>>>,
) {
    if case.starts_with("gc_host_reentry_collect_") {
        let callback_observation = collector.clone();
        let collect = store
            .create_host_func_with_caller(
                FuncType::new(vec![], vec![ValType::I32]),
                move |caller, _| {
                    let start = Instant::now();
                    let stats = caller.collect()?;
                    let observation = Observation {
                        ns: ns(start),
                        live: stats.live,
                        reclaimed: stats.reclaimed,
                        gc_payload_bytes: caller.store().gc_payload_bytes(),
                        continuation_bytes: caller.store().continuation_bytes(),
                    };
                    *callback_observation.lock().map_err(|_| {
                        wtime_runtime::Error::new(ErrorKind::Host, "collector observation poisoned")
                    })? = Some(observation);
                    Ok(vec![Value::I32(9)])
                },
            )
            .expect("reentrant collect host function creation failed");
        linker
            .define("env", "collect", External::Func(collect))
            .expect("collect import definition failed");
    } else if case.starts_with("gc_continuation_root_") {
        let callback_observation = collector.clone();
        let collect = store
            .create_host_func_with_caller(FuncType::new(vec![], vec![]), move |caller, _| {
                let start = Instant::now();
                let stats = caller.collect()?;
                let observation = Observation {
                    ns: ns(start),
                    live: stats.live,
                    reclaimed: stats.reclaimed,
                    gc_payload_bytes: caller.store().gc_payload_bytes(),
                    continuation_bytes: caller.store().continuation_bytes(),
                };
                *callback_observation.lock().map_err(|_| {
                    wtime_runtime::Error::new(ErrorKind::Host, "collector observation poisoned")
                })? = Some(observation);
                Ok(vec![])
            })
            .expect("continuation collect host function creation failed");
        linker
            .define("env", "collect", External::Func(collect))
            .expect("collect import definition failed");
    } else if case.starts_with("exception_host_throw_catch_") {
        let tag = store
            .create_tag(FuncType::new(vec![ValType::I32], vec![]))
            .expect("host number tag creation failed");
        linker
            .define("env", "number", External::Tag(tag))
            .expect("host tag definition failed");
        let captured_exception = host_exception.clone();
        let raise = store
            .create_host_func_with_outcome(FuncType::default(), move |caller, _| {
                let ex = caller.store().create_exception(tag, &[Value::I32(input)])?;
                *captured_exception.lock().map_err(|_| {
                    wtime_runtime::Error::new(
                        ErrorKind::Host,
                        "host exception observation poisoned",
                    )
                })? = Some(ex);
                Ok(RunOutcome::Exception(ex))
            })
            .expect("host throw function creation failed");
        linker
            .define("env", "raise", External::Func(raise))
            .expect("host raise import definition failed");
    }
}

fn run_single(bytes: &[u8], case: &str, n: i32, mode: &str, profile: &str) -> String {
    validate_case(case, n);
    let engine = engine(mode, profile);
    let collector = Arc::new(Mutex::new(None::<Observation>));
    let host_exception = Arc::new(Mutex::new(None::<ExceptionRef>));
    let setup_start = Instant::now();
    let module = engine.compile(bytes).expect("module compile failed");
    let mut store = Store::new(&engine);
    let mut linker = Linker::new(&engine);
    setup_host_imports(
        &mut store,
        &mut linker,
        case,
        n,
        collector.clone(),
        host_exception.clone(),
    );
    let instance = linker
        .instantiate(&mut store, &module)
        .expect("module instantiate failed");
    let setup_ns = ns(setup_start);

    if case.starts_with("gc_root_lifecycle_") {
        // The dedicated lifecycle branch keeps the same fresh Store and instance.
        return root_lifecycle_in_store(&mut store, instance, n, case, mode, profile, setup_ns);
    }
    assert!(
        !case.starts_with("gc_descriptor_cross_module_"),
        "descriptor_cross_module requires the two-file CLI"
    );
    let run = function(instance, &store, "run");
    let before_payload_bytes = store.gc_payload_bytes();
    let before_continuation_bytes = store.continuation_bytes();
    let invoke_start = Instant::now();
    let outcome = store
        .invoke(run, black_box(&[Value::I32(n)]))
        .expect("guest workload invocation failed");
    let invoke_ns = ns(invoke_start);
    let after_invoke_payload_bytes = store.gc_payload_bytes();
    let mut fields = String::new();
    let mut exceptional_ref = None;
    match outcome {
        RunOutcome::Returned(values) => {
            let expected = expected_i32(case, n).expect("return workload lacks scalar oracle");
            assert_eq!(
                values,
                [Value::I32(expected)],
                "independent scalar oracle mismatch"
            );
            fields.push_str(&format!(
                "\"outcome\":\"returned\",\"result\":{},",
                super::quote(&format!("i32:{expected}"))
            ));
            store.release_published(&values);
        }
        RunOutcome::Exception(ex) => {
            assert!(
                case.starts_with("exception_uncaught_throw_"),
                "unexpected uncaught exception outcome"
            );
            let tag = match export(instance, &store, "number") {
                External::Tag(tag) => tag,
                other => panic!("number export is not a tag: {other:?}"),
            };
            assert_eq!(
                store.exception_tag(ex).expect("exception tag query failed"),
                tag
            );
            let payload = store
                .exception_payload(ex)
                .expect("exception payload query failed");
            assert_eq!(
                payload,
                [Value::I32(n)],
                "uncaught exception payload mismatch"
            );
            fields.push_str(&format!(
                "\"outcome\":\"exception\",\"tag\":\"number\",\"payload\":{},",
                json_strings(&wire_values(payload))
            ));
            exceptional_ref = Some(ex);
        }
        RunOutcome::Suspended { .. } => panic!("unexpected suspension in GC/exception workload"),
    }
    let host_observation = collector
        .lock()
        .expect("collector observation poisoned")
        .take();
    if case.starts_with("gc_host_reentry_collect_") {
        let host = host_observation.expect("guest did not reach reentrant collection callback");
        assert_eq!(
            (host.live, host.reclaimed),
            (1, 0),
            "temporary child root oracle mismatch"
        );
        assert!(host.gc_payload_bytes > 0);
        fields.push_str(&format!("\"host_collection\":{{\"ns\":{:.3},\"live\":{},\"reclaimed\":{},\"gc_payload_bytes\":{}}},", host.ns, host.live, host.reclaimed, host.gc_payload_bytes));
    } else if case.starts_with("gc_continuation_root_") {
        let host = host_observation.expect("guest did not collect while continuation suspended");
        assert!(
            host.live >= 3,
            "continuation root graph was not retained: {host:?}"
        );
        assert_eq!(
            host.reclaimed, 1,
            "consumed initial continuation must be reclaimed"
        );
        fields.push_str(&format!("\"host_collection\":{{\"ns\":{:.3},\"live\":{},\"reclaimed\":{},\"gc_payload_bytes\":{},\"continuation_bytes\":{}}},", host.ns, host.live, host.reclaimed, host.gc_payload_bytes, host.continuation_bytes));
    }
    if let Some(ex) = host_exception
        .lock()
        .expect("host exception observation poisoned")
        .take()
    {
        store.release_published(&[Value::Ref(RefValue::Exn(ex))]);
    }
    if let Some(ex) = exceptional_ref.take() {
        store.release_published(&[Value::Ref(RefValue::Exn(ex))]);
    }
    let continuation_before_collect = store.continuation_bytes();
    let collect_start = Instant::now();
    let stats = store.collect().expect("post-invocation collection failed");
    let collect_ns = ns(collect_start);
    let after_collect_payload_bytes = store.gc_payload_bytes();
    let after_collect_continuation_bytes = store.continuation_bytes();
    let expected_reclaimed = if case.starts_with("exception_") {
        None
    } else {
        Some(gc_count(case, n))
    };
    if let Some(expected) = expected_reclaimed {
        assert_eq!(
            stats.live, 0,
            "fixture references should not escape invocation"
        );
        assert_eq!(
            stats.reclaimed, expected,
            "GC allocation-count oracle mismatch"
        );
        assert_eq!(
            after_collect_payload_bytes, 0,
            "GC payload remained after collection"
        );
    }
    assert_eq!(
        after_collect_continuation_bytes, 0,
        "continuation state remained after collection"
    );
    if case.starts_with("gc_continuation_root_") {
        assert_eq!(
            stats.reclaimed, 3,
            "expected object, descriptor and consumed continuation reclamation"
        );
    }
    let gc_fields = if expected_reclaimed.is_some() || case.starts_with("exception_") {
        format!(
            "\"collection\":{{\"ns\":{collect_ns:.3},\"live\":{},\"reclaimed\":{}}},\"gc_payload_bytes\":{{\"before_invoke\":{before_payload_bytes},\"after_invoke\":{after_invoke_payload_bytes},\"after_collect\":{after_collect_payload_bytes}}},\"continuation_bytes\":{{\"before_invoke\":{before_continuation_bytes},\"before_collect\":{continuation_before_collect},\"after_collect\":{after_collect_continuation_bytes}}},",
            stats.live, stats.reclaimed
        )
    } else {
        String::new()
    };
    format!(
        "{{\"schema_version\":2,\"case\":{},\"input\":{},\"mode\":{},\"profile\":{},\"driver\":\"single\",\"instrumented\":false,\"wasmtime\":{{\"status\":{},\"reason\":{}}},\"setup_ns\":{setup_ns:.3},\"invoke_ns\":{invoke_ns:.3},{fields}{gc_fields}\"report_complete\":true}}",
        super::quote(case),
        super::quote(&format!("i32:{n}")),
        super::quote(mode),
        super::quote(profile),
        super::quote(WASMTIME_STATUS),
        super::quote(WASMTIME_REASON)
    )
}

fn root_lifecycle_in_store(
    store: &mut Store,
    instance: wtime_runtime::Instance,
    n: i32,
    case: &str,
    mode: &str,
    profile: &str,
    setup_ns: f64,
) -> String {
    let checksum = function(instance, store, "checksum");
    let make = function(instance, store, "make");
    let read = function(instance, store, "read");
    let before_bytes = store.gc_payload_bytes();
    let start = Instant::now();
    let result = store
        .invoke(checksum, black_box(&[Value::I32(n)]))
        .expect("root checksum failed");
    let checksum_ns = ns(start);
    assert_eq!(
        result,
        RunOutcome::Returned(vec![Value::I32((n as i64 * (n as i64 - 1) / 2) as i32)])
    );
    if let RunOutcome::Returned(values) = &result {
        store.release_published(values);
    }
    let start = Instant::now();
    let after_checksum = store.collect().expect("post-checksum collection failed");
    let checksum_collect_ns = ns(start);
    assert_eq!(
        (after_checksum.live, after_checksum.reclaimed),
        (0, n as usize)
    );
    let after_checksum_bytes = store.gc_payload_bytes();
    assert_eq!(after_checksum_bytes, 0);

    let start = Instant::now();
    let made = store
        .invoke(make, black_box(&[Value::I32(n)]))
        .expect("root make failed");
    let make_ns = ns(start);
    let RunOutcome::Returned(values) = made else {
        panic!("make must return a reference")
    };
    let [value @ Value::Ref(RefValue::Gc(_))] = values.as_slice() else {
        panic!("make result must be one GC reference")
    };
    let token = store
        .root_value(*value)
        .expect("host value root registration failed");
    store.release_published(&values);
    let start = Instant::now();
    let rooted = store.collect().expect("rooted collection failed");
    let rooted_collect_ns = ns(start);
    assert_eq!((rooted.live, rooted.reclaimed), (1, 0));
    let rooted_bytes = store.gc_payload_bytes();
    assert!(rooted_bytes > 0);
    let read_result = store
        .invoke(read, black_box(&[token.value()]))
        .expect("rooted object read failed");
    assert_eq!(read_result, RunOutcome::Returned(vec![Value::I32(n)]));
    if let RunOutcome::Returned(values) = &read_result {
        store.release_published(values);
    }
    drop(token);
    let start = Instant::now();
    let released = store.collect().expect("released root collection failed");
    let released_collect_ns = ns(start);
    assert_eq!((released.live, released.reclaimed), (0, 1));
    let after_release_bytes = store.gc_payload_bytes();
    assert_eq!(after_release_bytes, 0);
    format!(
        "{{\"schema_version\":2,\"case\":{},\"input\":{},\"mode\":{},\"profile\":{},\"driver\":\"root_lifecycle\",\"instrumented\":false,\"wasmtime\":{{\"status\":{},\"reason\":{}}},\"setup_ns\":{setup_ns:.3},\"checksum_ns\":{checksum_ns:.3},\"make_ns\":{make_ns:.3},\"collect_ns\":{{\"after_checksum\":{checksum_collect_ns:.3},\"with_root\":{rooted_collect_ns:.3},\"after_release\":{released_collect_ns:.3}}},\"result\":{},\"report_complete\":true,\"root_category\":\"host_value_root\",\"rooted_collection\":{{\"live\":{},\"reclaimed\":{}}},\"released_collection\":{{\"live\":{},\"reclaimed\":{}}},\"gc_payload_bytes\":{{\"before\":{before_bytes},\"after_checksum\":{after_checksum_bytes},\"after_rooted_collect\":{rooted_bytes},\"after_release\":{after_release_bytes}}}}}",
        super::quote(case),
        super::quote(&format!("i32:{n}")),
        super::quote(mode),
        super::quote(profile),
        super::quote(WASMTIME_STATUS),
        super::quote(WASMTIME_REASON),
        super::quote(&format!("i32:{}", (n as i64 * (n as i64 - 1) / 2) as i32)),
        rooted.live,
        rooted.reclaimed,
        released.live,
        released.reclaimed
    )
}

fn run_cross_module(
    provider_bytes: &[u8],
    consumer_bytes: &[u8],
    n: i32,
    mode: &str,
    profile: &str,
) -> String {
    let engine = engine(mode, profile);
    let setup_start = Instant::now();
    let provider_module = engine
        .compile(provider_bytes)
        .expect("provider compile failed");
    let consumer_module = engine
        .compile(consumer_bytes)
        .expect("consumer compile failed");
    let mut store = Store::new(&engine);
    let provider = Linker::new(&engine)
        .instantiate(&mut store, &provider_module)
        .expect("provider instantiate failed");
    let make = function(provider, &store, "make");
    let mut linker = Linker::new(&engine);
    linker
        .define("provider", "make", External::Func(make))
        .expect("provider.make link failed");
    let consumer = linker
        .instantiate(&mut store, &consumer_module)
        .expect("consumer instantiate failed");
    let run = function(consumer, &store, "run");
    let setup_ns = ns(setup_start);
    let start = Instant::now();
    let result = store
        .invoke(run, black_box(&[Value::I32(n)]))
        .expect("consumer run failed");
    let invoke_ns = ns(start);
    assert_eq!(
        result,
        RunOutcome::Returned(vec![Value::I32(n)]),
        "cross-module descriptor oracle mismatch"
    );
    if let RunOutcome::Returned(values) = &result {
        store.release_published(values);
    }
    let bytes_before = store.gc_payload_bytes();
    let start = Instant::now();
    let stats = store.collect().expect("cross-module collection failed");
    let collect_ns = ns(start);
    let bytes_after = store.gc_payload_bytes();
    assert_eq!((stats.live, stats.reclaimed), (0, 3));
    assert_eq!(bytes_after, 0);
    format!(
        "{{\"schema_version\":2,\"case\":\"gc_descriptor_cross_module\",\"input\":{},\"mode\":{},\"profile\":{},\"driver\":\"cross_module\",\"instrumented\":false,\"wasmtime\":{{\"status\":{},\"reason\":{}}},\"setup_ns\":{setup_ns:.3},\"invoke_ns\":{invoke_ns:.3},\"result\":{},\"collection\":{{\"ns\":{collect_ns:.3},\"live\":{},\"reclaimed\":{}}},\"gc_payload_bytes\":{{\"before_collect\":{bytes_before},\"after_collect\":{bytes_after}}},\"report_complete\":true,\"normalized_provider_object\":\"descriptor_child_graph\"}}",
        super::quote(&format!("i32:{n}")),
        super::quote(mode),
        super::quote(profile),
        super::quote(WASMTIME_STATUS),
        super::quote(WASMTIME_REASON),
        super::quote(&format!("i32:{n}")),
        stats.live,
        stats.reclaimed
    )
}

/// `--gc-exception-driver FILE CASE INPUT MODE PROFILE`
/// `--gc-exception-cross-module PROVIDER CONSUMER INPUT MODE PROFILE`
pub fn try_main(args: &[String]) -> bool {
    if matches!(
        args.first().map(String::as_str),
        Some("--gc-exception-driver" | "--gc-exception-cross-module")
    ) {
        assert!(
            !cfg!(any(
                feature = "performance-counters",
                feature = "execution-reference"
            )),
            "GC timing requires counters disabled"
        );
    }
    match args.first().map(String::as_str) {
        Some("--gc-exception-driver") => {
            assert_eq!(
                args.len(),
                6,
                "--gc-exception-driver FILE CASE INPUT MODE PROFILE"
            );
            let n: i32 = args[3]
                .strip_prefix("i32:")
                .expect("input must be typed i32:N")
                .parse()
                .expect("invalid i32 input");
            let bytes = read_module(&args[1]);
            println!("{}", run_single(&bytes, &args[2], n, &args[4], &args[5]));
            true
        }
        Some("--gc-exception-cross-module") => {
            assert_eq!(
                args.len(),
                6,
                "--gc-exception-cross-module PROVIDER CONSUMER INPUT MODE PROFILE"
            );
            let n: i32 = args[3]
                .strip_prefix("i32:")
                .expect("input must be typed i32:N")
                .parse()
                .expect("invalid i32 input");
            let provider = read_module(&args[1]);
            let consumer = read_module(&args[2]);
            println!(
                "{}",
                run_cross_module(&provider, &consumer, n, &args[4], &args[5])
            );
            true
        }
        _ => false,
    }
}
