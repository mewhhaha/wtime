//! Repeated collection over a prebuilt, Store-global rooted graph.
//! Graph construction and final reclamation stay outside guest timing.
use std::{
    hint::black_box,
    sync::{Arc, Mutex},
    time::Instant,
};
use wtime_runtime::{
    Config, Error, ErrorKind, External, Func, FuncType, Linker, RunOutcome, Store, ValType, Value,
};
use wtime_types::{FeatureSet, ResourceLimits};

const MAX_BATCH: usize = 100_000;
const MAX_MODULE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Default)]
struct CollectionState {
    expected_live: usize,
    expected_payload: usize,
    callbacks: usize,
    payload_min: usize,
    payload_max: usize,
}

#[derive(Clone, Copy)]
struct Model {
    family: &'static str,
    nodes: usize,
    live: usize,
    input: i32,
    expected: i32,
}

fn model(family: &str, input: i32) -> Model {
    assert!([1, 17, 32].contains(&input), "declared collection count required");
    let (nodes, live) = match family {
        "gc_collect_wide_32" => (32usize, 33usize),
        "gc_collect_wide_1024" => (1024usize, 1025usize),
        "gc_collect_chain_256" => (256usize, 256usize),
        _ => panic!("unsupported repeated collection family"),
    };
    let n = i64::from(input);
    let graph_checksum = (nodes as i64) * (nodes.saturating_sub(1) as i64) / 2;
    let expected = graph_checksum + n * live as i64;
    Model {
        family: match family {
            "gc_collect_wide_32" => "wide",
            "gc_collect_wide_1024" => "wide",
            _ => "chain",
        },
        nodes,
        live,
        input,
        expected: expected as i32,
    }
}

fn elapsed(start: Instant) -> f64 {
    start.elapsed().as_nanos().max(1) as f64
}

struct Call {
    guest_ns: f64,
    callbacks: usize,
    payload_min: usize,
    payload_max: usize,
}

fn call(
    store: &mut Store,
    function: Func,
    model: Model,
    state: &Arc<Mutex<CollectionState>>,
) -> Call {
    {
        let mut state = state.lock().expect("collection host state lock");
        state.callbacks = 0;
        state.payload_min = usize::MAX;
        state.payload_max = 0;
    }
    let args = [Value::I32(model.input)];
    let start = Instant::now();
    let outcome = store
        .invoke(function, black_box(&args))
        .expect("repeated collection guest call failed");
    let guest_ns = elapsed(start);
    let RunOutcome::Returned(values) = outcome else {
        panic!("repeated collection guest did not return")
    };
    assert_eq!(values, vec![Value::I32(model.expected)], "collection checksum");
    store.release_published(&values);
    let (callbacks, payload_min, payload_max) = {
        let state = state.lock().expect("collection host state lock");
        assert_eq!(state.callbacks, model.input as usize, "collection callback count");
        assert!(state.payload_min > 0, "rooted graph has no GC payload");
        assert_eq!(state.payload_min, state.payload_max, "graph payload changed during collection");
        (state.callbacks, state.payload_min, state.payload_max)
    };
    Call {
        guest_ns,
        callbacks,
        payload_min,
        payload_max,
    }
}

