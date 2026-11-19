//! wtime-goose [--tier decoded|compact|register|register-adaptive=N] [--timing-json path] module.wasm [-- program arguments]
//!
//! Runs a direct Goose WebAssembly module (built by goose-to-wasm) on the
//! wtime interpreter, with the imports, output and exit codes of
//! goose-to-wasm's `goose-wasmtime` host. Stdin is the program's input.
//!
//! The timing JSON uses the same boundaries as `goose-wasmtime`:
//! `compile_instantiate_ns` covers module preparation (decode, validation and
//! interpreter lowering), import creation and instantiation; `start_function_ns`
//! covers `_start`.

#![forbid(unsafe_code)]

mod float;
mod host;
#[allow(dead_code)] // Copied whole from goose-to-wasm; not every helper is needed here.
mod pytext;

use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStringExt;
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use host::{State, define_imports};
use wtime_runtime::{Config, Engine, ExecutionPolicy, External, Linker, RunOutcome, Store};
use wtime_types::FeatureSet;

const USAGE: &str = "usage: wtime-goose [--tier decoded|compact|register|register-adaptive=N] [--timing-json path] module.wasm [-- program arguments]";

#[derive(Clone, Copy, Debug)]
enum Tier {
    Decoded,
    Compact,
    Register,
    RegisterAdaptive(u64),
}

impl Tier {
    fn name(self) -> String {
        match self {
            Tier::Decoded => "decoded".into(),
            Tier::Compact => "compact".into(),
            Tier::Register => "register-eager".into(),
            Tier::RegisterAdaptive(threshold) => format!("register-adaptive-{threshold}"),
        }
    }
}

struct Options {
    module: String,
    timing_json: Option<String>,
    tier: Tier,
    arguments: Option<Vec<Vec<u8>>>,
}

fn usage_error(message: &str) -> ! {
    eprintln!("{USAGE}\nwtime-goose: error: {message}");
    exit(2)
}

fn options() -> Options {
    let mut own: Vec<Vec<u8>> = std::env::args_os().skip(1).map(OsString::into_vec).collect();
    let arguments = own.iter().position(|argument| argument == b"--").map(|at| {
        let rest = own.split_off(at + 1);
        own.pop();
        rest
    });
    let mut own = own.into_iter().map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let (mut module, mut timing_json, mut tier) = (None, None, Tier::Decoded);
    while let Some(argument) = own.next() {
        match argument.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                exit(0)
            }
            "--timing-json" => timing_json = Some(own.next().unwrap_or_else(|| usage_error("--timing-json needs a path"))),
            "--tier" => {
                let value = own.next().unwrap_or_else(|| usage_error("--tier needs a value"));
                tier = match value.as_str() {
                    "decoded" => Tier::Decoded,
                    "compact" => Tier::Compact,
                    "register" | "register-eager" => Tier::Register,
                    other => match other.strip_prefix("register-adaptive=").map(str::parse) {
                        Some(Ok(threshold)) => Tier::RegisterAdaptive(threshold),
                        _ => usage_error(&format!("unknown tier {other}")),
                    },
                };
            }
            other if other.starts_with('-') => usage_error(&format!("unrecognized argument {other}")),
            _ if module.is_none() => module = Some(argument),
            other => usage_error(&format!("unrecognized argument {other}")),
        }
    }
    let Some(module) = module else { usage_error("the following arguments are required: module") };
    Options { module, timing_json, tier, arguments }
}

/// Limits wide enough for the Goose compiler compiled to Wasm: deep recursion
/// over syntax trees and a large 64-bit memory. wtime keeps guest frames on
/// the heap, so a deep call stack needs no native stack.
fn config() -> Config {
    let mut config = Config { features: FeatureSet::all_phase3_core(), ..Config::default() };
    let limits = &mut config.limits;
    limits.max_call_depth = 4_000_000;
    limits.max_stack_height = 64_000_000;
    limits.max_memory_pages = 1 << 20;
    limits.max_memory_bytes = 64 << 30;
    limits.max_store_memory_bytes = 64 << 30;
    limits.max_instructions = 1 << 30;
    limits.max_module_bytes = 256 << 20;
    config
}

fn engine(tier: Tier) -> Result<Engine, String> {
    let config = config();
    let engine = match tier {
        Tier::Decoded => Engine::with_execution_policy(config, ExecutionPolicy::Decoded),
        Tier::Compact => Engine::with_execution_policy(config, ExecutionPolicy::Compact),
        Tier::Register => Engine::with_execution_policy(config, ExecutionPolicy::Register),
        #[cfg(feature = "register-executor")]
        Tier::RegisterAdaptive(threshold) => Engine::with_register_entry_threshold(config, threshold),
        #[cfg(not(feature = "register-executor"))]
        Tier::RegisterAdaptive(_) => return Err("this build has no register executor".into()),
    };
    engine.map_err(|error| error.to_string())
}

struct Outcome {
    stdout: Vec<u8>,
    stderr: String,
    exit_code: i64,
    compile_instantiate_ns: u128,
    compile_ns: u128,
    start_function_ns: u128,
    engine_total_ns: u128,
}

