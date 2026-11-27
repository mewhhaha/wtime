//! wtime-aot: compiles a WebAssembly module ahead of time to a safe Rust
//! crate (compiled by rustc), C99 shared library (compiled by clang/gcc),
//! or directly to a Linux x86-64 ELF64 shared library (`--backend x64`).
//!
//! The module is decoded and validated by wtime itself. The generated code,
//! runtime, and compiler contain `#![forbid(unsafe_code)]`.
//!
//! Not covered: fuel and cancellation. A compiled program runs unmetered.

#![forbid(unsafe_code)]

mod aarch64;
mod build;
mod cgen;
mod codegen;
mod pgo;
mod sha256;
mod x64;

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::exit;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_OUT_SEQ: AtomicU64 = AtomicU64::new(0);

const USAGE: &str = "usage: wtime-aot module.wasm -o out [--name NAME] [--rt PATH] [--module-path PATH] [--flat-memory64] [--blot-cdylib] [--backend rust|c|x64|aarch64] [--guard-pages] [--spectre-mitigations] [--pgo-gen PATH | --pgo-use PATH] [--c-opt auto|O2|O1|O0] [--target-cpu NAME] [--jobs N] [--time-limit SECONDS]\n       wtime-aot build module.wasm --cache DIR [--toolchain NAME] [--module-path NAME] [--rt PATH] [--jobs N] [--time-limit SECONDS] [--target-cpu NAME] [--blot-cdylib] [--backend rust|c|x64|aarch64] [--guard-pages] [--spectre-mitigations] [--pgo-gen PATH | --pgo-use PATH] [--c-opt auto|O2|O1|O0] [--train PROGRAM-ARGUMENTS...]";

