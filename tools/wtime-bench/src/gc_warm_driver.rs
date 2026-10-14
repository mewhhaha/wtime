//! GC scalar stages with independent checks and collection outside guest timing.
//! Guest intervals exclude independent oracle and per-call GC reset costs.
use std::{hint::black_box, time::Instant};
use wtime_runtime::{Config, External, Linker, RunOutcome, Store, Value};
use wtime_types::{FeatureSet, ResourceLimits};

#[derive(Clone, Copy)]
struct Model {
    expected: i32,
    reclaimed: usize,
    export: &'static str,
}
fn model(family: &str, n: i32) -> Model {
    assert!(
        (0..=10_000).contains(&n),
        "bounded nonnegative input required"
    );
    let n = i64::from(n);
    let (answer, reclaimed) = match family {
        "gc_nominal_exact_casts" => (2 * n, n as usize),
        "gc_typed_function_refs" => (n * (n - 1) / 2 + n, 0),
        "gc_indirect_subtype_calls" => (128 * n, 0),
        "gc_type_churn_64" | "gc_type_churn_256" | "gc_type_churn_1024" => {
            let types: i64 = family.rsplit('_').next().unwrap().parse().unwrap();
            let cycles = n / types;
            let tail = n % types;
            (cycles * types * (types - 1) / 2 + tail * (tail - 1) / 2, 0)
        }
        "gc_layout_array_16" | "gc_layout_array_256" | "gc_layout_array_1024"
        | "gc_layout_struct_16" | "gc_layout_struct_256" | "gc_layout_struct_1024" => (255 * n, 1),
        _ => panic!("unsupported GC scalar family"),
    };
    Model {
        expected: answer as i32,
        reclaimed,
        export: if family.starts_with("gc_layout_struct_") { "run_struct" } else { "run" },
    }
}
fn ns(start: Instant) -> f64 {
    start.elapsed().as_nanos().max(1) as f64
}
struct Call {
    guest_ns: f64,
    reset_ns: f64,
    reclaimed: usize,
}
fn call(store: &mut Store, f: wtime_runtime::Func, n: i32, model: Model) -> Call {
    let arguments = [Value::I32(n)];
    let start = Instant::now();
    let outcome = store
        .invoke(f, black_box(&arguments))
        .expect("guest scalar invocation failed");
    let guest_ns = ns(start);
    let RunOutcome::Returned(values) = outcome else {
        panic!("scalar family returned an exception")
    };
    assert_eq!(
        values.as_slice(),
        &[Value::I32(model.expected)],
        "independent scalar oracle mismatch"
    );
    let reset_start = Instant::now();
    store.release_published(&values);
    // This reset is outside every guest interval, including warm calls.
    // It bounds allocation independently of calibration/batch size.
    let stats = store.collect().expect("post-call GC failed");
    assert_eq!(stats.live, 0, "unexpected retained guest roots");
    assert_eq!(
        stats.reclaimed, model.reclaimed,
        "allocation count mismatch"
    );
    assert_eq!(store.gc_payload_bytes(), 0, "payload retained after reset");
    assert_eq!(
        store.continuation_bytes(),
        0,
        "continuation retained after reset"
    );
    Call {
        guest_ns,
        reset_ns: ns(reset_start),
        reclaimed: stats.reclaimed,
    }
}

