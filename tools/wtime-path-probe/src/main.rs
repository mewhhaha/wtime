#![forbid(unsafe_code)]
//! Untimed execution-path observations; kept outside the timing harness/workspace.
use wtime_runtime::{Config, Engine, ErrorKind, ExecutionPolicy, External, FeatureSet,
    Func, FuncType, IndexType, Instance, Limits, Linker, Memory, MemoryType,
    RunOutcome, Store, ValType, Value, V128};

fn features(text: &str) -> Result<FeatureSet, String> {
    if let Some(profile) = text.strip_prefix("profile:") {
        return match profile {
            "mvp" => Ok(FeatureSet::mvp()),
            "stable-core" => Ok(FeatureSet::stable_core()),
            "all-phase3-core" => Ok(FeatureSet::all_phase3_core()),
            "mvp-saturating" | "simd" | "simd-relaxed" => {
                let mut value = FeatureSet::mvp();
                value.saturating_float_to_int = true;
                value.simd = profile != "mvp-saturating";
                value.relaxed_simd = profile == "simd-relaxed";
                Ok(value)
            },
            _ => Err(format!("unknown feature profile {profile}")),
        };
    }
    let mut features = FeatureSet::mvp();
    for name in text.split(',') {
        macro_rules! flags {
            ($($field:ident),*) => { match name {
                "mvp" => {},
                $(stringify!($field) => features.$field = true,)*
                _ => return Err(format!("unknown feature {name}")),
            }};
        }
        flags!(mutable_globals, saturating_float_to_int, sign_extension, multi_value,
            reference_types, bulk_memory, simd, tail_call, extended_const,
            function_references, gc, multi_memory, relaxed_simd, exception_handling,
            memory64, threads, compact_imports, wide_arithmetic, stack_switching,
            custom_page_sizes, custom_descriptors, branch_hinting);
    }
    Ok(features)
}

fn values(text: &str) -> Result<Vec<Value>, String> {
    if text == "-" { return Ok(Vec::new()); }
    text.split(';').map(|item| {
        let (kind, bits) = item.split_once(':').ok_or("typed input needs type:value")?;
        Ok(match kind {
            "i32" => Value::I32(bits.parse().map_err(|_| "invalid i32")?),
            "i64" => Value::I64(bits.parse().map_err(|_| "invalid i64")?),
            "f32_bits" => Value::F32(bits.parse().map_err(|_| "invalid f32 bits")?),
            "f64_bits" => Value::F64(bits.parse().map_err(|_| "invalid f64 bits")?),
            "v128_bits" => {
                if bits.len() != 32 || !bits.is_ascii() { return Err("invalid v128 bits".into()); }
                let mut raw = [0;16];
                for (index, byte) in raw.iter_mut().enumerate() {
                    *byte = u8::from_str_radix(&bits[index*2..index*2+2],16).map_err(|_| "invalid v128 hex")?;
                }
                Value::V128(V128(raw))
            },
            _ => return Err("unsupported typed input".into()),
        })
    }).collect()
}

fn outcome(result: wtime_runtime::Result<RunOutcome>) -> Result<String, String> {
    match result {
        Ok(RunOutcome::Returned(values)) => values.into_iter().map(|value| Ok(match value {
            Value::I32(value) => format!("i32:{value}"),
            Value::I64(value) => format!("i64:{value}"),
            Value::F32(value) => format!("f32_bits:{value}"),
            Value::F64(value) => format!("f64_bits:{value}"),
            Value::V128(value) => format!("v128_bits:{}", value.0.iter().map(|byte| format!("{byte:02x}")).collect::<String>()),
            Value::Ref(_) => return Err("reference result needs a semantic guest oracle".into()),
        })).collect::<Result<Vec<_>,String>>().map(|values| if values.is_empty() { "-".into() } else { values.join(";") }),
        Ok(_) => Err("unexpected suspension".into()),
        Err(error) if error.kind == ErrorKind::Trap => Ok(format!("trap:{:?}",error.trap_code.ok_or("trap code missing")?)),
        Err(error) => Err(format!("unexpected runtime error: {error}")),
    }
}

