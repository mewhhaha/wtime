//! `wtime-aot build`: a module's Wasm in, the path of its compiled program
//! out, through a cache keyed by content.
//!
//! This is the path for modules from untrusted sources. The uploader's only
//! input is the Wasm: it is validated, translated by this generator,
//! compiled against this runtime crate by the operator's toolchain, and
//! cached under the SHA-256 of everything that determines the program (the
//! Wasm, the module path the program reports, this generator's binary, the
//! runtime crate's sources and the toolchain's version). Nothing native or
//! textual from the uploader is compiled: names from the module reach the
//! generated source only escaped, its data only as a data file.
//!
//! Cargo runs offline, with an environment reduced to what locating the
//! toolchain needs, so no RUSTFLAGS, wrappers or registry access from the
//! caller's environment apply. The generated crates have no build scripts
//! and no dependencies other than the runtime crate.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::sha256::Sha256;

const USAGE: &str = "usage: wtime-aot build module.wasm --cache DIR [--toolchain NAME] [--module-path NAME] [--rt PATH] [--jobs N] [--time-limit SECONDS]";

pub fn main(arguments: Vec<String>, default_rt: PathBuf) -> Result<(), String> {
    let mut arguments = arguments.into_iter();
    let (mut module, mut cache, mut toolchain, mut module_path, mut rt, mut jobs) =
        (None, None, String::from("stable"), None, default_rt, None);
    let mut time_limit = std::time::Duration::from_secs(3600);
    while let Some(argument) = arguments.next() {
        let mut value = || arguments.next().ok_or_else(|| format!("{USAGE}\n{argument} needs a value"));
        match argument.as_str() {
            "--cache" => cache = Some(PathBuf::from(value()?)),
            "--toolchain" => toolchain = value()?,
            "--module-path" => module_path = Some(value()?),
            "--rt" => rt = PathBuf::from(value()?),
            "--jobs" => jobs = Some(value()?.parse::<u32>().map_err(|_| format!("{USAGE}\n--jobs takes a number"))?),
            "--time-limit" => {
                let seconds = value()?.parse::<f64>().ok().filter(|s| *s > 0.0);
                let limit = seconds.and_then(|s| std::time::Duration::try_from_secs_f64(s).ok());
                time_limit = limit.ok_or_else(|| format!("{USAGE}\n--time-limit takes a positive number of seconds"))?;
            }
            _ if module.is_none() && !argument.starts_with('-') => module = Some(PathBuf::from(argument)),
            other => return Err(format!("{USAGE}\nunrecognized argument {other}")),
        }
    }
    let (Some(module), Some(cache)) = (module, cache) else { return Err(USAGE.into()) };
    let program = build(&module, &cache, &toolchain, module_path.as_deref(), &rt, jobs, time_limit)?;
    println!("{}", program.display());
    Ok(())
}