fn run_module(data: &[u8], stdin: Vec<u8>, arguments: Vec<Vec<u8>>, tier: Tier) -> Result<Outcome, String> {
    let engine = engine(tier)?;
    let state = Arc::new(Mutex::new(State::new(stdin, arguments)));
    let started = Instant::now();
    let mut instantiated = started;
    let mut compiled = started;
    let mut store_slot: Option<Store> = None;
    let result = (|| -> Result<(), String> {
        let module = engine.compile(data).map_err(|error| error.to_string())?;
        compiled = Instant::now();
        let store = store_slot.insert(Store::new(&engine));
        let mut linker = Linker::new(&engine);
        for (module_name, name, function) in define_imports(store, &module, &state)? {
            linker.define(&module_name, &name, External::Func(function)).map_err(|error| error.to_string())?;
        }
        let instance = linker.instantiate(store, &module).map_err(|error| error.to_string())?;
        {
            let mut state = state.lock().expect("host state lock");
            if let Ok(External::Memory(memory)) = instance.export(store, "memory") {
                state.memory = Some(memory);
            }
            if let Ok(External::Func(reserve)) = instance.export(store, "goose.reserve") {
                state.reserve = Some(reserve);
            }
        }
        if instance.export(store, "__set_stack_limits").is_ok() {
            return Err("emscripten standalone modules are not supported by this host".into());
        }
        let External::Func(start) =
            instance.export(store, "_start").map_err(|_| "module does not export _start".to_string())?
        else {
            return Err("'_start' is not a function".into());
        };
        instantiated = Instant::now();
        let outcome = store.invoke(start, &[]).map_err(|error| error.to_string());
        #[cfg(feature = "diagnostics")]
        if std::env::var_os("WTIME_GOOSE_STATS").is_some() {
            eprintln!(
                "register: dispatches {} exits {} fallback frames {}; compact: dispatches {} fused {} inline {} memory {}",
                store.register_dispatches(),
                store.register_exits(),
                store.register_fallback_frames(),
                store.compact_dispatches(),
                store.compact_fused_groups(),
                store.compact_inline_leaf_calls(),
                store.compact_memory_accesses()
            );
        }
        match outcome? {
            RunOutcome::Returned(_) => Ok(()),
            RunOutcome::Exception(exception) => Err(format!("uncaught guest exception: {exception:?}")),
            RunOutcome::Suspended { tag, .. } => Err(format!("guest suspended at {tag:?}")),
        }
    })();
    let ended = Instant::now();
    let mut state = state.lock().expect("host state lock");
    if std::env::var_os("WTIME_GOOSE_MEMORY").is_some() {
        if let (Some(memory), Some(store)) = (state.memory, store_slot.as_ref()) {
            // The memory's length, found by probing: the store has no length query.
            let (mut low, mut high) = (0u64, 1u64 << 40);
            while low + 1 < high {
                let middle = (low + high) / 2;
                if store.read_memory(memory, middle - 1, 1).is_ok() { low = middle } else { high = middle }
            }
            state.err.extend_from_slice(format!("wtime memory bytes: {low}\n").as_bytes());
        }
    }
    let (exit_code, error) = match (result, state.exit, state.failure.take()) {
        (_, Some(code), _) => (code, String::new()),
        (Ok(()), None, _) => (0, String::new()),
        (Err(_), None, Some(failure)) => (1, failure),
        (Err(error), None, None) => (1, error),
    };
    Ok(Outcome {
        stdout: std::mem::take(&mut state.out),
        stderr: String::from_utf8_lossy(&state.err).into_owned() + &error,
        exit_code,
        compile_instantiate_ns: (instantiated - started).as_nanos(),
        compile_ns: compiled.saturating_duration_since(started).as_nanos(),
        start_function_ns: (ended - instantiated).as_nanos(),
        engine_total_ns: (ended - started).as_nanos(),
    })
}

fn run(options: &Options) -> Result<i64, String> {
    let data = std::fs::read(&options.module).map_err(|error| format!("{}: {error}", options.module))?;
    let mut input = Vec::new();
    std::io::stdin().lock().read_to_end(&mut input).map_err(|error| error.to_string())?;
    let program = match &options.arguments {
        None => vec![b"program".to_vec()],
        Some(arguments) => {
            let mut program = vec![options.module.as_bytes().to_vec()];
            program.extend(arguments.iter().cloned());
            program
        }
    };
    let outcome = run_module(&data, input, program, options.tier)?;
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(&outcome.stdout).and_then(|()| stdout.flush());
    if !outcome.stderr.is_empty() {
        let _ = std::io::stderr().lock().write_all((outcome.stderr.clone() + "\n").as_bytes());
    }
    if let Some(path) = &options.timing_json {
        let json = format!(
            "{{\n  \"engine\": \"wtime\",\n  \"tier\": \"{}\",\n  \"compile_instantiate_ns\": {},\n  \"compile_ns\": {},\n  \"start_function_ns\": {},\n  \"engine_total_ns\": {}\n}}\n",
            options.tier.name(),
            outcome.compile_instantiate_ns,
            outcome.compile_ns,
            outcome.start_function_ns,
            outcome.engine_total_ns
        );
        std::fs::write(path, json).map_err(|error| format!("{path}: {error}"))?;
    }
    Ok(outcome.exit_code)
}

fn main() {
    let options = options();
    let code = run(&options).unwrap_or_else(|error| {
        eprintln!("wtime Goose host: {error}");
        1
    });
    exit(code as i32)
}