fn run(
    path: &str,
    case: &str,
    family: &str,
    input: i32,
    mode: &str,
    profile: &str,
    stage: &str,
    batch: usize,
) -> String {
    assert_eq!(mode, "decoded");
    assert_eq!(profile, "all-phase3-core");
    assert_eq!(case, format!("{family}_n{input}"));
    let model = model(family, input);
    let max_batch =
        MAX_BATCH.min(100_000_000 / ((model.nodes + 1) * (input as usize + 1)));
    assert!((1..=max_batch).contains(&batch), "batch exceeds live graph bound");
    assert!(matches!(stage, "prepare" | "instantiate" | "first_call" | "warm_call" | "load_and_call"));
    assert!(stage == "warm_call" || batch == 1);
    assert!(path.len() <= 4096);
    let metadata = std::fs::metadata(path).expect("collection module metadata");
    assert!(metadata.len() <= MAX_MODULE_BYTES, "collection module exceeds size bound");
    let bytes = std::fs::read(path).expect("collection module read");
    assert!(bytes.len() as u64 <= MAX_MODULE_BYTES, "collection module exceeds size bound");

    let engine = super::configured_engine(
        Config {
            features: FeatureSet::all_phase3_core(),
            limits: ResourceLimits::default(),
        },
        mode,
    );
    let start = Instant::now();
    let module = engine.compile(&bytes).expect("collection module compile");
    let prepare_ns = elapsed(start);

    let start = Instant::now();
    let mut store = Store::new(&engine);
    let state = Arc::new(Mutex::new(CollectionState {
        expected_live: model.live,
        ..CollectionState::default()
    }));
    let callback_state = state.clone();
    let collect = store
        .create_host_func_with_caller(
            FuncType::new(vec![], vec![ValType::I32]),
            move |caller, _| {
                let (expected_live, expected_payload) = {
                    let state = callback_state
                        .lock()
                        .map_err(|_| Error::new(ErrorKind::Host, "collection host state lock"))?;
                    (state.expected_live, state.expected_payload)
                };
                let before = caller.store().gc_payload_bytes();
                let stats = caller.collect()?;
                let after = caller.store().gc_payload_bytes();
                if stats.live != expected_live
                    || stats.reclaimed != 0
                    || before == 0
                    || before != expected_payload
                    || after != before
                {
                    return Err(Error::new(
                        ErrorKind::Host,
                        "rooted collection graph state differs",
                    ));
                }
                let mut state = callback_state
                    .lock()
                    .map_err(|_| Error::new(ErrorKind::Host, "collection host state lock"))?;
                state.callbacks += 1;
                state.payload_min = state.payload_min.min(after);
                state.payload_max = state.payload_max.max(after);
                Ok(vec![Value::I32(stats.live as i32)])
            },
        )
        .expect("collection host import creation");
    let mut linker = Linker::new(&engine);
    linker.define("env", "collect", External::Func(collect)).expect("collection host import");
    let instance = linker.instantiate(&mut store, &module).expect("collection module instantiate");
    let External::Func(init) = instance.export(&store, "init").expect("init export") else {
        panic!("init export is not a function")
    };
    let External::Func(function) = instance.export(&store, "run").expect("run export") else {
        panic!("run export is not a function")
    };
    let External::Func(clear) = instance.export(&store, "clear").expect("clear export") else {
        panic!("clear export is not a function")
    };
    // Build once, outside all measured guest intervals. The exported global is
    // the Store root; init returns no reference that could create a published root.
    match store.invoke(init, &[]).expect("collection graph initialization") {
        RunOutcome::Returned(values) if values.is_empty() => {}
        other => panic!("init returned unexpected outcome: {other:?}"),
    }
    let graph_payload = store.gc_payload_bytes();
    assert!(graph_payload > 0, "initialized graph has no GC payload");
    state.lock().expect("collection host state lock").expected_payload = graph_payload;
    let instantiate_ns = elapsed(start);

    let first = call(&mut store, function, model, &state);
    let mut warm_ns = 0.0;
    let mut callbacks = first.callbacks;
    let mut calls = 1usize;
    let mut payload_min = first.payload_min;
    let mut payload_max = first.payload_max;
    if stage == "warm_call" {
        for _ in 0..batch {
            let observation = call(&mut store, function, model, &state);
            warm_ns += observation.guest_ns;
            callbacks += observation.callbacks;
            calls += 1;
            payload_min = payload_min.min(observation.payload_min);
            payload_max = payload_max.max(observation.payload_max);
        }
    }

    let measured_guest_ns = if stage == "warm_call" { warm_ns } else { first.guest_ns };
    let sample_ns = match stage {
        "prepare" => prepare_ns,
        "instantiate" => instantiate_ns,
        "first_call" => first.guest_ns,
        "warm_call" => warm_ns / batch as f64,
        "load_and_call" => prepare_ns + instantiate_ns + first.guest_ns,
        _ => unreachable!(),
    };

    let cleanup_start = Instant::now();
    match store.invoke(clear, &[]).expect("collection graph clear") {
        RunOutcome::Returned(values) if values.is_empty() => {}
        other => panic!("clear returned unexpected outcome: {other:?}"),
    }
    let final_stats = store.collect().expect("final graph collection");
    let cleanup_ns = elapsed(cleanup_start);
    assert_eq!((final_stats.live, final_stats.reclaimed), (0, model.live));
    let final_payload = store.gc_payload_bytes();
    assert_eq!(final_payload, 0);
    assert_eq!(store.continuation_bytes(), 0);
    assert!(payload_min <= graph_payload && graph_payload <= payload_max);

    format!(
        concat!(
            "{{\"schema_version\":2,\"driver\":\"gc_collect_stages\",\"case\":{},\"family\":{},\"shape\":{},\"nodes\":{},\"live_objects\":{},\"input\":{},",
            "\"mode\":{},\"profile\":{},\"stage\":{},\"batch\":{},\"max_batch\":{},\"instrumented\":false,",
            "\"measurement_policy\":\"sum_guest_repeated_collect_intervals_excluding_setup_oracles_and_final_cleanup\",",
            "\"sample_ns\":{},\"prepare_ns\":{},\"instantiate_ns\":{},\"first_call_ns\":{},\"measured_guest_ns\":{},\"cleanup_ns\":{},",
            "\"calls_checked\":{},\"callbacks_checked\":{},\"callback_live\":{},\"callback_reclaimed\":0,\"callback_payload_min\":{},\"callback_payload_max\":{},",
            "\"final_live\":{},\"final_reclaimed\":{},\"final_payload_bytes\":{},\"final_continuation_bytes\":0,\"result\":{},\"report_complete\":true}}"
        ),
        super::quote(case),
        super::quote(family),
        super::quote(model.family),
        model.nodes,
        model.live,
        input,
        super::quote(mode),
        super::quote(profile),
        super::quote(stage),
        batch,
        max_batch,
        sample_ns,
        prepare_ns,
        instantiate_ns,
        first.guest_ns,
        measured_guest_ns,
        cleanup_ns,
        calls,
        callbacks,
        model.live,
        payload_min,
        payload_max,
        final_stats.live,
        final_stats.reclaimed,
        final_payload,
        super::quote(&format!("i32:{}", model.expected)),
    )
}

pub fn try_main(args: &[String]) -> bool {
    if args.first().map(String::as_str) != Some("--gc-collect-driver") {
        return false;
    }
    assert!(
        !cfg!(any(feature = "performance-counters", feature = "execution-reference")),
        "GC collection timings require an uninstrumented build"
    );
    assert_eq!(args.len(), 9);
    let output = run(
        &args[1],
        &args[2],
        &args[3],
        args[4].parse().expect("collection input"),
        &args[5],
        &args[6],
        &args[7],
        args[8].parse().expect("collection batch"),
    );
    println!("{output}");
    true
}