fn function(store: &Store, instance: Instance, name: &str) -> Result<Func, String> {
    match instance.export(store,name).map_err(|error| error.to_string())? {
        External::Func(function) => Ok(function),
        _ => Err(format!("{name} is not a function")),
    }
}

fn memory(store: &Store, instance: Instance, name: &str) -> Result<Memory, String> {
    match instance.export(store, name).map_err(|error| error.to_string())? {
        External::Memory(memory) => Ok(memory),
        _ => Err(format!("{name} is not a memory")),
    }
}

fn seed(store: &mut Store, memory: Memory, pages: usize, source: bool) -> Result<(), String> {
    let (multiplier, addend, page_multiplier) = if source { (29u8, 7u8, 53u8) } else { (13, 101, 37) };
    let bytes = (0..pages * 65536).map(|index| (index as u8).wrapping_mul(multiplier)
        .wrapping_add(addend).wrapping_add(((index >> 16) as u8).wrapping_mul(page_multiplier))).collect::<Vec<_>>();
    store.write_memory(memory, 0, &bytes).map_err(|error| error.to_string())
}

fn setup(engine: &Engine, store: &mut Store, module: &wtime_runtime::Module,
         driver: &str, arguments: &[Value]) -> Result<Instance, String> {
    let mut linker = Linker::new(engine);
    if driver == "host_tick" {
        let callback = store.create_host_func(FuncType::new(vec![ValType::I32], vec![ValType::I32]), |args| {
            match args {
                [Value::I32(value)] => Ok(vec![Value::I32(value.wrapping_add(1))]),
                _ => Err(wtime_runtime::Error::new(ErrorKind::Host, "host.tick expects i32")),
            }
        }).map_err(|error| error.to_string())?;
        linker.define("host", "tick", External::Func(callback)).map_err(|error| error.to_string())?;
    }
    if driver == "alias" {
        let memory = store.create_memory(MemoryType { limits: Limits::new(17, Some(17)),
            index_type: IndexType::I32, shared: false, page_size_log2: 16 }).map_err(|error| error.to_string())?;
        for name in ["left", "right"] {
            linker.define("mem", name, External::Memory(memory)).map_err(|error| error.to_string())?;
        }
    }
    let instance = linker.instantiate(store, module).map_err(|error| error.to_string())?;
    if driver == "table_a" || driver == "representative" {
        let name = if driver == "table_a" { "set_target" } else { "prepare" };
        let function = function(store, instance, name)?;
        let table_args = [Value::I32(0)];
        let args = if driver == "table_a" { &table_args[..] } else { arguments };
        if outcome(store.invoke(function, args))? != "-" { return Err("setup returned values".into()); }
    }
    if driver == "seeded-copy" {
        let source = memory(store, instance, "source")?;
        seed(store, source, 16, true)?;
    }
    if matches!(driver, "seeded-copy" | "seeded-fill") {
        let destination = memory(store, instance, "memory")?;
        seed(store, destination, 17, false)?;
    }
    Ok(instance)
}

