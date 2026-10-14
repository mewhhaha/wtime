#![forbid(unsafe_code)]
use std::{hint::black_box, time::Instant};
use wtime_runtime::{Config, Engine, External, Linker, RunOutcome, Store, Value};
use wtime_types::FeatureSet;
mod protocol;

fn ours() -> Engine {
    let config = Config {
        features: FeatureSet::mvp(),
        ..Config::default()
    };
    assert!(config.limits.fuel.is_none());
    Engine::new(config).unwrap()
}
fn theirs() -> wasmtime::Engine {
    let mut config = wasmtime::Config::new();
    config.strategy(wasmtime::Strategy::Cranelift);
    config.cranelift_opt_level(wasmtime::OptLevel::Speed);
    config.consume_fuel(false);
    config.epoch_interruption(false);
    // The parallel-compilation and cache Cargo features are disabled.
    wasmtime::Engine::new(&config).unwrap()
}
fn invoke(store: &mut Store, func: wtime_runtime::Func, args: &[Value]) -> i32 {
    match store.invoke(func, black_box(args)).unwrap() {
        RunOutcome::Returned(values) => match values.as_slice() {
            [Value::I32(value)] => *value,
            other => panic!("unexpected result {other:?}"),
        },
        other => panic!("unexpected outcome {other:?}"),
    }
}
fn foreign_invoke(
    store: &mut wasmtime::Store<()>,
    func: &wasmtime::Func,
    args: &[wasmtime::Val],
    out: &mut [wasmtime::Val],
) -> i32 {
    func.call(store, black_box(args), out).unwrap();
    out[0].unwrap_i32()
}
fn measure_batch(batch: usize, mut call: impl FnMut() -> i32, expected: i32) -> f64 {
    let start = Instant::now();
    let mut result = 0;
    for _ in 0..batch {
        result = black_box(call());
    }
    let ns = start.elapsed().as_nanos() as f64 / batch as f64;
    assert_eq!(result, expected);
    ns
}
fn batch_size(mut call: impl FnMut() -> i32, expected: i32) -> usize {
    let ns = measure_batch(1, &mut call, expected).max(1.0);
    (20_000_000.0 / ns).ceil().clamp(1.0, 100_000.0) as usize
}
fn row(case: &str, stage: &str, engine: &str, sample: usize, batch: usize, ns: f64) {
    println!("{case},{stage},{engine},{sample},{batch},{ns:.3}");
}
fn order(seed: &mut u64) -> [usize; 2] {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    if *seed & 1 == 0 { [0, 1] } else { [1, 0] }
}
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if protocol::try_main(&args) {
        return;
    }
    assert!(
        args.len() == 6,
        "usage: bench FILE CASE INPUT EXPECTED SAMPLES SEED; CASE=prepare_large omits invocation"
    );
    let bytes = std::fs::read(&args[0]).unwrap();
    let case = &args[1];
    let input = args[2].parse::<i32>().unwrap();
    let expected = args[3].parse::<i32>().unwrap();
    let samples = args[4].parse::<usize>().unwrap();
    let mut seed = args[5].parse::<u64>().unwrap();
    assert!(samples >= 5);
    // Compiler/process caches are warmed; each timed preparation uses a fresh engine.
    black_box(ours().compile(&bytes).unwrap());
    black_box(wasmtime::Module::new(&theirs(), &bytes).unwrap());
    for sample in 0..samples {
        for engine in order(&mut seed) {
            let ns = if engine == 0 {
                let e = ours();
                let start = Instant::now();
                let m = e.compile(black_box(&bytes)).unwrap();
                let ns = start.elapsed().as_nanos() as f64;
                black_box(m);
                ns
            } else {
                let e = theirs();
                let start = Instant::now();
                let m = wasmtime::Module::new(&e, black_box(&bytes)).unwrap();
                let ns = start.elapsed().as_nanos() as f64;
                black_box(m);
                ns
            };
            row(
                case,
                "prepare",
                if engine == 0 { "wtime" } else { "wasmtime" },
                sample,
                1,
                ns,
            );
        }
    }
    if case == "prepare_large" {
        return;
    }
    // Actual first-use latency: loaded binary through its first completed call.
    // Engine construction and file I/O remain outside the measured interval.
    for sample in 0..samples {
        for which in order(&mut seed) {
            if which == 0 {
                let e = ours();
                let start = Instant::now();
                let module = e.compile(&bytes).unwrap();
                let mut store = Store::new(&e);
                let instance = Linker::new(&e).instantiate(&mut store, &module).unwrap();
                let External::Func(func) = instance.export(&store, "run").unwrap() else {
                    panic!()
                };
                let value = invoke(&mut store, func, &[Value::I32(input)]);
                let ns = start.elapsed().as_nanos() as f64;
                assert_eq!(value, expected);
                row(case, "load_and_call", "wtime", sample, 1, ns);
            } else {
                let e = theirs();
                let start = Instant::now();
                let module = wasmtime::Module::new(&e, &bytes).unwrap();
                let mut store = wasmtime::Store::new(&e, ());
                let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
                let func = instance.get_func(&mut store, "run").unwrap();
                let value = foreign_invoke(
                    &mut store,
                    &func,
                    &[wasmtime::Val::I32(input)],
                    &mut [wasmtime::Val::I32(0)],
                );
                let ns = start.elapsed().as_nanos() as f64;
                assert_eq!(value, expected);
                row(case, "load_and_call", "wasmtime", sample, 1, ns);
            }
        }
    }
    let engine = ours();
    let module = engine.compile(&bytes).unwrap();
    let linker = Linker::new(&engine);
    let foreign_engine = theirs();
    let foreign_module = wasmtime::Module::new(&foreign_engine, &bytes).unwrap();
    let argv = [Value::I32(input)];
    let foreign_argv = [wasmtime::Val::I32(input)];
    for sample in 0..samples {
        for which in order(&mut seed) {
            if which == 0 {
                let mut store = Store::new(&engine);
                let start = Instant::now();
                let instance = linker.instantiate(&mut store, &module).unwrap();
                row(
                    case,
                    "instantiate",
                    "wtime",
                    sample,
                    1,
                    start.elapsed().as_nanos() as f64,
                );
                let External::Func(func) = instance.export(&store, "run").unwrap() else {
                    panic!()
                };
                let start = Instant::now();
                let value = invoke(&mut store, func, &argv);
                let ns = start.elapsed().as_nanos() as f64;
                assert_eq!(value, expected);
                row(case, "first_call", "wtime", sample, 1, ns);
            } else {
                let mut store = wasmtime::Store::new(&foreign_engine, ());
                let start = Instant::now();
                let instance = wasmtime::Instance::new(&mut store, &foreign_module, &[]).unwrap();
                row(
                    case,
                    "instantiate",
                    "wasmtime",
                    sample,
                    1,
                    start.elapsed().as_nanos() as f64,
                );
                let func = instance.get_func(&mut store, "run").unwrap();
                let mut out = [wasmtime::Val::I32(0)];
                let start = Instant::now();
                let value = foreign_invoke(&mut store, &func, &foreign_argv, &mut out);
                let ns = start.elapsed().as_nanos() as f64;
                assert_eq!(value, expected);
                row(case, "first_call", "wasmtime", sample, 1, ns);
            }
        }
    }
    let mut store = Store::new(&engine);
    let instance = linker.instantiate(&mut store, &module).unwrap();
    let External::Func(func) = instance.export(&store, "run").unwrap() else {
        panic!()
    };
    let mut foreign_store = wasmtime::Store::new(&foreign_engine, ());
    let foreign_instance =
        wasmtime::Instance::new(&mut foreign_store, &foreign_module, &[]).unwrap();
    let foreign_func = foreign_instance
        .get_func(&mut foreign_store, "run")
        .unwrap();
    let mut out = [wasmtime::Val::I32(0)];
    for _ in 0..5 {
        assert_eq!(invoke(&mut store, func, &argv), expected);
        assert_eq!(
            foreign_invoke(&mut foreign_store, &foreign_func, &foreign_argv, &mut out),
            expected
        );
    }
    let batch = batch_size(|| invoke(&mut store, func, &argv), expected);
    let foreign_batch = batch_size(
        || foreign_invoke(&mut foreign_store, &foreign_func, &foreign_argv, &mut out),
        expected,
    );
    for sample in 0..samples {
        for which in order(&mut seed) {
            if which == 0 {
                let ns = measure_batch(batch, || invoke(&mut store, func, &argv), expected);
                row(case, "warm_call", "wtime", sample, batch, ns);
            } else {
                let ns = measure_batch(
                    foreign_batch,
                    || foreign_invoke(&mut foreign_store, &foreign_func, &foreign_argv, &mut out),
                    expected,
                );
                row(case, "warm_call", "wasmtime", sample, foreign_batch, ns);
            }
        }
    }
}