fn main() {
    let all: Vec<String> = std::env::args().skip(1).collect();
    if all.first().is_some_and(|first| first == "build") {
        let rt = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("rt");
        match build::main(all[1..].to_vec(), rt) {
            Ok(()) => return,
            Err(build::BuildError::AdmissionRejected(error)) => {
                eprintln!("wtime-aot build: {error}");
                exit(x64::X64_ADMISSION_REJECTED_EXIT_CODE);
            }
            Err(build::BuildError::Usage(error)) => {
                eprintln!("wtime-aot build: {error}");
                exit(2);
            }
            Err(build::BuildError::Failed(error)) => {
                eprintln!("wtime-aot build: {error}");
                exit(1);
            }
        }
    }
    let start_time = Instant::now();
    let mut args = all.into_iter();
    let (mut module, mut out, mut name, mut rt, mut module_path) = (None, None, None, None, None);
    let mut flat64 = false;
    let mut target = codegen::Target::GooseBin;
    let mut backend = build::Backend::Rust;
    let mut guard_pages = false;
    let mut spectre_mitigations = false;
    let mut pgo_gen: Option<PathBuf> = None;
    let mut pgo_use: Option<PathBuf> = None;
    let mut c_opt = cgen::COpt::Auto;
    let mut c_opt_explicit = false;
    let mut target_cpu = String::from("native");
    let mut jobs: Option<u32> = None;
    let mut time_limit = Duration::from_secs(3600);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" => out = args.next().map(PathBuf::from),
            "--name" => name = args.next(),
            "--rt" => rt = args.next().map(PathBuf::from),
            "--module-path" => module_path = args.next(),
            "--flat-memory64" => flat64 = true,
            "--blot-cdylib" => target = codegen::Target::BlotCdylib,
            "--backend" => match args.next().as_deref() {
                Some("rust") => backend = build::Backend::Rust,
                Some("c") => backend = build::Backend::C,
                Some("x64") => backend = build::Backend::X64,
                Some("aarch64") => backend = build::Backend::Aarch64,
                other => {
                    eprintln!("{USAGE}\ninvalid --backend {other:?}");
                    exit(2);
                }
            },
            "--guard-pages" => guard_pages = true,
            "--spectre-mitigations" => spectre_mitigations = true,
            "--pgo-gen" => match args.next() {
                Some(p) if !p.is_empty() => pgo_gen = Some(PathBuf::from(p)),
                _ => {
                    eprintln!("{USAGE}\n--pgo-gen needs a profile output path");
                    exit(2);
                }
            },
            "--pgo-use" => match args.next() {
                Some(p) if !p.is_empty() => pgo_use = Some(PathBuf::from(p)),
                _ => {
                    eprintln!("{USAGE}\n--pgo-use needs a WTPGO1 profile path");
                    exit(2);
                }
            },
            "--c-opt" => match args.next().as_deref().map(cgen::COpt::parse) {
                Some(Ok(v)) => {
                    c_opt = v;
                    c_opt_explicit = true;
                }
                Some(Err(e)) => {
                    eprintln!("{USAGE}\n{e}");
                    exit(2);
                }
                None => {
                    eprintln!("{USAGE}\n--c-opt needs a value");
                    exit(2);
                }
            },
            "--target-cpu" => match args.next() {
                Some(cpu)
                    if !cpu.is_empty()
                        && !cpu.starts_with('-')
                        && cpu.chars().all(|c| {
                            c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'
                        }) =>
                {
                    target_cpu = cpu;
                }
                _ => {
                    eprintln!("{USAGE}\n--target-cpu takes a valid CPU name");
                    exit(2);
                }
            },
            "--jobs" => match args.next().and_then(|v| v.parse::<u32>().ok()) {
                Some(n) => jobs = Some(n),
                None => {
                    eprintln!("{USAGE}\n--jobs takes a number");
                    exit(2);
                }
            },
            "--time-limit" => {
                let parsed = args
                    .next()
                    .and_then(|v| v.parse::<f64>().ok())
                    .filter(|s| *s > 0.0)
                    .and_then(|s| Duration::try_from_secs_f64(s).ok());
                match parsed {
                    Some(d) => time_limit = d,
                    None => {
                        eprintln!("{USAGE}\n--time-limit takes a positive number of seconds");
                        exit(2);
                    }
                }
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                exit(0)
            }
            _ if module.is_none() && !arg.starts_with('-') => module = Some(PathBuf::from(arg)),
            other => {
                eprintln!("{USAGE}\nunrecognized argument {other}");
                exit(2)
            }
        }
    }
    if spectre_mitigations && backend == build::Backend::Rust {
        eprintln!("wtime-aot: {}", build::SPECTRE_RUST_ERROR);
        exit(2);
    }
    if guard_pages && !matches!(backend, build::Backend::X64 | build::Backend::Aarch64) {
        eprintln!(
            "wtime-aot: --guard-pages is only supported with --backend x64 or --backend aarch64"
        );
        exit(2);
    }
    if guard_pages && spectre_mitigations {
        eprintln!("wtime-aot: --guard-pages cannot be combined with --spectre-mitigations");
        exit(2);
    }
    if pgo_gen.is_some() && pgo_use.is_some() {
        eprintln!("wtime-aot: --pgo-gen and --pgo-use cannot be used together");
        exit(2);
    }
    if (pgo_gen.is_some() || pgo_use.is_some())
        && !matches!(backend, build::Backend::X64 | build::Backend::Aarch64)
    {
        eprintln!(
            "wtime-aot: --pgo-gen/--pgo-use are only supported with --backend x64 or --backend aarch64"
        );
        exit(2);
    }
    if let Some(ref pg) = pgo_gen
        && let Err(e) = pgo::validate_pgo_gen_path(pg)
    {
        eprintln!("wtime-aot: {e}");
        exit(2);
    }
    let loaded_pgo_use = match pgo_use {
        Some(ref pu) => match pgo::PgoProfile::load_file(pu) {
            Ok(prof) => Some(prof),
            Err(e) => {
                eprintln!("wtime-aot: {e}");
                exit(1);
            }
        },
        None => None,
    };
    let (Some(module), Some(out)) = (module, out) else {
        eprintln!("{USAGE}");
        exit(2)
    };
    if c_opt_explicit && backend != build::Backend::C {
        eprintln!("wtime-aot: --c-opt is only supported with --backend c");
        exit(2);
    }
    if flat64 && backend != build::Backend::Rust {
        eprintln!("wtime-aot: --flat-memory64 is only supported with --backend rust");
        exit(2);
    }
    let deadline = start_time + time_limit;
    if backend == build::Backend::X64 {
        let x64_target = match target {
            codegen::Target::BlotCdylib => x64::module::X64Target::BlotCdylib,
            codegen::Target::GooseBin => x64::module::X64Target::StandaloneExec,
        };
        let (_profile, isa) = match x64::resolve_x64_isa(&target_cpu) {
            Ok(v) => v,
            Err(error) => {
                eprintln!("wtime-aot: {error}");
                exit(1);
            }
        };
        let bytes = match build::read_bounded_wasm_for_x64(&module, Some(deadline)) {
            Ok(bytes) => bytes,
            Err(build::BuildError::AdmissionRejected(error)) => {
                eprintln!("wtime-aot: {error}");
                exit(x64::X64_ADMISSION_REJECTED_EXIT_CODE);
            }
            Err(build::BuildError::Usage(error)) => {
                eprintln!("wtime-aot: {error}");
                exit(2);
            }
            Err(build::BuildError::Failed(error)) => {
                eprintln!("wtime-aot: {error}");
                exit(1);
            }
        };
        let opts = x64::X64CompileOptions {
            target: x64_target,
            isa,
            jobs: jobs.unwrap_or(0) as usize,
            deadline: Some(deadline),
            guard_pages,
            spectre_mitigations,
            pgo_gen_path: pgo_gen,
            pgo_use: loaded_pgo_use,
            ..Default::default()
        };
        let elf_bytes = match x64::compile_wasm_to_x64_so(&bytes, &opts) {
            Ok(img) => img,
            Err(x64::X64CompileError::AdmissionRejected(error)) => {
                eprintln!("wtime-aot: {error}");
                exit(x64::X64_ADMISSION_REJECTED_EXIT_CODE);
            }
            Err(x64::X64CompileError::Failed(error)) => {
                eprintln!("wtime-aot: {error}");
                exit(1);
            }
        };
        if Instant::now() >= deadline {
            eprintln!("wtime-aot: compilation time limit exceeded");
            exit(1);
        }
        let t_write = Instant::now();
        if let Err(error) = write_output_atomic(&out, &elf_bytes) {
            eprintln!("wtime-aot: {error}");
            exit(1);
        }
        if std::env::var_os("WTIME_AOT_TIMINGS").is_some_and(|v| v == "1") {
            eprintln!(
                "wtime-aot x64 write={:.2}ms total_cli={:.2}ms",
                t_write.elapsed().as_secs_f64() * 1000.0,
                start_time.elapsed().as_secs_f64() * 1000.0,
            );
        }
        return;
    }
    if backend == build::Backend::Aarch64 {
        let a64_target = match target {
            codegen::Target::BlotCdylib => x64::module::X64Target::BlotCdylib,
            codegen::Target::GooseBin => x64::module::X64Target::StandaloneExec,
        };
        let bytes = match build::read_bounded_wasm_for_x64(&module, Some(deadline)) {
            Ok(bytes) => bytes,
            Err(build::BuildError::AdmissionRejected(error)) => {
                eprintln!("wtime-aot: {error}");
                exit(x64::X64_ADMISSION_REJECTED_EXIT_CODE);
            }
            Err(build::BuildError::Usage(error)) => {
                eprintln!("wtime-aot: {error}");
                exit(2);
            }
            Err(build::BuildError::Failed(error)) => {
                eprintln!("wtime-aot: {error}");
                exit(1);
            }
        };
        let opts = aarch64::A64CompileOptions {
            target: a64_target,
            optimize: true,
            jobs: jobs.unwrap_or(0) as usize,
            deadline: Some(deadline),
            guard_pages,
            spectre_mitigations,
            pgo_gen_path: pgo_gen,
            pgo_use: loaded_pgo_use,
            ..Default::default()
        };
        let elf_bytes = match aarch64::compile_wasm_to_aarch64(&bytes, &opts) {
            Ok(img) => img,
            Err(x64::X64CompileError::AdmissionRejected(error)) => {
                eprintln!("wtime-aot: {error}");
                exit(x64::X64_ADMISSION_REJECTED_EXIT_CODE);
            }
            Err(x64::X64CompileError::Failed(error)) => {
                eprintln!("wtime-aot: {error}");
                exit(1);
            }
        };
        if Instant::now() >= deadline {
            eprintln!("wtime-aot: compilation time limit exceeded");
            exit(1);
        }
        if let Err(error) = write_output_atomic(&out, &elf_bytes) {
            eprintln!("wtime-aot: {error}");
            exit(1);
        }
        return;
    }
    let bytes =
        match build::read_bounded_wasm(&module, build::MAX_WASM_BYTES_C_RUST, Some(deadline)) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("{error}");
                exit(1)
            }
        };
    if backend == build::Backend::C {
        if target != codegen::Target::BlotCdylib {
            eprintln!("wtime-aot: --backend c currently requires --blot-cdylib");
            exit(2);
        }
        if let Err(error) =
            cgen::generate_c_with_deadline(&bytes, &out, c_opt, spectre_mitigations, Some(deadline))
        {
            eprintln!("wtime-aot: {error}");
            exit(1);
        }
        return;
    }
    let name = name.unwrap_or_else(|| {
        let stem = module
            .file_stem()
            .map_or("module".into(), |s| s.to_string_lossy().into_owned());
        format!(
            "aot-{}",
            stem.replace(|c: char| !c.is_ascii_alphanumeric(), "-")
        )
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
    if let Err(error) = codegen::generate_target_with_deadline(
        &bytes,
        &out,
        &name,
        &rt,
        &module_path,
        flat64,
        target,
        Some(deadline),
    ) {
        eprintln!("wtime-aot: {error}");
        exit(1)
    }
}

fn write_output_atomic(out: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let parent = match out.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => std::path::Path::new("."),
    };
    let fname = out
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("output.so");
    let seq = NEXT_OUT_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(".{fname}.{}.{seq}.tmp", std::process::id()));
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let write_res = opts
        .open(&tmp)
        .and_then(|mut f| f.write_all(bytes))
        .map_err(|e| format!("{}: {e}", tmp.display()));
    let publish_res = write_res.and_then(|()| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("{}: {e}", tmp.display()))?;
        }
        fs::rename(&tmp, out).map_err(|e| format!("{}: {e}", out.display()))
    });
    if let Err(e) = publish_res {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}