/// Builds (or finds in `cache`) the program for `module`; its path.
pub fn build(
    module: &Path,
    cache: &Path,
    toolchain: &str,
    module_path: Option<&str>,
    rt: &Path,
    jobs: Option<u32>,
    time_limit: std::time::Duration,
) -> Result<PathBuf, String> {
    let read = |path: &Path| fs::read(path).map_err(|e| format!("{}: {e}", path.display()));
    let wasm = read(module)?;
    // The program reports the module's own name, not where the operator
    // keeps it.
    let module_path = match module_path {
        Some(path) => path.to_string(),
        None => module.file_name().map_or("module.wasm".into(), |name| name.to_string_lossy().into_owned()),
    };
    fs::create_dir_all(cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let cache = cache.canonicalize().map_err(|e| format!("{}: {e}", cache.display()))?;
    // Cargo reads configuration (which can name a rustc wrapper to run) from
    // every ancestor of the directory it builds in; none may have any.
    for directory in cache.ancestors() {
        for name in ["config", "config.toml"] {
            let config = directory.join(".cargo").join(name);
            if config.exists() {
                return Err(format!(
                    "{} would configure the build; keep the cache where no ancestor has a .cargo configuration",
                    config.display()
                ));
            }
        }
    }
    let rt = rt.canonicalize().map_err(|e| format!("{}: {e}", rt.display()))?;

    // The key: everything the program depends on.
    let mut key = Sha256::new();
    let mut field = |name: &str, bytes: &[u8]| {
        key.update(name.as_bytes());
        key.update(&(bytes.len() as u64).to_le_bytes());
        key.update(bytes);
    };
    field("wtime-aot build 1", &[]);
    field("wasm", &wasm);
    field("module path", module_path.as_bytes());
    let generator = std::env::current_exe().map_err(|e| format!("this generator: {e}"))?;
    field("generator", &read(&generator)?);
    for (name, bytes) in runtime_sources(&rt)? {
        field(&name, &bytes);
    }
    let version = rustc_version(toolchain)?;
    field("toolchain", version.as_bytes());
    let key = key.hex();

    let entry = cache.join(&key);
    let program = entry.join("program");
    if program.is_file() {
        return Ok(program);
    }

    // Generate and compile in a directory of this build's own, target
    // directory included, then move the program into place: builds of the
    // same or of different modules share nothing while they run.
    let name = format!("aot-{key}");
    let work = cache.join("work").join(format!("{key}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    let result = generate_and_compile(&wasm, &work, &name, &rt, &module_path, toolchain, jobs, time_limit)
        .and_then(|built| {
            fs::create_dir_all(&entry).map_err(|e| format!("{}: {e}", entry.display()))?;
            let staged = entry.join(format!("program.{}", std::process::id()));
            fs::copy(&built, &staged).map_err(|e| format!("{}: {e}", built.display()))?;
            fs::rename(&staged, &program).map_err(|e| format!("{}: {e}", program.display()))
        });
    let _ = fs::remove_dir_all(&work);
    result?;
    let record = format!(
        "key {key}\nwasm sha256 {}\nmodule path {module_path:?}\ngenerator {}\nruntime {}\ntoolchain {}\n",
        crate::sha256::hex(&wasm),
        generator.display(),
        rt.display(),
        version.trim().replace('\n', "; ")
    );
    let _ = fs::write(entry.join("key.txt"), record);
    Ok(program)
}

/// Generates the crates into `work` and compiles them there; the program.
#[allow(clippy::too_many_arguments)]
fn generate_and_compile(
    wasm: &[u8],
    work: &Path,
    name: &str,
    rt: &Path,
    module_path: &str,
    toolchain: &str,
    jobs: Option<u32>,
    time_limit: std::time::Duration,
) -> Result<PathBuf, String> {
    crate::codegen::generate(wasm, work, name, rt, module_path)?;
    let target = work.join("target");
    let mut cargo = Command::new("cargo");
    cargo.env_clear().current_dir(work).stdin(Stdio::null());
    for variable in ["PATH", "HOME", "RUSTUP_HOME", "CARGO_HOME"] {
        if let Some(value) = std::env::var_os(variable) {
            cargo.env(variable, value);
        }
    }
    // rustc recurses on nested code: a stack of its own size for that.
    cargo.env("RUSTUP_TOOLCHAIN", toolchain).env("CARGO_TARGET_DIR", &target).env("RUST_MIN_STACK", "268435456");
    cargo.args(["build", "--release", "--offline", "--quiet"]);
    if let Some(jobs) = jobs {
        cargo.args(["--jobs", &jobs.to_string()]);
    }
    // A process group of its own, so that the time limit ends rustc too.
    std::os::unix::process::CommandExt::process_group(&mut cargo, 0);
    let mut child = cargo.spawn().map_err(|e| format!("cargo: {e}"))?;
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| format!("cargo: {e}"))? {
            break status;
        }
        if started.elapsed() > time_limit {
            let group = format!("-{}", child.id());
            let _ = Command::new("kill").args(["-KILL", "--", &group]).status();
            let _ = child.wait();
            return Err(format!("the build exceeded its time limit of {:?}", time_limit));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    if !status.success() {
        return Err(format!("cargo build failed ({status})"));
    }
    Ok(target.join("release").join(name))
}

/// The runtime crate's manifest and sources, by relative path, in order.
fn runtime_sources(rt: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut files = Vec::new();
    let mut pending = vec![rt.join("Cargo.toml"), rt.join("src")];
    while let Some(path) = pending.pop() {
        if path.is_dir() {
            let entries = fs::read_dir(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            for entry in entries {
                pending.push(entry.map_err(|e| format!("{}: {e}", path.display()))?.path());
            }
        } else {
            let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let name = path.strip_prefix(rt).unwrap_or(&path).to_string_lossy().into_owned();
            files.push((name, bytes));
        }
    }
    files.sort();
    Ok(files)
}

/// `rustc -vV` of the toolchain.
fn rustc_version(toolchain: &str) -> Result<String, String> {
    let output = Command::new("rustc")
        .env("RUSTUP_TOOLCHAIN", toolchain)
        .arg("-vV")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("rustc: {e}"))?;
    if !output.status.success() {
        return Err(format!("toolchain {toolchain}: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
