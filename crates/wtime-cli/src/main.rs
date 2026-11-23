#![forbid(unsafe_code)]
use std::{env, fs::File, io::Read, process::ExitCode};
use wtime_runtime::{Config, Engine, External, Linker, RunOutcome, Store, Value};
use wtime_types::{Error, ErrorKind, FeatureSet};

const HELP: &str = "\
wtime-cli [--profile mvp|stable-core|all-phase3-core] [--fuel N] [--] <COMMAND> MODULE.wasm [ARGS...]

Commands:
  check   MODULE.wasm                      Validate a binary WebAssembly module.
  exports MODULE.wasm                      List declared exports (metadata-only; does not instantiate).
  hints   MODULE.wasm                      Inspect branch-hinting custom metadata.
  run     MODULE.wasm EXPORT [TYPE:VAL...] Instantiate and invoke an exported function.

Options:
  --profile PROFILE  Feature profile: mvp, stable-core (default), or all-phase3-core.
  --fuel N           Instruction fuel budget (default: unlimited).
  -h, --help         Print this help message.
  -V, --version      Print version information.

Exit codes:
  0  Success
  1  CLI usage, I/O, or host/link error
  2  Malformed, invalid, unsupported, or disabled-feature module
  3  Guest trap
  4  Uncaught guest exception or suspension
  5  Resource limit or cancellation";

struct CliError {
    code: u8,
    message: String,
}

