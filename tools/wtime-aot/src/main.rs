//! wtime-aot: compiles a WebAssembly module ahead of time to a safe Rust
//! crate, which rustc then compiles to native code.
//!
//! The module is decoded and validated by wtime itself. Each function becomes
//! a Rust method: the operand stack becomes typed local variables, blocks and
//! loops become labeled Rust blocks and loops, branches become `break` and
//! `continue` after assigning the target's values, and every trap is an
//! `Err` returned through the call chain. Memory accesses are bounds checked
//! (`wtime_aot_rt::Memory`), calls count their depth so that deep recursion
//! traps instead of overflowing the native stack, and the numeric operators
//! keep Wasm's semantics (division traps, min/max, trapping and saturating
//! truncation, NaN-aware comparisons). The generated code, the runtime and
//! the generator contain no unsafe code.
//!
//! Supported: one memory (32- or 64-bit), function imports served by the
//! Goose host (`wtime_aot_rt::goose`), numeric globals, active data
//! segments, the MVP instruction set without tables, sign extension,
//! saturating truncation, memory.copy and memory.fill, and the SIMD subset
//! that treats v128 as 16 bytes (load, store, const, i64x2 splat and lanes).
//! Anything else is refused when generating.
//!
//! Not covered: fuel and cancellation. A compiled program runs unmetered.

#![forbid(unsafe_code)]

mod codegen;

use std::path::PathBuf;
use std::process::exit;

const USAGE: &str = "usage: wtime-aot module.wasm -o crate-dir [--name NAME] [--rt PATH] [--module-path PATH]";

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut module, mut out, mut name, mut rt, mut module_path) = (None, None, None, None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" => out = args.next().map(PathBuf::from),
            "--name" => name = args.next(),
            "--rt" => rt = args.next().map(PathBuf::from),
            "--module-path" => module_path = args.next(),
            "-h" | "--help" => {
                println!("{USAGE}");
                exit(0)
            }
            _ if module.is_none() => module = Some(PathBuf::from(arg)),
            other => {
                eprintln!("{USAGE}\nunrecognized argument {other}");
                exit(2)
            }
        }
    }
    let (Some(module), Some(out)) = (module, out) else {
        eprintln!("{USAGE}");
        exit(2)
    };
    let name = name.unwrap_or_else(|| {
        let stem = module.file_stem().map_or("module".into(), |s| s.to_string_lossy().into_owned());
        format!("aot-{}", stem.replace(|c: char| !c.is_ascii_alphanumeric(), "-"))
    });
    let rt = rt.unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("rt"));
    // The generated crates sit at different depths: they need the runtime's
    // absolute path.
    let rt = match rt.canonicalize() {
        Ok(rt) => rt,
        Err(error) => {
            eprintln!("{}: {error}", rt.display());
            exit(1)
        }
    };
    let module_path = module_path.unwrap_or_else(|| module.to_string_lossy().into_owned());
    let bytes = match std::fs::read(&module) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("{}: {error}", module.display());
            exit(1)
        }
    };
    if let Err(error) = codegen::generate(&bytes, &out, &name, &rt, &module_path) {
        eprintln!("wtime-aot: {error}");
        exit(1)
    }
}
