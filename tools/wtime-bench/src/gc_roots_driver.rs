//! Staged host reentry with fixed numeric/reference local pressure.
//! Oracles and heap resets are outside every guest interval.
use std::{
    hint::black_box,
    sync::{Arc, Mutex},
    time::Instant,
};
use wtime_runtime::{
    Config, Error, ErrorKind, External, Func, FuncType, Linker, RunOutcome, Store, ValType, Value,
};
use wtime_types::{FeatureSet, ResourceLimits};

#[derive(Default)]
struct HostState {
    nested: Option<Func>,
    callbacks: usize,
    nested_checked: usize,
    payload_min: usize,
    payload_max: usize,
}
#[derive(Clone, Copy)]
struct Model {
    slots: usize,
    numeric: bool,
    input: i32,
    expected: i32,
}
fn model(family: &str, input: i32) -> Model {
    assert!(
        [0, 1, 17, 32].contains(&input),
        "bounded declared input required"
    );
    let (slots, numeric) = match family {
        "gc_root_snapshots_numeric_64" => (64, true),
        "gc_root_snapshots_numeric_256" => (256, true),
        "gc_root_snapshots_numeric_512" => (512, true),
        "gc_root_snapshots_reference_64" => (64, false),
        "gc_root_snapshots_reference_256" => (256, false),
        "gc_root_snapshots_reference_512" => (512, false),
        _ => panic!("unsupported root snapshot family"),
    };
    let count = slots as i64;
    let expected =
        (count + 43) * i64::from(input) + if numeric { count * (count - 1) / 2 } else { 0 };
    Model {
        slots,
        numeric,
        input,
        expected: expected as i32,
    }
}
fn elapsed(start: Instant) -> f64 {
    start.elapsed().as_nanos().max(1) as f64
}
struct Call {
    guest_ns: f64,
    reset_ns: f64,
    callbacks: usize,
    payload_min: usize,
    payload_max: usize,
}
fn call(store: &mut Store, function: Func, model: Model, state: &Arc<Mutex<HostState>>) -> Call {
    {
        let mut state = state.lock().expect("host state lock");
        state.callbacks = 0;
        state.nested_checked = 0;
        state.payload_min = usize::MAX;
        state.payload_max = 0;
    }
    let args = [Value::I32(model.input)];
    let start = Instant::now();
    let outcome = store
        .invoke(function, black_box(&args))
        .expect("root snapshot guest failed");
    let guest_ns = elapsed(start);
    let RunOutcome::Returned(values) = outcome else {
        panic!("root snapshot guest did not return")
    };
    assert_eq!(
        values,
        vec![Value::I32(model.expected)],
        "independent root checksum"
    );
    let (callbacks, payload_min, payload_max) = {
        let state = state.lock().expect("host state lock");
        assert_eq!(
            state.callbacks, model.input as usize,
            "callback count differs"
        );
        assert_eq!(state.nested_checked, state.callbacks, "nested calls differ");
        if state.callbacks > 0 {
            assert!(state.payload_min > 0);
            assert_eq!(state.payload_min, state.payload_max);
        }
        (
            state.callbacks,
            if state.callbacks == 0 {
                0
            } else {
                state.payload_min
            },
            state.payload_max,
        )
    };
    let reset_start = Instant::now();
    store.release_published(&values);
    let stats = store.collect().expect("root reset failed");
    assert_eq!(
        (stats.live, stats.reclaimed),
        (0, 1),
        "root lifetime/reset differs"
    );
    assert_eq!(store.gc_payload_bytes(), 0);
    assert_eq!(store.continuation_bytes(), 0);
    Call {
        guest_ns,
        reset_ns: elapsed(reset_start),
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
    let max_batch = 100_000usize.min(100_000_000usize / ((model.slots + 8) * (input as usize + 1)));
    assert!((1..=max_batch).contains(&batch));
    assert!(matches!(
        stage,
        "prepare" | "instantiate" | "first_call" | "warm_call" | "load_and_call"
    ));
    assert!(stage == "warm_call" || batch == 1);
    assert!(path.len() <= 4096);
    assert!(std::fs::metadata(path).expect("module metadata").len() <= 4 * 1024 * 1024);
    let bytes = std::fs::read(path).expect("root module read");
    let engine = super::configured_engine(
        Config {
            features: FeatureSet::all_phase3_core(),
            limits: ResourceLimits::default(),
        },
        mode,
    );
    let start = Instant::now();
    let module = engine.compile(&bytes).expect("root compile");
    let prepare_ns = elapsed(start);
    let start = Instant::now();
    let mut store = Store::new(&engine);
    let state = Arc::new(Mutex::new(HostState::default()));
    let callback_state = state.clone();
    let collect = store
        .create_host_func_with_caller(
            FuncType::new(vec![], vec![ValType::I32]),
            move |caller, _| {
                let nested = callback_state
                    .lock()
                    .map_err(|_| Error::new(ErrorKind::Host, "root host lock"))?
                    .nested
                    .ok_or_else(|| Error::new(ErrorKind::Host, "root helper unavailable"))?;
                let before = caller.store().gc_payload_bytes();
                let stats = caller.collect()?;
                if stats.live != 1
                    || stats.reclaimed != 0
                    || before == 0
                    || caller.store().gc_payload_bytes() != before
                {
                    return Err(Error::new(
                        ErrorKind::Host,
                        "root callback lifetime differs",
                    ));
                }
                let RunOutcome::Returned(values) = caller.invoke(nested, &[])? else {
                    return Err(Error::new(ErrorKind::Host, "nested outcome differs"));
                };
                if values != [Value::I32(42)] {
                    return Err(Error::new(ErrorKind::Host, "nested result differs"));
                }
                let mut state = callback_state
                    .lock()
                    .map_err(|_| Error::new(ErrorKind::Host, "root host lock"))?;
                state.callbacks += 1;
                state.nested_checked += 1;
                state.payload_min = state.payload_min.min(before);
                state.payload_max = state.payload_max.max(before);
                Ok(values)
            },
        )
        .expect("root callback creation");
    let mut linker = Linker::new(&engine);
    linker
        .define("env", "collect", External::Func(collect))
        .expect("root import");
    let instance = linker
        .instantiate(&mut store, &module)
        .expect("root instantiate");
    let External::Func(function) = instance.export(&store, "run").expect("run export") else {
        panic!("wrong run export")
    };
    let External::Func(nested) = instance.export(&store, "nested").expect("nested export") else {
        panic!("wrong nested export")
    };
    state.lock().expect("host lock").nested = Some(nested);
    let instantiate_ns = elapsed(start);
    let first = call(&mut store, function, model, &state);
    let mut warm_ns = 0.0;
    let mut reset_ns = first.reset_ns;
    let mut callbacks = first.callbacks;
    let mut calls = 1usize;
    let mut payload_min = first.payload_min;
    let mut payload_max = first.payload_max;
    if stage == "warm_call" {
        for _ in 0..batch {
            let observation = call(&mut store, function, model, &state);
            warm_ns += observation.guest_ns;
            reset_ns += observation.reset_ns;
            callbacks += observation.callbacks;
            calls += 1;
            if observation.callbacks > 0 {
                payload_min = payload_min.min(observation.payload_min);
                payload_max = payload_max.max(observation.payload_max);
            }
        }
    }
    let sample_ns = match stage {
        "prepare" => prepare_ns,
        "instantiate" => instantiate_ns,
        "first_call" => first.guest_ns,
        "warm_call" => warm_ns / batch as f64,
        "load_and_call" => prepare_ns + instantiate_ns + first.guest_ns,
        _ => unreachable!(),
    };
    let guest_ns = if stage == "warm_call" {
        warm_ns
    } else {
        first.guest_ns
    };
    format!(
        concat!(
            "{{\"schema_version\":2,\"driver\":\"gc_root_stages\",\"case\":{},\"family\":{},\"input\":{},",
            "\"mode\":{},\"profile\":{},\"stage\":{},\"batch\":{},\"max_batch\":{},\"instrumented\":false,",
            "\"measurement_policy\":\"sum_guest_invoke_intervals_excluding_each_oracle_and_gc_reset\",",
            "\"sample_ns\":{},\"prepare_ns\":{},\"instantiate_ns\":{},\"first_call_ns\":{},\"measured_guest_ns\":{},\"reset_ns\":{},",
            "\"calls_checked\":{},\"total_reclaimed\":{},\"reclaimed_per_call\":1,\"live_after_reset\":0,\"payload_bytes_after_reset\":0,\"continuation_bytes_after_reset\":0,",
            "\"callbacks_checked\":{},\"nested_calls_checked\":{},\"callback_live\":{},\"callback_reclaimed\":0,\"callback_payload_min\":{},\"callback_payload_max\":{},",
            "\"shape\":{},\"bank_slots\":{},\"result\":{},\"report_complete\":true}}"
        ),
        super::quote(case),
        super::quote(family),
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
        guest_ns,
        reset_ns,
        calls,
        calls,
        callbacks,
        callbacks,
        usize::from(callbacks > 0),
        payload_min,
        payload_max,
        super::quote(if model.numeric {
            "numeric"
        } else {
            "reference"
        }),
        model.slots,
        super::quote(&format!("i32:{}", model.expected))
    )
}
pub fn try_main(args: &[String]) -> bool {
    if args.first().map(String::as_str) != Some("--gc-root-driver") {
        return false;
    }
    assert!(
        !cfg!(any(
            feature = "performance-counters",
            feature = "execution-reference"
        )),
        "root timings require an uninstrumented build"
    );
    assert_eq!(args.len(), 9);
    let output = run(
        &args[1],
        &args[2],
        &args[3],
        args[4].parse().expect("root input"),
        &args[5],
        &args[6],
        &args[7],
        args[8].parse().expect("root batch"),
    );
    println!("{output}");
    true
}
