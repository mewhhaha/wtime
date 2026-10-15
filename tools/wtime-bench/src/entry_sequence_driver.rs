//! Individual invocation latencies for one fixed cold/FIFO/recompile schedule.
use std::{fmt::Write, hint::black_box, time::Instant};
use wtime_runtime::{
    Config, Engine, ExecutionPolicy, External, Func, Linker, RunOutcome, Store, Value,
};
use wtime_types::FeatureSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Strategy {
    Eager,
    Adaptive(u64),
}
struct Request {
    file: String,
    input: i32,
    schedule_threshold: usize,
    strategy: Strategy,
    diagnostic: bool,
}
impl Request {
    fn parse(args: &[String]) -> Result<Self, String> {
        if args.len() != 7 {
            return Err("usage: --entry-sequence|--entry-sequence-diagnostic FILE INPUT SCHEDULE_THRESHOLD register eager|adaptive THRESHOLD".into());
        }
        let diagnostic = match args[0].as_str() {
            "--entry-sequence" => false,
            "--entry-sequence-diagnostic" => true,
            _ => return Err("unknown sequence command".into()),
        };
        if args[4] != "register" || !cfg!(feature = "register-executor") {
            return Err("sequence requires available Register mode".into());
        }
        let positive = |text: &str| -> Result<u64, String> {
            if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err("invalid unsigned integer".into());
            }
            text.parse().map_err(|_| "unsigned integer overflow".into())
        };
        let schedule = positive(&args[3])?;
        if !(1..=64).contains(&schedule) {
            return Err("schedule threshold must be 1..64".into());
        }
        let threshold = positive(&args[6])?;
        let strategy = match (args[5].as_str(), threshold) {
            ("eager", 0) => Strategy::Eager,
            ("adaptive", value) if value == schedule => Strategy::Adaptive(value),
            _ => {
                return Err(
                    "eager requires 0; adaptive threshold must equal schedule threshold".into(),
                );
            }
        };
        Ok(Self {
            file: args[1].clone(),
            input: args[2].parse().map_err(|_| "invalid i32 input")?,
            schedule_threshold: schedule as usize,
            strategy,
            diagnostic,
        })
    }
    fn engine(&self) -> Result<Engine, String> {
        let config = Config {
            features: FeatureSet::mvp(),
            ..Config::default()
        };
        match self.strategy {
            Strategy::Eager => Engine::with_execution_policy(config, ExecutionPolicy::Register),
            #[cfg(feature = "register-executor")]
            Strategy::Adaptive(value) => Engine::with_register_entry_threshold(config, value),
            #[cfg(not(feature = "register-executor"))]
            Strategy::Adaptive(_) => return Err("Register unavailable".into()),
        }
        .map_err(|error| error.to_string())
    }
    fn validate_build(&self) -> Result<(), String> {
        if self.diagnostic {
            if !cfg!(feature = "execution-reference") {
                return Err("diagnostic requires execution-reference".into());
            }
        } else if cfg!(any(
            feature = "execution-reference",
            feature = "performance-counters"
        )) {
            return Err("timing requires an uninstrumented build".into());
        }
        Ok(())
    }
}
fn result(
    outcome: Result<RunOutcome, wtime_runtime::Error>,
    input: i32,
    body: usize,
) -> Result<i32, String> {
    let value = match outcome.map_err(|error| error.to_string())? {
        RunOutcome::Returned(values) => match values.as_slice() {
            [Value::I32(value)] => black_box(*value),
            _ => return Err("sequence function must return one i32".into()),
        },
        _ => return Err("sequence function did not return".into()),
    };
    if value != input.wrapping_add(body as i32) {
        return Err(format!("oracle mismatch for body {body}: got {value}"));
    }
    Ok(value)
}
fn schedule(threshold: usize) -> impl Iterator<Item = (usize, usize, &'static str)> {
    (0..66)
        .map(|body| (body, "initial"))
        .chain(std::iter::once((0, "revisit")))
        .flat_map(move |(body, phase)| (1..=threshold + 1).map(move |visit| (body, visit, phase)))
}