impl CliError {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            code: 1,
            message: message.into(),
        }
    }
    fn from_wtime(error: Error) -> Self {
        let code = match error.kind {
            ErrorKind::Decode | ErrorKind::Validation | ErrorKind::DisabledFeature => 2,
            ErrorKind::Trap => 3,
            ErrorKind::ResourceLimit => 5,
            ErrorKind::Link | ErrorKind::Host => 1,
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

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

fn format_value(value: Value) -> String {
    match value {
        Value::I32(v) => format!("I32({v})"),
        Value::I64(v) => format!("I64({v})"),
        Value::F32(bits) => {
            let f = f32::from_bits(bits);
            format!("F32({f} [0x{bits:08x}])")
        }
        Value::F64(bits) => {
            let f = f64::from_bits(bits);
            format!("F64({f} [0x{bits:016x}])")
        }
        other => format!("{other:?}"),
    }
}

fn read_bounded(path: &str, max_bytes: usize) -> Result<Vec<u8>, CliError> {
    let file = File::open(path).map_err(|e| CliError::usage(format!("{path}: {e}")))?;
    if let Ok(meta) = file.metadata()
        && meta.is_file()
        && meta.len() > max_bytes as u64
    {
        return Err(CliError::from_wtime(Error::new(
            ErrorKind::ResourceLimit,
            format!("module exceeds max_module_bytes ({max_bytes})"),
        )));
    }
    let limit = (max_bytes as u64).saturating_add(1);
    let mut bytes = Vec::new();
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|e| CliError::usage(format!("{path}: {e}")))?;
    if bytes.len() > max_bytes {
        return Err(CliError::from_wtime(Error::new(
            ErrorKind::ResourceLimit,
            format!("module exceeds max_module_bytes ({max_bytes})"),
        )));
    }
    Ok(bytes)
}

enum Action {
    Help,
    Version,
    Command {
        config: Box<Config>,
        command: String,
        path: String,
        export: Option<String>,
        arguments: Vec<Value>,
    },
}

fn parse_args(raw: Vec<String>) -> Result<Action, CliError> {
    if raw.is_empty() {
        return Ok(Action::Help);
    }
    let mut config = Config::default();
    let mut positional = Vec::new();
    let mut idx = 0usize;
    let mut after_sep = false;
    while idx < raw.len() {
        let arg = &raw[idx];
        if !after_sep && arg == "--" {
            after_sep = true;
            idx += 1;
            continue;
        }
        if !after_sep && matches!(arg.as_str(), "-h" | "--help") {
            return Ok(Action::Help);
        }
        if !after_sep && matches!(arg.as_str(), "-V" | "--version") {
            return Ok(Action::Version);
        }
        if !after_sep && positional.is_empty() && arg.starts_with('-') {
            match arg.as_str() {
                "--profile" => {
                    let Some(argument) = raw.get(idx + 1) else {
                        return Err(CliError::usage("missing value for --profile"));
                    };
                    config.features = match argument.as_str() {
                        "mvp" => FeatureSet::mvp(),
                        "stable-core" => FeatureSet::stable_core(),
                        "all-phase3-core" => FeatureSet::all_phase3_core(),
                        _ => return Err(CliError::usage(format!("unknown profile {argument}"))),
                    };
                    idx += 2;
                    continue;
                }
                "--fuel" => {
                    let Some(argument) = raw.get(idx + 1) else {
                        return Err(CliError::usage("missing value for --fuel"));
                    };
                    config.limits.fuel = Some(
                        argument
                            .parse()
                            .map_err(|_| CliError::usage("invalid fuel budget"))?,
                    );
                    idx += 2;
                    continue;
                }
                _ => return Err(CliError::usage(format!("unknown option {arg}"))),
            }
        }
        positional.push(arg.clone());
        idx += 1;
    }

    if positional.len() < 2 {
        return Err(CliError::usage(
            "expected command and module path; see --help",
        ));
    }
    let mut iter = positional.into_iter();
    let command = iter.next().unwrap();
    if !matches!(command.as_str(), "check" | "exports" | "hints" | "run") {
        return Err(CliError::usage(format!("unknown command {command}")));
    }
    let path = iter.next().unwrap();
    let rest: Vec<String> = iter.collect();
    let (export, arguments) = match command.as_str() {
        "check" | "exports" | "hints" => {
            if !rest.is_empty() {
                return Err(CliError::usage(format!("unexpected argument to {command}")));
            }
            (None, Vec::new())
        }
        "run" => {
            let mut rest_iter = rest.into_iter();
            let export = rest_iter
                .next()
                .ok_or_else(|| CliError::usage("run requires an export name"))?;
            let arguments = rest_iter
                .map(|s| value(&s).map_err(CliError::usage))
                .collect::<Result<Vec<_>, _>>()?;
            (Some(export), arguments)
        }
        _ => unreachable!(),
    };
    Ok(Action::Command {
        config: Box::new(config),
        command,
        path,
        export,
        arguments,
    })
}

fn run_args(raw: Vec<String>) -> Result<(), CliError> {
    let (config, command, path, export, arguments) = match parse_args(raw)? {
        Action::Help => {
            println!("{HELP}");
            return Ok(());
        }
        Action::Version => {
            println!("wtime-cli {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Action::Command {
            config,
            command,
            path,
            export,
            arguments,
        } => (*config, command, path, export, arguments),
    };

    let max_bytes = config.limits.max_module_bytes;
    let engine = Engine::new(config).map_err(CliError::from_wtime)?;
    let bytes = read_bounded(&path, max_bytes)?;
    let module = engine.compile(&bytes).map_err(CliError::from_wtime)?;

    if command == "check" {
        println!("valid");
        return Ok(());
    }
    if command == "hints" {
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
    if command == "exports" {
        let mut exports = module.exports();
        exports.sort_by(|a, b| a.0.cmp(b.0));
        for (name, kind, index) in exports {
            println!("{name}\t{kind:?}({index})");
        }
        return Ok(());
    }

    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .map_err(CliError::from_wtime)?;
    let name = export.expect("validated by parse_args");
    let External::Func(function) = instance
        .export(&store, &name)
        .map_err(CliError::from_wtime)?
    else {
        return Err(CliError::usage(format!("{name} is not a function")));
    };
    match store
        .invoke(function, &arguments)
        .map_err(CliError::from_wtime)?
    {
        RunOutcome::Returned(results) => {
            for result in results {
                println!("{}", format_value(result));
            }
        }
        RunOutcome::Exception(exception) => {
            return Err(CliError {
                code: 4,
                message: format!("uncaught guest exception: {exception:?}"),
            });
        }
        RunOutcome::Suspended { tag, payload, .. } => {
            return Err(CliError {
                code: 4,
                message: format!("guest suspended at {tag:?} with {payload:?}"),
            });
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run_args(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("wtime-cli: {}", error.message);
            ExitCode::from(error.code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn t057_argument_types_and_errors() {
        assert_eq!(value("i32:-8"), Ok(Value::I32(-8)));
        assert_eq!(value("i64:9223372036854775807"), Ok(Value::I64(i64::MAX)));
        assert_eq!(value("f32:-0"), Ok(Value::F32(0x80000000)));
        assert!(value("i32:2147483648").is_err());
        assert!(value("ref:0").is_err());
    }

    #[test]
    fn cli_flags_help_version_formatting_and_exit_codes() {
        assert!(matches!(
            parse_args(vec!["--version".into()]),
            Ok(Action::Version)
        ));
        assert!(matches!(
            parse_args(vec!["check".into(), "--help".into()]),
            Ok(Action::Help)
        ));
        let parsed = parse_args(vec![
            "--fuel".into(),
            "100".into(),
            "--".into(),
            "run".into(),
            "-weird.wasm".into(),
            "f".into(),
            "-7".into(),
        ])
        .ok();
        assert!(matches!(
            parsed,
            Some(Action::Command {
                path,
                arguments,
                ..
            }) if path == "-weird.wasm" && arguments == vec![Value::I32(-7)]
        ));
        assert_eq!(format_value(Value::I32(-5)), "I32(-5)");
        assert_eq!(
            format_value(Value::F32(1.5f32.to_bits())),
            "F32(1.5 [0x3fc00000])"
        );
        assert_eq!(
            CliError::from_wtime(Error::new(ErrorKind::Validation, "bad")).code,
            2
        );
        assert_eq!(
            CliError::from_wtime(Error::new(ErrorKind::Trap, "div0")).code,
            3
        );
        assert_eq!(
            CliError::from_wtime(Error::new(ErrorKind::ResourceLimit, "oom")).code,
            5
        );
    }

    #[test]
    fn exports_is_metadata_only_and_read_bounded_enforces_limit() {
        // Module with an unlinked import AND a trapping start function, plus export "f".
        let wasm: Vec<u8> = vec![
            0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic + version
            0x01, 0x04, 0x01, 0x60, 0x00, 0x00, // type 0: () -> ()
            0x02, 0x07, 0x01, 0x01, b'm', 0x01, b'i', 0x00, 0x00, // import "m"."i" (func 0)
            0x03, 0x02, 0x01, 0x00, // func 1: type 0
            0x07, 0x05, 0x01, 0x01, b'f', 0x00, 0x01, // export "f" -> func 1
            0x08, 0x01, 0x01, // start func 1
            0x0a, 0x05, 0x01, 0x03, 0x00, 0x00, 0x0b, // func 1: unreachable; end
        ];
        let dir = std::env::temp_dir().join(format!("wtime-cli-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mod_path = dir.join("trap_start.wasm");
        fs::write(&mod_path, &wasm).unwrap();
        // Metadata-only exports must succeed even though import is unlinked and start traps.
        run_args(vec!["exports".into(), mod_path.to_str().unwrap().into()])
            .ok()
            .expect("exports must not instantiate or run start");
        // Bounded read rejects oversized files with ResourceLimit exit code 5.
        let err = read_bounded(mod_path.to_str().unwrap(), 8).err().unwrap();
        assert_eq!(err.code, 5);
        let _ = fs::remove_dir_all(&dir);
    }
}