fn run(args: &[String]) -> Result<String, String> {
    if args.len() != 8 {
        return Err("usage: FILE FEATURES eager|adaptive THRESHOLD INPUTS EXPECTED DRIVER REPEATS".into());
    }
    let threshold: u64 = args[3].parse().map_err(|_| "invalid threshold")?;
    let repeats: usize = args[7].parse().map_err(|_| "invalid repeats")?;
    let driver = args[6].as_str();
    if !matches!(driver, "plain" | "host_tick" | "table_a" | "representative" | "alias" | "multi" | "seeded-copy" | "seeded-fill" | "compile_only") {
        return Err("unknown setup driver".into());
    }
    let compile_only = driver == "compile_only";
    let features = features(&args[1])?;
    let config = Config { features, ..Config::default() };
    let engine = match args[2].as_str() {
        "eager" if threshold == 0 && repeats == if compile_only { 0 } else { 1 } => Engine::with_execution_policy(config,ExecutionPolicy::Register),
        "adaptive" if threshold > 0 && threshold <= 64 && repeats == if compile_only { 0 } else { threshold as usize + 1 } => Engine::with_register_entry_threshold(config,threshold),
        _ => return Err("eager requires 0/1; adaptive requires threshold 1..64 and repeats=threshold+1".into()),
    }.map_err(|error| error.to_string())?;
    let arguments = values(&args[4])?;
    let module = engine.compile(&std::fs::read(&args[0]).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    let mut rows = Vec::new();
    for entry in 1..=repeats {
        let mut store = Store::new(&engine);
        let instance = setup(&engine, &mut store, &module, driver, &arguments)?;
        let run = function(&store,instance,"run")?;
        store.reset_register_activation_snapshot().map_err(|error| error.to_string())?;
        let before = (store.register_dispatches(),store.register_exits(),store.register_fallback_frames());
        let result = outcome(store.invoke(run,&arguments))?;
        if result != args[5] && !(args[5] == "trap" && result.starts_with("trap:")) {
            return Err(format!("oracle mismatch: expected {}, got {result}",args[5]));
        }
        let dispatches = store.register_dispatches().checked_sub(before.0).ok_or("dispatch count decreased")?;
        let exits = store.register_exits().checked_sub(before.1).ok_or("exit count decreased")?;
        let fallback = store.register_fallback_frames().checked_sub(before.2).ok_or("fallback count decreased")?;
        let snapshot = store.register_activation_snapshot();
        if snapshot.current_bytes != 0 || snapshot.current_frames != 0 || snapshot.active_scopes != 0 || snapshot.overflowed {
            return Err("activation scope did not clean up".into());
        }
        let class = if exits > 0 { "prefix_exit" }
            else if dispatches > 0 && fallback > 0 { "mixed" }
            else if dispatches > 0 { "optimized" } else if fallback > 0 { "whole_function_fallback" }
            else { return Err("no execution path was observed".into()); };
        let code = module.register_cache_snapshot().map(|cache| cache.retained_bytes.to_string()).unwrap_or_else(|| "null".into());
        rows.push(format!("{{\"entry\":{entry},\"result\":{result:?},\"register_dispatches\":{dispatches},\"register_exits\":{exits},\"fallback_frames\":{fallback},\"classification\":{class:?},\"working_peak_bytes\":{},\"working_peak_frames\":{},\"capacity_samples\":{},\"cache_retained_bytes\":{code},\"active_scopes_after\":0}}",snapshot.peak_bytes,snapshot.peak_frames,snapshot.capacity_samples));
    }
    Ok(format!("{{\"schema\":\"wtime-workload-path-v1\",\"instrumented\":true,\"timing_evidence\":false,\"compiled\":true,\"compile_only\":{compile_only},\"strategy\":{:?},\"threshold\":{threshold},\"invocations\":[{}]}}",args[2],rows.join(",")))
}

fn main() {
    match run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(output) => println!("{output}"),
        Err(error) => { eprintln!("path probe failed: {error}"); std::process::exit(2); },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_values_preserve_float_bits_and_signed_integer_boundaries() {
        assert_eq!(values("i32:-2147483648;i64:9223372036854775807;f32_bits:2147483648;f64_bits:9223372036854775808").unwrap(),
            vec![Value::I32(i32::MIN),Value::I64(i64::MAX),Value::F32(0x80000000),Value::F64(0x8000000000000000)]);
    }
    #[test]
    fn malformed_values_and_unknown_feature_bits_fail_closed() {
        for value in ["i32:2147483648","v128_bits:no","f64_bits:-1","ref:unknown",""] { assert!(values(value).is_err()); }
        assert!(features("mvp,unknown").is_err());
        assert!(features("mvp,simd").unwrap().simd);
    }
    #[test]
    fn invalid_strategy_and_repetition_fail_before_file_io() {
        for (strategy,threshold,repeats) in [("eager","3","1"),("adaptive","0","1"),("adaptive","3","3"),("reference","0","1")] {
            let args=["missing.wasm","mvp",strategy,threshold,"i32:7","i32:8","plain",repeats].map(str::to_owned);
            assert!(run(&args).unwrap_err().contains("eager requires"));
        }
    }
}