// CLI wiring must enforce default timing feature set, and pass a family separate
// from the registry ID. The complete registry ID is still bound in compare.py.
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
    assert_eq!(profile, "all-phase3-core");
    let expected_case = if family.starts_with("gc_layout_") {
        format!("{family}_n{input}")
    } else {
        format!("gc_warm_{}_n{}", family.strip_prefix("gc_").unwrap(), input)
    };
    assert_eq!(case, expected_case);
    let model = model(family, input);
    let batch = if batch == 0 { 1 } else { batch };
    assert!(matches!(
        stage,
        "prepare" | "instantiate" | "first_call" | "warm_call" | "load_and_call"
    ));
    let max_batch = 100_000usize.min(100_000_000usize / input.max(1) as usize);
    assert!(
        (1..=max_batch).contains(&batch),
        "bounded call/input product required"
    );
    assert!(stage == "warm_call" || batch == 1);
    assert!(path.len() <= 4096);
    let metadata = std::fs::metadata(path).expect("module metadata failed");
    assert!(metadata.len() <= 4 * 1024 * 1024);
    let bytes = std::fs::read(path).expect("module read failed");
    assert!(bytes.len() <= 4 * 1024 * 1024, "module size limit");
    let engine = super::configured_engine(
        Config {
            features: FeatureSet::all_phase3_core(),
            limits: ResourceLimits::default(),
        },
        mode,
    );
    let prepare_start = Instant::now();
    let module = engine.compile(&bytes).expect("validated compile failed");
    let prepare_ns = ns(prepare_start);
    let instantiate_start = Instant::now();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .expect("instantiate failed");
    let instantiate_ns = ns(instantiate_start);
    let External::Func(f) = instance
        .export(&store, model.export)
        .expect("missing scalar export")
    else {
        panic!("wrong export kind")
    };
    let first = call(&mut store, f, input, model);
    let mut warm_ns = 0.0;
    let mut reset_ns = first.reset_ns;
    let mut reclaimed = first.reclaimed;
    let mut calls = 1usize;
    if stage == "warm_call" {
        for _ in 0..batch {
            let observation = call(&mut store, f, input, model);
            warm_ns += observation.guest_ns;
            reset_ns += observation.reset_ns;
            reclaimed = reclaimed
                .checked_add(observation.reclaimed)
                .expect("count overflow");
            calls += 1;
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
    let measured_guest_ns = if stage == "warm_call" {
        warm_ns
    } else {
        first.guest_ns
    };
    format!(
        "{{\"schema_version\":2,\"driver\":\"gc_scalar_stages\",\"case\":{},\"family\":{},\"input\":{},\"mode\":{},\"profile\":{},\"stage\":{},\"batch\":{},\"max_batch\":{},\"instrumented\":false,\"measurement_policy\":\"sum_guest_invoke_intervals_excluding_each_oracle_and_gc_reset\",\"sample_ns\":{},\"prepare_ns\":{},\"instantiate_ns\":{},\"first_call_ns\":{},\"measured_guest_ns\":{},\"reset_ns\":{},\"calls_checked\":{},\"total_reclaimed\":{},\"reclaimed_per_call\":{},\"live_after_reset\":0,\"payload_bytes_after_reset\":0,\"continuation_bytes_after_reset\":0,\"result\":{},\"report_complete\":true}}",
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
        measured_guest_ns,
        reset_ns,
        calls,
        reclaimed,
        model.reclaimed,
        super::quote(&format!("i32:{}", model.expected))
    )
}

/// --gc-scalar-driver FILE CASE FAMILY INPUT MODE PROFILE STAGE BATCH
pub fn try_main(args: &[String]) -> bool {
    if args.first().map(String::as_str) != Some("--gc-scalar-driver") {
        return false;
    }
    assert!(
        !cfg!(any(
            feature = "performance-counters",
            feature = "execution-reference"
        )),
        "GC stage timings require an uninstrumented default build"
    );
    assert_eq!(
        args.len(),
        9,
        "--gc-scalar-driver FILE CASE FAMILY INPUT MODE PROFILE STAGE BATCH"
    );
    let input = args[4]
        .strip_prefix("i32:")
        .expect("typed i32:N input required")
        .parse::<i32>()
        .expect("invalid input");
    let batch = args[8].parse::<usize>().expect("invalid batch");
    println!(
        "{}",
        run(
            &args[1], &args[2], &args[3], input, &args[5], &args[6], &args[7], batch
        )
    );
    true
}
