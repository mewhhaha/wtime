//! Fresh lifetimes expose preparation at the actual invocation threshold.
use std::{hint::black_box, time::Instant};
use wtime_runtime::{
    Config, Engine, ExecutionPolicy, External, Instance, Linker, RunOutcome, Store, Value,
};
use wtime_types::FeatureSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Strategy {
    Eager,
    Adaptive(u64),
}
impl Strategy {
    fn name(self) -> &'static str {
        match self {
            Self::Eager => "eager",
            Self::Adaptive(_) => "adaptive",
        }
    }
    fn threshold(self) -> String {
        match self {
            Self::Eager => "null".into(),
            Self::Adaptive(value) => value.to_string(),
        }
    }
}
struct Request {
    file: String,
    case: String,
    input: i32,
    expected: i32,
    calls: usize,
    strategy: Strategy,
    diagnostic: bool,
}
impl Request {
    fn parse(args: &[String]) -> Result<Self, String> {
        if args.len() != 9 {
            return Err("usage: --entry-batch|--entry-diagnostic FILE CASE INPUT EXPECTED CALLS register eager|adaptive THRESHOLD".into());
        }
        let diagnostic = match args[0].as_str() {
            "--entry-batch" => false,
            "--entry-diagnostic" => true,
            _ => return Err("unknown entry command".into()),
        };
        if args[2].is_empty()
            || args[2].len() > 128
            || !args[2]
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'-')
        {
            return Err("invalid case identity".into());
        }
        let input = args[3].parse().map_err(|_| "invalid i32 input")?;
        let expected = args[4].parse().map_err(|_| "invalid i32 expected result")?;
        let calls = args[5]
            .parse::<usize>()
            .map_err(|_| "invalid invocation count")?;
        if ![1, 10, 100, 10000].contains(&calls) {
            return Err("invocation count must be 1, 10, 100 or 10000".into());
        }
        if args[6] != "register" {
            return Err("entry protocol requires register mode".into());
        }
        let threshold = args[8]
            .parse::<u64>()
            .map_err(|_| "invalid entry threshold")?;
        let strategy = match (args[7].as_str(), threshold) {
            ("eager", 0) => Strategy::Eager,
            ("adaptive", value) if value > 0 => Strategy::Adaptive(value),
            _ => {
                return Err(
                    "eager requires threshold 0; adaptive requires a positive threshold".into(),
                );
            }
        };
        Ok(Self {
            file: args[1].clone(),
            case: args[2].clone(),
            input,
            expected,
            calls,
            strategy,
            diagnostic,
        })
    }
    fn engine(&self) -> Result<Engine, String> {
        let config = Config {
            features: FeatureSet::mvp(),
            ..Config::default()
        };
        assert!(config.limits.fuel.is_none());
        match self.strategy {
            Strategy::Eager => Engine::with_execution_policy(config, ExecutionPolicy::Register)
                .map_err(|error| error.to_string()),
            #[cfg(feature = "register-executor")]
            Strategy::Adaptive(threshold) => {
                Engine::with_register_entry_threshold(config, threshold)
                    .map_err(|error| error.to_string())
            }
            #[cfg(not(feature = "register-executor"))]
            Strategy::Adaptive(_) => Err("adaptive entry requires register-executor".into()),
        }
    }
    fn settings(&self) -> String {
        format!(
            "\"case\":\"{}\",\"input\":{},\"expected\":{},\"mode\":\"register\",\"strategy\":\"{}\",\"threshold\":{},\"requested_calls\":{},\"source_fuel\":null,\"feature_profile\":\"mvp\"",
            self.case,
            self.input,
            self.expected,
            self.strategy.name(),
            self.strategy.threshold(),
            self.calls
        )
    }
}
fn instrumented() -> bool {
    cfg!(any(
        feature = "execution-reference",
        feature = "performance-counters"
    ))
}
fn function(store: &Store, instance: Instance) -> Result<wtime_runtime::Func, String> {
    match instance
        .export(store, "run")
        .map_err(|error| error.to_string())?
    {
        External::Func(function) => Ok(function),
        _ => Err("run export must be a function".into()),
    }
}
fn invoke(store: &mut Store, function: wtime_runtime::Func, input: i32) -> Result<i32, String> {
    match store
        .invoke(function, black_box(&[Value::I32(input)]))
        .map_err(|error| error.to_string())?
    {
        RunOutcome::Returned(values) => match values.as_slice() {
            [Value::I32(value)] => Ok(black_box(*value)),
            _ => Err("run must return one i32".into()),
        },
        _ => Err("run did not return".into()),
    }
}
fn checked_calls(
    calls: usize,
    expected: i32,
    mut call: impl FnMut() -> Result<i32, String>,
) -> Result<(usize, i32), String> {
    let mut actual = 0;
    let mut last = 0;
    for index in 1..=calls {
        last = call()?;
        if last != expected {
            return Err(format!(
                "oracle mismatch at call {index}: expected {expected}, got {last}"
            ));
        }
        actual += 1;
    }
    Ok((actual, last))
}
fn batch(
    request: &Request,
    bytes: &[u8],
    inspect: impl FnOnce(&Store, Instance),
) -> Result<String, String> {
    if instrumented() {
        return Err("timing requires an uninstrumented build".into());
    }
    // Parsing, file I/O and Engine construction precede this one lifetime.
    let engine = request.engine()?;
    let start = Instant::now();
    let module = engine
        .compile(black_box(bytes))
        .map_err(|error| error.to_string())?;
    let prepared = start.elapsed().as_nanos();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .map_err(|error| error.to_string())?;
    let function = function(&store, instance)?;
    let instantiated = start.elapsed().as_nanos();
    let (actual, result) = checked_calls(request.calls, request.expected, || {
        invoke(&mut store, function, request.input)
    })?;
    let total = start.elapsed().as_nanos();
    // The task fixture exposes a memory counter to prove there was no hidden
    // warmup. Read it after timing, without making another guest invocation.
    let fixture_calls = if request.case == "p135_counter" {
        let External::Memory(counter) = instance
            .export(&store, "count")
            .map_err(|error| error.to_string())?
        else {
            return Err("counter fixture must export count memory".into());
        };
        let bytes: [u8; 4] = store
            .read_memory(counter, 0, 4)
            .map_err(|error| error.to_string())?
            .try_into()
            .map_err(|_| "invalid counter size")?;
        let count = i32::from_le_bytes(bytes);
        if count != request.calls as i32 {
            return Err(format!(
                "fixture count mismatch: expected {}, got {count}",
                request.calls
            ));
        }
        count.to_string()
    } else {
        "null".to_owned()
    };
    inspect(&store, instance);
    Ok(format!(
        "{{\"schema\":\"wtime-entry-batch-v1\",\"schema_version\":1,\"driver\":\"entry_batch\",{},\"actual_calls\":{actual},\"fixture_calls\":{fixture_calls},\"result\":{result},\"instrumented\":false,\"total_ns\":{total},\"prepare_ns\":{prepared},\"instantiate_ns\":{},\"invoke_ns\":{}}}",
        request.settings(),
        instantiated - prepared,
        total - instantiated
    ))
}
#[cfg(feature = "execution-reference")]
fn diagnostic(request: &Request, bytes: &[u8]) -> Result<String, String> {
    let engine = request.engine()?;
    let module = engine.compile(bytes).map_err(|error| error.to_string())?;
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .map_err(|error| error.to_string())?;
    let function = function(&store, instance)?;
    let mut rows = Vec::new();
    rows.try_reserve_exact(request.calls)
        .map_err(|_| "diagnostic allocation refused")?;
    let mut first = None;
    let mut call_index = 0;
    let (actual, result) = checked_calls(request.calls, request.expected, || {
        let before = (
            store.register_dispatches(),
            store.register_exits(),
            store.register_fallback_frames(),
        );
        let result = invoke(&mut store, function, request.input)?;
        call_index += 1;
        let steps = store
            .register_dispatches()
            .checked_sub(before.0)
            .ok_or("dispatch counter decreased")?;
        let exits = store
            .register_exits()
            .checked_sub(before.1)
            .ok_or("exit counter decreased")?;
        let fallback = store
            .register_fallback_frames()
            .checked_sub(before.2)
            .ok_or("fallback counter decreased")?;
        if steps != 0 && first.is_none() {
            first = Some(call_index);
        }
        rows.push(format!("{{\"call\":{call_index},\"register_dispatches\":{steps},\"register_exits\":{exits},\"fallback_frames\":{fallback}}}"));
        Ok(result)
    })?;
    let first = first.map_or_else(|| "null".to_owned(), |index| index.to_string());
    Ok(format!(
        "{{\"schema\":\"wtime-entry-diagnostic-v1\",\"schema_version\":1,\"driver\":\"entry_diagnostic\",{},\"actual_calls\":{actual},\"result\":{result},\"instrumented\":true,\"first_register_call\":{first},\"per_call\":[{}]}}",
        request.settings(),
        rows.join(",")
    ))
}
#[cfg(not(feature = "execution-reference"))]
fn diagnostic(_: &Request, _: &[u8]) -> Result<String, String> {
    Err("diagnostics require execution-reference".into())
}

