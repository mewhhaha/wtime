#![forbid(unsafe_code)]
use std::{env, fs, process::ExitCode};
use wtime_runtime::{Config, Engine, External, Linker, RunOutcome, Store, Value};
use wtime_types::FeatureSet;

fn value(s: &str) -> Result<Value, String> {
    let (kind, data) = s.split_once(':').unwrap_or(("i32", s));
    Ok(match kind {
        "i32" => Value::I32(data.parse().map_err(|_| "invalid i32 argument")?),
        "i64" => Value::I64(data.parse().map_err(|_| "invalid i64 argument")?),
        "f32" => Value::F32(
            data.parse::<f32>()
                .map_err(|_| "invalid f32 argument")?
                .to_bits(),
        ),
        "f64" => Value::F64(
            data.parse::<f64>()
                .map_err(|_| "invalid f64 argument")?
                .to_bits(),
        ),
        _ => return Err(format!("unknown value type {kind}")),
    })
}
fn run() -> Result<(), String> {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        println!(
            "wtime-cli [--profile mvp|stable-core|all-phase3-core] [--fuel N] <check|exports|hints|run> MODULE.wasm [EXPORT [TYPE:VALUE ...]]"
        );
        return Ok(());
    }
    let mut config = Config::default();
    while args.first().is_some_and(|a| a.starts_with("--")) {
        let option = args.remove(0);
        if args.is_empty() {
            return Err(format!("missing value for {option}"));
        }
        let argument = args.remove(0);
        match option.as_str() {
            "--profile" => {
                config.features = match argument.as_str() {
                    "mvp" => FeatureSet::mvp(),
                    "stable-core" => FeatureSet::stable_core(),
                    "all-phase3-core" => FeatureSet::all_phase3_core(),
                    _ => return Err(format!("unknown profile {argument}")),
                }
            }
            "--fuel" => {
                config.limits.fuel = Some(argument.parse().map_err(|_| "invalid fuel budget")?)
            }
            _ => return Err(format!("unknown option {option}")),
        }
    }
    if args.len() < 2 {
        return Err("expected command and module path; see --help".into());
    }
    let command = args.remove(0);
    if !matches!(command.as_str(), "check" | "exports" | "hints" | "run") {
        return Err(format!("unknown command {command}"));
    }
    let path = args.remove(0);
    let bytes = fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let engine = Engine::new(config).map_err(|e| e.to_string())?;
    let module = engine.compile(&bytes).map_err(|e| e.to_string())?;
    if command == "check" {
        if !args.is_empty() {
            return Err("unexpected argument to check".into());
        }
        println!("valid");
        return Ok(());
    }
    if command == "hints" {
        if !args.is_empty() {
            return Err("unexpected argument to hints".into());
        }
        let inspection = module.branch_hints();
        for hint in inspection.hints {
            println!(
                "function={} offset={} {:?}",
                hint.function, hint.instruction_offset, hint.likelihood
            );
        }
        for diagnostic in inspection.diagnostics {
            eprintln!("ignored optional metadata: {diagnostic}");
        }
        return Ok(());
    }
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .map_err(|e| e.to_string())?;
    if command == "exports" {
        if !args.is_empty() {
            return Err("unexpected argument to exports".into());
        }
        let mut exports = instance.exports(&store).map_err(|e| e.to_string())?;
        exports.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, external) in exports {
            println!("{name}\t{external:?}");
        }
        return Ok(());
    }
    if args.is_empty() {
        return Err("run requires an export name".into());
    }
    let name = args.remove(0);
    let External::Func(function) = instance.export(&store, &name).map_err(|e| e.to_string())?
    else {
        return Err(format!("{name} is not a function"));
    };
    let arguments = args
        .iter()
        .map(|s| value(s))
        .collect::<Result<Vec<_>, _>>()?;
    match store
        .invoke(function, &arguments)
        .map_err(|e| e.to_string())?
    {
        RunOutcome::Returned(results) => {
            for result in results {
                println!("{result:?}");
            }
        }
        RunOutcome::Exception(exception) => {
            return Err(format!("uncaught guest exception: {exception:?}"));
        }
        RunOutcome::Suspended { tag, payload, .. } => {
            return Err(format!("guest suspended at {tag:?} with {payload:?}"));
        }
    }
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("wtime: {error}");
            ExitCode::FAILURE
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn t057_argument_types_and_errors() {
        assert_eq!(value("i32:-8"), Ok(Value::I32(-8)));
        assert_eq!(value("i64:9223372036854775807"), Ok(Value::I64(i64::MAX)));
        assert_eq!(value("f32:-0"), Ok(Value::F32(0x80000000)));
        assert!(value("i32:2147483648").is_err());
        assert!(value("ref:0").is_err());
    }
}