fn run(request: &Request, bytes: &[u8]) -> Result<String, String> {
    request.validate_build()?;
    let engine = request.engine()?;
    // All capacity, source I/O and Engine work precede preparation and call timers.
    let calls = 67 * (request.schedule_threshold + 1);
    let mut rows = Vec::new();
    rows.try_reserve_exact(calls)
        .map_err(|_| "sequence allocation refused")?;
    let start = (!request.diagnostic).then(Instant::now);
    let module = engine
        .compile(black_box(bytes))
        .map_err(|error| error.to_string())?;
    let prepare_ns = start.map(|timer| timer.elapsed().as_nanos());
    let start = (!request.diagnostic).then(Instant::now);
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .map_err(|error| error.to_string())?;
    let instantiate_ns = start.map(|timer| timer.elapsed().as_nanos());
    let functions = (0..66)
        .map(|body| {
            match instance
                .export(&store, &format!("f{body:02}"))
                .map_err(|error| error.to_string())?
            {
                External::Func(function) => Ok(function),
                _ => Err("sequence export must be a function".to_owned()),
            }
        })
        .collect::<Result<Vec<Func>, String>>()?;
    for (body, visit, phase) in schedule(request.schedule_threshold) {
        let args = black_box([Value::I32(request.input)]);
        #[cfg(feature = "execution-reference")]
        let dispatch_before = store.register_dispatches();
        #[cfg(feature = "execution-reference")]
        let fallback_before = store.register_fallback_frames();
        // No export lookup, oracle, snapshot, formatting or row storage is timed.
        let start = (!request.diagnostic).then(Instant::now);
        let outcome = store.invoke(functions[body], &args);
        let elapsed = start.map(|timer| timer.elapsed().as_nanos());
        let value = result(outcome, request.input, body)?;
        let mut row =
            format!("{{\"body\":{body},\"visit\":{visit},\"phase\":\"{phase}\",\"result\":{value}");
        if let Some(ns) = elapsed {
            write!(row, ",\"elapsed_ns\":{ns}").unwrap();
        }
        #[cfg(feature = "execution-reference")]
        if request.diagnostic {
            let dispatches = store
                .register_dispatches()
                .checked_sub(dispatch_before)
                .ok_or("dispatch counter decreased")?;
            let fallback = store
                .register_fallback_frames()
                .checked_sub(fallback_before)
                .ok_or("fallback counter decreased")?;
            if let Strategy::Adaptive(_) = request.strategy {
                use wtime_runtime::RegisterCacheEntryState;
                let cache = module
                    .register_cache_snapshot()
                    .ok_or("cache snapshot missing")?;
                let entry = cache
                    .entries
                    .iter()
                    .find(|entry| entry.body == body)
                    .ok_or("entry snapshot missing")?;
                let ready = matches!(entry.state, RegisterCacheEntryState::Ready { .. });
                let expected_ready = visit >= request.schedule_threshold;
                let expected_builders =
                    if phase == "initial" { body as u64 } else { 66 } + u64::from(expected_ready);
                let expected_evictions = if phase == "initial" {
                    body.saturating_sub(63) as u64
                } else {
                    3
                };
                if ready != expected_ready
                    || (!ready && entry.state != RegisterCacheEntryState::Cold)
                    || entry.entries != visit as u64
                    || cache.active_body.is_some()
                    || cache.lifecycle.builder_starts != expected_builders
                    || cache.lifecycle.published != expected_builders
                    || cache.lifecycle.evictions != expected_evictions
                    || cache.entries.len()
                        != if phase == "initial" {
                            (body + 1).min(64)
                        } else {
                            64
                        }
                    || cache.retained_bytes > cache.byte_limit
                    || dispatches != if ready { 4 } else { 0 }
                    || fallback != u64::from(!ready)
                {
                    return Err(format!(
                        "diagnostic schedule transition differs at {phase}/{body}/{visit}"
                    ));
                }
                write!(row, ",\"cache_state\":\"{}\",\"entries\":{},\"builder_starts\":{},\"published\":{},\"evictions\":{},\"resident_entries\":{},\"retained_bytes\":{},\"byte_limit\":{}",
                    if ready { "ready" } else { "cold" }, entry.entries, cache.lifecycle.builder_starts,
                    cache.lifecycle.published, cache.lifecycle.evictions, cache.entries.len(), cache.retained_bytes, cache.byte_limit).unwrap();
            } else if module.register_cache_snapshot().is_some() || dispatches != 4 || fallback != 0
            {
                return Err(
                    "eager diagnostic must use eager artifacts without a lazy cache".into(),
                );
            }
            write!(
                row,
                ",\"register_dispatches\":{dispatches},\"fallback_frames\":{fallback}"
            )
            .unwrap();
        }
        row.push('}');
        rows.push(row);
    }
    let (strategy, threshold) = match request.strategy {
        Strategy::Eager => ("eager", "null".to_owned()),
        Strategy::Adaptive(value) => ("adaptive", value.to_string()),
    };
    let mut output = format!(
        "{{\"schema\":\"wtime-entry-sequence-v1\",\"schema_version\":1,\"driver\":\"entry_sequence\",\"case\":\"uniform66\",\"mode\":\"register\",\"feature_profile\":\"mvp\",\"source_fuel\":null,\"input\":{},\"strategy\":\"{strategy}\",\"threshold\":{threshold},\"schedule_threshold\":{},\"requested_calls\":{calls},\"actual_calls\":{},\"instrumented\":{},\"diagnostic\":{}",
        request.input,
        request.schedule_threshold,
        rows.len(),
        request.diagnostic,
        request.diagnostic
    );
    if let (Some(prepare), Some(instantiate)) = (prepare_ns, instantiate_ns) {
        write!(
            output,
            ",\"prepare_ns\":{prepare},\"instantiate_ns\":{instantiate}"
        )
        .unwrap();
    }
    write!(output, ",\"per_call\":[{}]}}", rows.join(",")).unwrap();
    Ok(output)
}
fn run_args(args: &[String]) -> Result<String, String> {
    let request = Request::parse(args)?;
    request.validate_build()?;
    let bytes =
        std::fs::read(&request.file).map_err(|error| format!("fixture read failed: {error}"))?;
    run(&request, &bytes)
}
pub(super) fn try_main(args: &[String]) -> bool {
    if !matches!(
        args.first().map(String::as_str),
        Some("--entry-sequence" | "--entry-sequence-diagnostic")
    ) {
        return false;
    }
    match run_args(args) {
        Ok(output) => println!("{output}"),
        Err(error) => {
            eprintln!("entry sequence failed: {error}");
            std::process::exit(2);
        }
    }
    true
}

