//! Guest intervals and independent post-call state for P113/P044.
//! Lifecycle checking, collection, memory reset and readback are outside timers.
use std::{hint::black_box, time::Instant};
use wtime_runtime::{Config, ErrorKind, External, Instance, Linker, Module, RefValue, RunOutcome, Store, Value};
use wtime_types::FeatureSet;

const CASES: &[&str] = &["integration_gc_prefix", "integration_eh_prefix", "integration_cont_prefix",
    "integration_numeric_control", "integration_ref_entry_control", "integration_cont_entry_control"];
const TARGET_NS: f64 = 20_000_000.0;
const MAX_BATCH: usize = 100_000;

struct Snapshot {
    result: String,
    outcome: String,
    payload: String,
    lifecycle: String,
    memory: Vec<u8>,
    captured_memory: Option<Vec<u8>>,
}
fn function(instance: Instance, store: &Store, name: &str) -> wtime_runtime::Func {
    let External::Func(value) = instance.export(store, name).unwrap() else { panic!("not a function"); };
    value
}
fn memory(instance: Instance, store: &Store) -> wtime_runtime::Memory {
    let External::Memory(value) = instance.export(store, "memory").unwrap() else { panic!("not a memory"); };
    value
}
fn returned(value: i32) -> String {
    format!("{{\"kind\":\"returned\",\"values\":[\"i32:{value}\"]}}")
}
fn scalar(outcome: RunOutcome) -> i32 {
    let RunOutcome::Returned(values) = outcome else { panic!("expected return"); };
    let [Value::I32(value)] = values.as_slice() else { panic!("expected i32"); };
    *value
}
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        text.push(DIGITS[(byte >> 4) as usize] as char);
        text.push(DIGITS[(byte & 15) as usize] as char);
    }
    text
}
fn finish(store: &mut Store, instance: Instance, case: &str, n: i32, outcome: RunOutcome) -> Snapshot {
    let mem = memory(instance, store);
    let mut captured_memory = None;
    let (result, outcome_json, payload, lifecycle) = match outcome {
        RunOutcome::Returned(values) => {
            let [Value::I32(value)] = values.as_slice() else { panic!("bad result shape"); };
            let expected = if case == "integration_ref_entry_control" { 1 } else { n.wrapping_add(32) };
            assert_eq!(*value, expected, "independent scalar oracle");
            let value_json = returned(*value);
            let stats = store.collect().unwrap();
            let life = if case == "integration_gc_prefix" {
                format!("{{\"gc_allocation_count\":{},\"post_release_collection\":{{\"live\":{},\"reclaimed\":{}}}}}",
                    stats.live + stats.reclaimed, stats.live, stats.reclaimed)
            } else { "{}".to_owned() };
            (value_json.clone(), value_json, "{\"values\":[]}".to_owned(), life)
        }
        RunOutcome::Suspended { payload, continuation, .. } => {
            assert!(matches!(case, "integration_cont_prefix" | "integration_cont_entry_control"));
            if case == "integration_cont_prefix" { assert_eq!(payload, vec![Value::I32(n.wrapping_add(32))]); }
            else { assert!(payload.is_empty()); }
            let capture_bytes = store.continuation_bytes();
            captured_memory = Some(store.read_memory(mem, 0, 65_536).unwrap());
            let root = store.root(continuation).unwrap();
            let handle = Value::Ref(RefValue::Gc(root.reference()));
            store.release_published(&[handle]);
            store.release_published(&payload);
            assert!(store.collect().unwrap().live > 0, "capture must retain the continuation");
            let resume = function(instance, store, "resume");
            let value = scalar(store.invoke(resume, &[handle, Value::I32(7)]).unwrap());
            assert_eq!(value, 7);
            let after_resume = store.continuation_bytes();
            let second = store.invoke(resume, &[handle, Value::I32(7)]).unwrap_err();
            assert_eq!(second.kind, ErrorKind::Trap);
            drop(root);
            let stats = store.collect().unwrap();
            let after_collect = store.continuation_bytes();
            let payload_json = if case == "integration_cont_prefix" {
                format!("{{\"values\":[{{\"type\":\"i32\",\"value\":{}}}]}}", n.wrapping_add(32))
            } else { "{\"values\":[]}".to_owned() };
            let life = format!("{{\"resume_input\":\"i32:7\",\"resume_result\":{},\"second_resume\":{{\"kind\":\"trap\",\"error_kind\":\"Trap\"}},\"capture_bytes\":{},\"after_resume_bytes\":{},\"after_drop_and_collect_bytes\":{},\"collection_after_root_release\":{{\"live\":{},\"reclaimed\":{}}}}}",
                returned(value), capture_bytes, after_resume, after_collect, stats.live, stats.reclaimed);
            ("\"suspended\"".to_owned(), "{\"kind\":\"suspended\"}".to_owned(), payload_json, life)
        }
        other => panic!("unexpected integration outcome: {other:?}"),
    };
    Snapshot { result, outcome: outcome_json, payload, lifecycle,
        memory: store.read_memory(mem, 0, 65_536).unwrap(), captured_memory }
}
fn instantiate(engine: &wtime_runtime::Engine, module: &Module) -> (Store, Instance) {
    let mut store = Store::new(engine);
    let instance = Linker::new(engine).instantiate(&mut store, module).unwrap();
    (store, instance)
}
fn sample(bytes: &[u8], case: &str, stage: &str, n: i32, requested_batch: usize, mode: &str, probe: bool) -> (f64, usize, Snapshot) {
    let engine = super::configured_engine(Config { features: FeatureSet::all_phase3_core(), ..Config::default() }, mode);
    if stage == "load_and_call" {
        let start = Instant::now();
        let module = engine.compile(black_box(bytes)).unwrap();
        let (mut store, instance) = instantiate(&engine, &module);
        let func = function(instance, &store, "run");
        let outcome = store.invoke(func, &[Value::I32(n)]).unwrap();
        let elapsed = start.elapsed().as_nanos().max(1) as f64;
        return (elapsed, 1, finish(&mut store, instance, case, n, outcome));
    }
    let prepare_start = Instant::now();
    let module = engine.compile(black_box(bytes)).unwrap();
    let prepare_ns = prepare_start.elapsed().as_nanos().max(1) as f64;
    let instantiate_start = Instant::now();
    let (mut store, instance) = instantiate(&engine, &module);
    let instantiate_ns = instantiate_start.elapsed().as_nanos().max(1) as f64;
    let func = function(instance, &store, "run");
    if stage != "warm_call" {
        let start = Instant::now();
        let outcome = store.invoke(func, &[Value::I32(n)]).unwrap();
        let call_ns = start.elapsed().as_nanos().max(1) as f64;
        let snapshot = finish(&mut store, instance, case, n, outcome);
        return (match stage { "prepare" => prepare_ns, "instantiate" => instantiate_ns,
            "first_call" => call_ns, _ => panic!("unknown stage") }, 1, snapshot);
    }
    assert!(!probe);
    let outcome = store.invoke(func, &[Value::I32(n)]).unwrap();
    let mut snapshot = finish(&mut store, instance, case, n, outcome);
    let mut batch = requested_batch.max(1);
    for _ in 0..8 {
        let mut measured = 0.0;
        for _ in 0..batch {
            store.write_memory(memory(instance, &store), 0, &[0; 4]).unwrap();
            let start = Instant::now();
            let outcome = store.invoke(func, black_box(&[Value::I32(n)])).unwrap();
            measured += start.elapsed().as_nanos().max(1) as f64;
            snapshot = finish(&mut store, instance, case, n, outcome);
        }
        if requested_batch != 0 || measured >= TARGET_NS { return (measured / batch as f64, batch, snapshot); }
        assert!(batch < MAX_BATCH, "calibration must reach the target");
        batch = ((batch as f64 * TARGET_NS / measured * 1.1).ceil() as usize).clamp(batch + 1, MAX_BATCH);
    }
    panic!("calibration did not converge")
}
pub(super) fn try_main(args: &[String]) -> bool {
    if args.first().map(String::as_str) != Some("--integration-driver") { return false; }
    assert!(args.len() == 7 || (args.len() == 8 && args[7] == "--probe"), "--integration-driver FILE CASE STAGE INPUT BATCH MODE [--probe]");
    let path = &args[1]; let case = args[2].as_str(); let stage = args[3].as_str();
    assert!(CASES.contains(&case));
    assert!(std::fs::metadata(path).unwrap().len() <= 4 * 1024 * 1024);
    let bytes = std::fs::read(path).unwrap(); let n = args[4].parse::<i32>().unwrap();
    let requested_batch = args[5].parse::<usize>().unwrap(); assert!(requested_batch <= MAX_BATCH);
    let mode = &args[6]; let probe = args.len() == 8;
    let (ns, batch, state) = sample(&bytes, case, stage, n, requested_batch, mode, probe);
    let instrumented = cfg!(feature = "performance-counters") || cfg!(feature = "execution-reference");
    let captured = state.captured_memory.as_ref().map(|bytes| format!(",\"initial_memory_hex\":{}", super::quote(&hex(bytes)))).unwrap_or_default();
    println!("{{\"schema_version\":2,\"case\":{},\"stage\":{},\"input\":\"i32:{}\",\"batch\":{},\"calls_checked\":{},\"report_complete\":true,\"measured_guest_ns\":{},\"mode\":{},\"profile\":\"all-phase3-core\",\"instrumented\":{},\"diagnostic\":{},\"timing_excluded\":{},\"ns_per_operation\":{},\"expected\":{},\"result\":{},\"outcome\":{},\"payload\":{},\"lifecycle\":{},\"memory_count\":1,\"memory_pages\":1,\"memory_hex\":{}{} }}",
        super::quote(case), super::quote(stage), n, batch, batch, ns * batch as f64, super::quote(mode), instrumented, instrumented,
        probe, ns, state.result, state.result, state.outcome, state.payload, state.lifecycle,
        super::quote(&hex(&state.memory)), captured);
    true
}
