#![forbid(unsafe_code)]
//! Compile-only diagnostic counters; never a complete heap census or timing.
use std::{env, fs, process::ExitCode};
use wtime_runtime::{Config, Engine, ExecutionPolicy, FeatureSet};

fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c <= '\u{1f}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn profile(name: &str) -> Result<FeatureSet, String> {
    match name {
        "mvp" => Ok(FeatureSet::mvp()),
        "stable-core" => Ok(FeatureSet::stable_core()),
        "all-phase3-core" => Ok(FeatureSet::all_phase3_core()),
        _ => Err(format!("unknown profile: {name}")),
    }
}

fn mode(name: &str, config: Config) -> Result<Engine, String> {
    match name {
        "decoded" => Engine::with_execution_policy(config, ExecutionPolicy::Decoded),
        "compact" => Engine::with_execution_policy(config, ExecutionPolicy::Compact),
        "register-eager" => Engine::with_execution_policy(config, ExecutionPolicy::Register),
        "register-adaptive-3" => Engine::with_register_entry_threshold(config, 3),
        _ => return Err(format!("unknown mode: {name}")),
    }
    .map_err(|error| error.to_string())
}

fn run(path: &str, profile_name: &str, mode_name: &str) -> Result<String, String> {
    let path = std::path::Path::new(path);
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let features = profile(profile_name)?;
    let config = Config {
        features,
        ..Config::default()
    };
    let engine = mode(mode_name, config)?;
    let module = engine.compile(&bytes).map_err(|error| error.to_string())?;

    let compact_counts = if mode_name == "compact" {
        match module.compact_preparation_counts() {
            Some((accepted, declined, attempted, retained)) => format!(
                "{{\"accepted_functions\":{accepted},\"declined_functions\":{declined},\
             \"attempted_work\":{attempted},\"retained_bytes\":{retained}}}"
            ),
            None => "null".to_owned(),
        }
    } else {
        "null".to_owned()
    };

    let mut cache_state = "not_requested";
    let cache_snapshot = if mode_name == "register-adaptive-3" {
        if let Some(snapshot) = module.register_cache_snapshot() {
            cache_state = "empty";
            if !snapshot.entries.is_empty()
                || snapshot.active_body.is_some()
                || snapshot.lifecycle.preparation_starts != 0
                || snapshot.lifecycle.builder_starts != 0
            {
                return Err(
                    "compile-only adaptive cache was not empty before guest entry".to_owned(),
                );
            }
            format!(
                "{{\"state_after_compile\":\"empty\",\"threshold\":{},\
             \"entry_limit\":{},\"entry_count\":{},\"active_body\":null,\
             \"retained_bytes\":{},\"byte_limit\":{},\
             \"remaining_attempted_work\":{},\"remaining_retained_work\":{},\
             \"lifecycle\":{{\"preparation_starts\":{},\"builder_starts\":{},\
             \"completed\":{},\"published\":{},\"publication_failures\":{},\
             \"declined\":{},\"exhausted\":{},\"invalid\":{},\"aborted\":{},\
             \"evictions\":{}}}}}",
                snapshot.threshold,
                snapshot.entry_limit,
                snapshot.entries.len(),
                snapshot.retained_bytes,
                snapshot.byte_limit,
                snapshot.remaining_attempted_work,
                snapshot.remaining_retained_work,
                snapshot.lifecycle.preparation_starts,
                snapshot.lifecycle.builder_starts,
                snapshot.lifecycle.completed,
                snapshot.lifecycle.published,
                snapshot.lifecycle.publication_failures,
                snapshot.lifecycle.declined,
                snapshot.lifecycle.exhausted,
                snapshot.lifecycle.invalid,
                snapshot.lifecycle.aborted,
                snapshot.lifecycle.evictions,
            )
        } else {
            cache_state = "unavailable";
            "null".to_owned()
        }
    } else {
        "null".to_owned()
    };

    let profile_json = json_string(profile_name);
    let mode_json = json_string(mode_name);
    let path_json = json_string(&path.to_string_lossy());
    let cache_state_json = json_string(cache_state);
    // These are separate explicitly reported runtime counters. They are not
    // added together: their allocation scopes overlap and are not a total RSS.
    let output = format!(
        "{{\"schema\":\"wtime-module-footprint-v1\",\"status\":\"passed\",\"timing_evidence\":false,\"instrumented\":true,\"complete_heap_census\":false,\
         \"profile\":{profile_json},\"mode\":{mode_json},\"input\":{path_json},\
         \"wasm_bytes\":{},\"source_bytes\":{},\"lowered_control_bytes\":{},\
         \"function_plan_bytes\":{},\"guest_invocations\":0,\
         \"compact_preparation_counts\":{compact_counts},\
         \"register_cache_state\":{cache_state_json},\
         \"register_cache_snapshot\":{cache_snapshot}}}",
        bytes.len(),
        module.source_bytes(),
        module.lowered_control_bytes(),
        module.function_plan_bytes(),
    );
    Ok(output)
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() != 3 {
        eprintln!("usage: wtime-footprint-inspect FILE PROFILE MODE");
        eprintln!("profiles: mvp | stable-core | all-phase3-core");
        eprintln!("modes: decoded | compact | register-eager | register-adaptive-3");
        return ExitCode::FAILURE;
    }
    match run(&args[0], &args[1], &args[2]) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("footprint inspection failed: {error}");
            ExitCode::FAILURE
        }
    }
}