#[cfg(all(test, feature = "register-executor"))]
mod tests {
    use super::*;
    const UNIFORM: &[u8] = include_bytes!(
        "../../../crates/wtime-runtime/tests/fixtures/performance/p138/uniform66.wasm"
    );
    fn args(n: usize, strategy: &str, diagnostic: bool) -> Vec<String> {
        [
            if diagnostic {
                "--entry-sequence-diagnostic"
            } else {
                "--entry-sequence"
            }
            .to_owned(),
            "missing".into(),
            i32::MAX.to_string(),
            n.to_string(),
            "register".into(),
            strategy.into(),
            if strategy == "eager" {
                "0".into()
            } else {
                n.to_string()
            },
        ]
        .into()
    }
    #[test]
    fn p141_sequence_parser_and_build_reject_before_fixture_io() {
        let original = args(3, "adaptive", cfg!(feature = "execution-reference"));
        for (index, values) in [
            (
                3,
                vec!["0", "65", "-1", "true", "1.0", "18446744073709551616"],
            ),
            (4, vec!["decoded", "native"]),
            (5, vec!["unknown"]),
            (6, vec!["0", "2", "-1", "+3", "18446744073709551616"]),
        ] {
            for value in values {
                let mut invalid = original.clone();
                invalid[index] = value.into();
                let error = run_args(&invalid).unwrap_err();
                assert!(!error.starts_with("fixture read failed"));
            }
        }
        let mut wrong_build = original;
        wrong_build[0] = if cfg!(feature = "execution-reference") {
            "--entry-sequence"
        } else {
            "--entry-sequence-diagnostic"
        }
        .into();
        assert!(
            !run_args(&wrong_build)
                .unwrap_err()
                .starts_with("fixture read failed")
        );
    }
    #[test]
    fn p141_sequence_executes_every_initial_and_revisit_entry() {
        for n in [1, 3, 10, 64] {
            let plan = schedule(n).collect::<Vec<_>>();
            assert_eq!(plan.len(), 67 * (n + 1));
            assert_eq!(plan[0], (0, 1, "initial"));
            assert_eq!(plan[66 * (n + 1)], (0, 1, "revisit"));
            for strategy in ["eager", "adaptive"] {
                let request =
                    Request::parse(&args(n, strategy, cfg!(feature = "execution-reference")))
                        .unwrap();
                let output = run(&request, UNIFORM).unwrap();
                assert_eq!(output.matches("\"body\":").count(), plan.len());
                assert!(output.contains(&format!("\"actual_calls\":{}", plan.len())));
                if request.diagnostic {
                    assert!(!output.contains("_ns"));
                    if matches!(n, 3 | 10) {
                        eprintln!("P141_TRACE {output}");
                    }
                } else {
                    assert_eq!(output.matches("\"elapsed_ns\":").count(), plan.len());
                }
            }
        }
    }
    #[test]
    fn p141_sequence_oracle_and_fifo_recompile_are_checked() {
        for body in [0, 1, 65] {
            let expected = i32::MAX.wrapping_add(body as i32);
            assert_eq!(
                result(
                    Ok(RunOutcome::Returned(vec![Value::I32(expected)])),
                    i32::MAX,
                    body
                )
                .unwrap(),
                expected
            );
            assert!(
                result(
                    Ok(RunOutcome::Returned(vec![Value::I32(
                        expected.wrapping_add(1)
                    )])),
                    i32::MAX,
                    body
                )
                .is_err()
            );
        }
        let mut bad = UNIFORM.to_vec();
        let position = bad
            .windows(6)
            .position(|bytes| bytes == [0x20, 0, 0x41, 0, 0x6a, 0x0b])
            .unwrap();
        bad[position + 3] = 1;
        let request =
            Request::parse(&args(3, "adaptive", cfg!(feature = "execution-reference"))).unwrap();
        assert!(
            run(&request, &bad)
                .unwrap_err()
                .starts_with("oracle mismatch for body 0")
        );
        #[cfg(feature = "execution-reference")]
        {
            let request = Request::parse(&args(3, "adaptive", true)).unwrap();
            let output = run(&request, UNIFORM).unwrap();
            assert!(output.contains(
                "\"builder_starts\":67,\"published\":67,\"evictions\":3,\"resident_entries\":64"
            ));
            assert!(output.contains("\"phase\":\"revisit\""));
            assert!(!output.contains("elapsed_ns"));
        }
    }
}