pub(super) fn try_main(args: &[String]) -> bool {
    if !matches!(
        args.first().map(String::as_str),
        Some("--entry-batch" | "--entry-diagnostic")
    ) {
        return false;
    }
    let result = (|| {
        let request = Request::parse(args)?;
        // Feature refusals happen before reading a fixture or executing a guest.
        if request.diagnostic && !cfg!(feature = "execution-reference") {
            return Err("diagnostics require execution-reference".into());
        }
        if !request.diagnostic && instrumented() {
            return Err("timing requires an uninstrumented build".into());
        }
        let bytes = std::fs::read(&request.file).map_err(|error| error.to_string())?;
        if request.diagnostic {
            diagnostic(&request, &bytes)
        } else {
            batch(&request, &bytes, |_, _| {})
        }
    })();
    match result {
        Ok(row) => println!("{row}"),
        Err(error) => {
            eprintln!("entry protocol failed: {error}");
            std::process::exit(2);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    const COUNTER: &[u8] = include_bytes!("../fixtures/p135/counter.wasm");
    const BAD: &[u8] = include_bytes!("../fixtures/p135/late_bad_result.wasm");
    fn request(command: &str, calls: usize, strategy: &str, threshold: u64) -> Request {
        Request::parse(&[
            command.into(),
            "unused".into(),
            "p135_counter".into(),
            "7".into(),
            "8".into(),
            calls.to_string(),
            "register".into(),
            strategy.into(),
            threshold.to_string(),
        ])
        .unwrap()
    }
    #[test]
    fn p135_entry_batch_parser_rejects_unsupported_counts_and_strategy_pairs() {
        for calls in [1, 10, 100, 10000] {
            assert_eq!(request("--entry-batch", calls, "eager", 0).calls, calls);
            assert_eq!(
                request("--entry-batch", calls, "adaptive", 3).strategy,
                Strategy::Adaptive(3)
            );
        }
        let args = [
            "--entry-batch",
            "missing",
            "counter",
            "7",
            "8",
            "10",
            "register",
            "adaptive",
            "3",
        ]
        .map(str::to_owned);
        for (index, values) in [
            (5, vec!["0", "2", "-1", "10001", "18446744073709551616"]),
            (6, vec!["decoded", "native"]),
            (7, vec!["unknown"]),
            (8, vec!["0", "-1", "18446744073709551616"]),
        ] {
            for value in values {
                let mut bad = args.clone();
                bad[index] = value.into();
                assert!(Request::parse(&bad).is_err());
            }
        }
        let mut bad = args.clone();
        bad[7] = "eager".into();
        assert!(Request::parse(&bad).is_err());
    }
    #[test]
    fn p135_fresh_batch_times_compile_instantiation_and_exact_calls() {
        for strategy in ["eager", "adaptive"] {
            for calls in [1, 10, 100, 10000] {
                let request = request(
                    "--entry-batch",
                    calls,
                    strategy,
                    if strategy == "eager" { 0 } else { 3 },
                );
                if instrumented() {
                    assert!(
                        batch(&request, COUNTER, |_, _| panic!("instrumented guest ran")).is_err()
                    );
                    continue;
                }
                for _ in 0..2 {
                    let row = batch(&request, COUNTER, |store, instance| {
                        let External::Memory(count) = instance.export(store, "count").unwrap()
                        else {
                            panic!()
                        };
                        assert_eq!(
                            store.read_memory(count, 0, 4).unwrap(),
                            (calls as i32).to_le_bytes()
                        );
                    })
                    .unwrap();
                    assert!(row.contains(&format!("\"actual_calls\":{calls}")));
                    assert!(row.contains("\"total_ns\":"));
                    assert!(!row.contains("ns_per_operation"));
                }
            }
        }
    }
    #[test]
    fn p135_oracle_checks_every_call_and_stops_on_mismatch() {
        let mut attempted = 0;
        let error = checked_calls(10, 8, || {
            attempted += 1;
            Ok(if attempted == 2 { 12345 } else { 8 })
        })
        .unwrap_err();
        assert_eq!(attempted, 2);
        assert_eq!(error, "oracle mismatch at call 2: expected 8, got 12345");
        let request = request("--entry-batch", 10, "adaptive", 3);
        if !instrumented() {
            assert_eq!(
                batch(&request, BAD, |_, _| panic!("failed batch inspected")).unwrap_err(),
                error
            );
        }
    }
    #[test]
    fn p135_diagnostic_reports_dispatch_deltas_without_timing_or_promotion_claims() {
        let request = request("--entry-diagnostic", 10, "adaptive", 3);
        #[cfg(feature = "execution-reference")]
        for _ in 0..2 {
            let row = diagnostic(&request, COUNTER).unwrap();
            assert!(row.contains("\"first_register_call\":3"));
            assert!(row.contains(
                "\"call\":1,\"register_dispatches\":0,\"register_exits\":0,\"fallback_frames\":1"
            ));
            assert!(row.contains("\"actual_calls\":10"));
            for forbidden in ["_ns", "ns_per_operation", "promotion", "compile_count"] {
                assert!(!row.contains(forbidden));
            }
        }
        #[cfg(not(feature = "execution-reference"))]
        assert!(diagnostic(&request, COUNTER).is_err());
    }
}
