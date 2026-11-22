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
//! caller's environment apply; the target CPU (`--target-cpu`, default
//! native) goes through CARGO_ENCODED_RUSTFLAGS, above any configuration.
//! `--time-limit` bounds the build. The generated crates have no build scripts
//! and no dependencies other than the runtime crate.
//!
//! `--train ARGUMENTS` makes the build profile-guided: an instrumented
//! program runs once with ARGUMENTS (the program's own flags, then `--` and
//! the guest's arguments, chosen by the operator, never the uploader), in
//! the same sandbox as any run, and rustc then optimizes with the profile
//! it wrote (`-C profile-use`, merged by the toolchain's `llvm-profdata`).
//! The profile steers layout and inlining only, never semantics; the
//! instrumented and the final program are cached, the final one under a
//! key that includes the profile.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::sha256::Sha256;

const USAGE: &str = "usage: wtime-aot build module.wasm --cache DIR [--toolchain NAME] [--module-path NAME] [--rt PATH] [--jobs N] [--time-limit SECONDS] [--target-cpu NAME] [--blot-cdylib] [--backend rust|c] [--c-opt auto|O2|O1|O0] [--train PROGRAM-ARGUMENTS...]";

/// Codegen/compiler backend used by `wtime-aot build`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Rust,
    C,
}

pub fn main(arguments: Vec<String>, default_rt: PathBuf) -> Result<(), String> {
    let mut arguments = arguments.into_iter();
    let (mut module, mut cache, mut toolchain, mut module_path, mut rt, mut jobs) =
        (None, None, String::from("stable"), None, default_rt, None);
    let mut time_limit = std::time::Duration::from_secs(3600);
    // The program runs where it is built: by default for this machine's CPU,
    // as a JIT would compile.
    let mut target_cpu = String::from("native");
    let mut target = crate::codegen::Target::GooseBin;
    let mut backend = Backend::Rust;
    let mut c_opt = crate::cgen::COpt::Auto;
    let mut train = None;
    while let Some(argument) = arguments.next() {
        let mut value = || arguments.next().ok_or_else(|| format!("{USAGE}\n{argument} needs a value"));
        match argument.as_str() {
            "--cache" => cache = Some(PathBuf::from(value()?)),
            "--toolchain" => toolchain = value()?,
            "--module-path" => module_path = Some(value()?),
            "--rt" => rt = PathBuf::from(value()?),
            "--jobs" => jobs = Some(value()?.parse::<u32>().map_err(|_| format!("{USAGE}\n--jobs takes a number"))?),
            "--target-cpu" => {
                target_cpu = value()?;
                if !target_cpu.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.') {
                    return Err(format!("{USAGE}\n--target-cpu takes a CPU name"));
                }
            }
            "--time-limit" => {
                let seconds = value()?.parse::<f64>().ok().filter(|s| *s > 0.0);
                let limit = seconds.and_then(|s| std::time::Duration::try_from_secs_f64(s).ok());
                time_limit = limit.ok_or_else(|| format!("{USAGE}\n--time-limit takes a positive number of seconds"))?;
            }
            "--blot-cdylib" => target = crate::codegen::Target::BlotCdylib,
            "--backend" => {
                backend = match value()?.as_str() {
                    "rust" => Backend::Rust,
                    "c" => Backend::C,
                    other => return Err(format!("{USAGE}\ninvalid --backend {other:?}")),
                };
            }
            "--c-opt" => c_opt = crate::cgen::COpt::parse(&value()?)?,
            // Everything after --train is the training run's command line.
            "--train" => {
                train = Some(arguments.by_ref().collect::<Vec<_>>());
            }
            _ if module.is_none() && !argument.starts_with('-') => module = Some(PathBuf::from(argument)),
            other => return Err(format!("{USAGE}\nunrecognized argument {other}")),
        }
    }
    let (Some(module), Some(cache)) = (module, cache) else { return Err(USAGE.into()) };
    if backend == Backend::C && target != crate::codegen::Target::BlotCdylib {
        return Err("--backend c currently requires --blot-cdylib".into());
    }
    if backend == Backend::C && train.is_some() {
        return Err("--train is not supported with --backend c".into());
    }
    let options = Options { toolchain: &toolchain, target_cpu: &target_cpu, jobs, time_limit, target, backend, c_opt };
    let program = match train {
        None => build(&module, &cache, module_path.as_deref(), &rt, &options)?,
        Some(train) => build_trained(&module, &cache, module_path.as_deref(), &rt, &options, &train)?,
    };
    println!("{}", program.display());
    Ok(())
}

/// How programs are compiled.
pub struct Options<'a> {
    pub toolchain: &'a str,
    pub target_cpu: &'a str,
    pub jobs: Option<u32>,
    pub time_limit: std::time::Duration,
    pub target: crate::codegen::Target,
    pub backend: Backend,
    pub c_opt: crate::cgen::COpt,
}

/// What a build starts from: the Wasm, its validated settings and the key
/// of everything the program depends on.
struct Prepared {
    wasm: Vec<u8>,
    module_path: String,
    cache: PathBuf,
    rt: PathBuf,
    flat64: bool,
    key: Sha256,
    record: String,
}

/// Builds (or finds in `cache`) the program for `module`; its path.
pub fn build(module: &Path, cache: &Path, module_path: Option<&str>, rt: &Path, options: &Options) -> Result<PathBuf, String> {
    if options.backend == Backend::C {
        return build_c(module, cache, options);
    }
    let prepared = prepare(module, cache, module_path, rt, options)?;
    cached(&prepared, prepared.key.clone(), &[], options, "")
}

/// Enforces an owner-controlled (`0o700`), symlink-free private directory on Unix.
/// Fails closed if `path` is a symlink, any ancestor is group/world-writable without the sticky bit,
/// `chmod(0o700)` fails (e.g. `EPERM` when owned by another UID), or non-owner permission bits remain.
fn ensure_private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let sym_meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if sym_meta.file_type().is_symlink() || !sym_meta.file_type().is_dir() {
            return Err(format!("{}: cache path must be a real directory (not a symlink)", path.display()));
        }
        // Propagate `set_permissions` error: on Unix `chmod` fails with `EPERM` if not owned by euid.
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("{}: cannot enforce owner-only 0700 cache permissions: {e}", path.display()))?;
        let post = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if post.file_type().is_symlink() || !post.file_type().is_dir() {
            return Err(format!("{}: cache directory changed type during validation", path.display()));
        }
        if post.permissions().mode() & 0o077 != 0 {
            return Err(format!("{}: cache directory must be owner-only (0700)", path.display()));
        }
        let canon = path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?;
        for anc in canon.ancestors().skip(1) {
            let m = fs::metadata(anc)
                .map_err(|e| format!("{}: {e}", anc.display()))?
                .permissions()
                .mode();
            // Reject non-sticky group/world-writable ancestors that would allow directory replacement.
            if (m & 0o022) != 0 && (m & 0o1000) == 0 {
                return Err(format!(
                    "{}: cache ancestor {} is writable without sticky bit",
                    path.display(),
                    anc.display()
                ));
            }
        }
    }
    Ok(())
}

/// Marks a newly staged native artifact immutable (`0o555`) before atomic publication on Unix.
fn mark_artifact_immutable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o555))
            .map_err(|e| format!("{}: cannot mark artifact read-only: {e}", path.display()))?;
    }
    Ok(())
}

/// Atomically writes `digest_file` containing `sha` (propagating all errors).
fn publish_digest_sidecar(entry: &Path, artifact_name: &str, sha: &str, digest_file: &Path) -> Result<(), String> {
    let staged_sha = entry.join(format!("{artifact_name}.sha256.{}", std::process::id()));
    if let Err(e) = fs::write(&staged_sha, format!("{sha}\n")) {
        let _ = fs::remove_file(&staged_sha);
        return Err(format!("{}: {e}", staged_sha.display()));
    }
    if let Err(e) = fs::rename(&staged_sha, digest_file) {
        let _ = fs::remove_file(&staged_sha);
        return Err(format!("{}: {e}", digest_file.display()));
    }
    Ok(())
}

/// Acquires an exclusive publication lock on `entry/.publish.lock`.
fn lock_cache_entry(entry: &Path) -> Result<fs::File, String> {
    ensure_private_dir(entry)?;
    let lock_path = entry.join(".publish.lock");
    let lock_file = fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| format!("{}: {e}", lock_path.display()))?;
    lock_file.lock().map_err(|e| format!("locking {}: {e}", lock_path.display()))?;
    Ok(lock_file)
}

/// Verifies `artifact` exists and matches its `.sha256` sidecar.
fn verified_cached_artifact(artifact: &Path, digest_file: &Path) -> bool {
    artifact.is_file()
        && fs::read_to_string(digest_file).ok().is_some_and(|expected| {
            fs::read(artifact).ok().is_some_and(|bytes| crate::sha256::hex(&bytes) == expected.trim())
        })
}

/// Fast C99 + parallel `clang` + `ld.lld` build path for Blot ABI 8 cdylibs.
fn build_c(module: &Path, cache: &Path, options: &Options) -> Result<PathBuf, String> {
    let wasm = fs::read(module).map_err(|e| format!("{}: {e}", module.display()))?;
    ensure_private_dir(cache)?;
    let cache = cache.canonicalize().map_err(|e| format!("{}: {e}", cache.display()))?;
    let generator = std::env::current_exe().map_err(|e| format!("this generator: {e}"))?;
    let gen_bytes = fs::read(&generator).map_err(|e| format!("{}: {e}", generator.display()))?;
    let (cc, cc_ver, _) = crate::cgen::detect_cc()?;
    let cc_target = crate::cgen::cc_target_fingerprint(&cc, options.target_cpu)?;

    let mut key = Sha256::new();
    field(&mut key, "wtime-aot build c 2", &[]);
    field(&mut key, "wasm", &wasm);
    field(&mut key, "generator", &gen_bytes);
    field(&mut key, "cc", cc_ver.as_bytes());
    field(&mut key, "target cpu", options.target_cpu.as_bytes());
    field(&mut key, "cc target fingerprint", cc_target.as_bytes());
    field(&mut key, "c opt", options.c_opt.tag().as_bytes());
    let key_hex = key.hex();

    let entry = cache.join(&key_hex);
    let _lock = lock_cache_entry(&entry)?;
    let program = entry.join("library.so");
    let digest_file = entry.join("library.so.sha256");
    if verified_cached_artifact(&program, &digest_file) {
        return Ok(program);
    }
    let _ = fs::remove_file(&program);
    let _ = fs::remove_file(&digest_file);
    let staged = entry.join(format!("library.so.{}", std::process::id()));
    let res = crate::cgen::build_c_blot_cdylib(&wasm, &cache, options.target_cpu, options.jobs, options.c_opt, &staged);
    match res {
        Ok(()) => {
            let so_bytes = fs::read(&staged).map_err(|e| format!("{}: {e}", staged.display()))?;
            let so_sha = crate::sha256::hex(&so_bytes);
            if let Err(e) = mark_artifact_immutable(&staged)
                .and_then(|()| fs::rename(&staged, &program).map_err(|e| format!("{}: {e}", program.display())))
                .and_then(|()| publish_digest_sidecar(&entry, "library.so", &so_sha, &digest_file))
            {
                let _ = fs::remove_file(&staged);
                let _ = fs::remove_file(&program);
                let _ = fs::remove_file(&digest_file);
                return Err(e);
            }
            let _ = fs::write(
                entry.join("key.txt"),
                format!(
                    "key {key_hex}\nbackend c\nc opt {}\ncc {cc_ver}\ntarget cpu {}\ncc target {cc_target}\nwasm sha256 {}\nso sha256 {so_sha}\n",
                    options.c_opt.tag(),
                    options.target_cpu,
                    crate::sha256::hex(&wasm)
                ),
            );
            Ok(program)
        }
        Err(e) => {
            let _ = fs::remove_file(&staged);
            Err(e)
        }
    }
}

/// Builds (or finds in `cache`) the program for `module`, optimized with the
/// profile of a run of it with `train`; its path.
pub fn build_trained(
    module: &Path,
    cache: &Path,
    module_path: Option<&str>,
    rt: &Path,
    options: &Options,
    train: &[String],
) -> Result<PathBuf, String> {
    let prepared = prepare(module, cache, module_path, rt, options)?;
    let mut key = prepared.key.clone();
    field(&mut key, "profile-generate", &[]);
    let instrumented = cached(&prepared, key, &["-Cprofile-generate".into()], options, "instrumented for profiling\n")?;

    // The training run, in a directory of its own for the raw profiles.
    let profiles = prepared.cache.join("work").join(format!("profile-{}", std::process::id()));
    let _ = fs::remove_dir_all(&profiles);
    fs::create_dir_all(&profiles).map_err(|e| format!("{}: {e}", profiles.display()))?;
    let result = train_and_merge(&instrumented, train, &profiles, options);
    let profile = result.and_then(|merged| fs::read(&merged).map_err(|e| format!("{}: {e}", merged.display())));
    let profile = match profile {
        Ok(profile) => profile,
        Err(error) => {
            let _ = fs::remove_dir_all(&profiles);
            return Err(error);
        }
    };
    let mut key = prepared.key.clone();
    field(&mut key, "profile-use", &profile);
    let digest = crate::sha256::hex(&profile);
    let stored = prepared.cache.join("work").join(format!("profile-{digest}-{}.profdata", std::process::id()));
    fs::write(&stored, &profile).map_err(|e| format!("{}: {e}", stored.display()))?;
    let _ = fs::remove_dir_all(&profiles);
    let flag = format!("-Cprofile-use={}", stored.display());
    let note = format!("profile-guided: profile sha256 {digest}, training arguments {train:?}\n");
    let result = cached(&prepared, key, &[flag], options, &note);
    let _ = fs::remove_file(&stored);
    result
}

fn field(key: &mut Sha256, name: &str, bytes: &[u8]) {
    key.update(name.as_bytes());
    key.update(&(bytes.len() as u64).to_le_bytes());
    key.update(bytes);
}

/// Reads and checks what a build starts from.
fn prepare(module: &Path, cache: &Path, module_path: Option<&str>, rt: &Path, options: &Options) -> Result<Prepared, String> {
    let read = |path: &Path| fs::read(path).map_err(|e| format!("{}: {e}", path.display()));
    let wasm = read(module)?;
    // The program reports the module's own name, not where the operator
    // keeps it.
    let module_path = match module_path {
        Some(path) => path.to_string(),
        None => module.file_name().map_or("module.wasm".into(), |name| name.to_string_lossy().into_owned()),
    };
    ensure_private_dir(cache)?;
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
    field(&mut key, "wtime-aot build 1", &[]);
    field(&mut key, "wasm", &wasm);
    field(&mut key, "module path", module_path.as_bytes());
    let generator = std::env::current_exe().map_err(|e| format!("this generator: {e}"))?;
    field(&mut key, "generator", &read(&generator)?);
    for (name, bytes) in runtime_sources(&rt)? {
        field(&mut key, &name, &bytes);
    }
    let version = rustc_version(options.toolchain)?;
    field(&mut key, "toolchain", version.as_bytes());
    // The CPU and the features it resolves to here: a program built for one
    // machine's CPU is not reused for another's.
    field(&mut key, "target cpu", options.target_cpu.as_bytes());
    field(&mut key, "target features", target_features(options.toolchain, options.target_cpu)?.as_bytes());
    // A 64-bit memory is flat (allocated whole at its maximum, accessed
    // without a slow path) when this machine grants its maximum.
    let flat64 = match crate::codegen::memory64_max(&wasm)? {
        Some(pages) => usize::try_from(pages.min(1 << 20).saturating_mul(65536)).is_ok_and(|bytes| {
            let mut probe: Vec<u8> = Vec::new();
            probe.try_reserve_exact(bytes).is_ok()
        }),
        None => false,
    };
    field(&mut key, "flat memory64", &[flat64 as u8]);
    let target_tag = match options.target {
        crate::codegen::Target::GooseBin => "goose-bin",
        crate::codegen::Target::BlotCdylib => "blot-cdylib",
    };
    field(&mut key, "target", target_tag.as_bytes());
    let record = format!(
        "wasm sha256 {}\nmodule path {module_path:?}\ngenerator {}\nruntime {}\ntoolchain {}\ntarget cpu {}\ntarget {target_tag}\nflat memory64 {flat64}\n",
        crate::sha256::hex(&wasm),
        generator.display(),
        rt.display(),
        version.trim().replace('\n', "; "),
        options.target_cpu,
    );
    Ok(Prepared { wasm, module_path, cache, rt, flat64, key, record })
}

/// The program cached under `key`, built with `rustflags` if not there yet.
fn cached(prepared: &Prepared, key: Sha256, rustflags: &[String], options: &Options, note: &str) -> Result<PathBuf, String> {
    let key = key.hex();
    let cache = &prepared.cache;
    let entry = cache.join(&key);
    let _lock = lock_cache_entry(&entry)?;
    let artifact_name = match options.target {
        crate::codegen::Target::GooseBin => "program",
        crate::codegen::Target::BlotCdylib => "library.so",
    };
    let program = entry.join(artifact_name);
    let digest_file = entry.join(format!("{artifact_name}.sha256"));
    if verified_cached_artifact(&program, &digest_file) {
        return Ok(program);
    }
    let _ = fs::remove_file(&program);
    let _ = fs::remove_file(&digest_file);

    // Generate and compile in a directory of this build's own, target
    // directory included, then move the program into place: builds of the
    // same or of different modules share nothing while they run.
    let name = format!("aot-{key}");
    let work = cache.join("work").join(format!("{key}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    let result = generate_and_compile(prepared, &work, &name, rustflags, options).and_then(|built| {
        let staged = entry.join(format!("{artifact_name}.{}", std::process::id()));
        fs::copy(&built, &staged).map_err(|e| format!("{}: {e}", built.display()))?;
        let bytes = fs::read(&staged).map_err(|e| format!("{}: {e}", staged.display()))?;
        let sha = crate::sha256::hex(&bytes);
        if let Err(e) = mark_artifact_immutable(&staged)
            .and_then(|()| fs::rename(&staged, &program).map_err(|e| format!("{}: {e}", program.display())))
            .and_then(|()| publish_digest_sidecar(&entry, artifact_name, &sha, &digest_file))
        {
            let _ = fs::remove_file(&staged);
            let _ = fs::remove_file(&program);
            let _ = fs::remove_file(&digest_file);
            return Err(e);
        }
        Ok(sha)
    });
    let _ = fs::remove_dir_all(&work);
    let sha = result?;
    let _ = fs::write(entry.join("key.txt"), format!("key {key}\n{}artifact sha256 {sha}\n{note}", prepared.record));
    Ok(program)
}

/// Runs the instrumented program with `train` and merges the profile it
/// writes into `profiles`; the merged profile's path.
fn train_and_merge(instrumented: &Path, train: &[String], profiles: &Path, options: &Options) -> Result<PathBuf, String> {
    // The program's own sandbox applies: it reaches files and variables
    // only as `train` grants. Its output is not needed.
    let mut run = Command::new(instrumented);
    run.env_clear()
        .env("LLVM_PROFILE_FILE", profiles.join("%m-%p.profraw"))
        .args(train)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    std::os::unix::process::CommandExt::process_group(&mut run, 0);
    wait(run, "the training run", options.time_limit)?;
    let raw: Vec<PathBuf> = fs::read_dir(profiles)
        .map_err(|e| format!("{}: {e}", profiles.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|e| e == "profraw"))
        .collect();
    if raw.is_empty() {
        return Err("the training run wrote no profile".into());
    }
    let sysroot = Command::new("rustc")
        .env("RUSTUP_TOOLCHAIN", options.toolchain)
        .args(["--print", "sysroot"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("rustc: {e}"))?;
    let sysroot = PathBuf::from(String::from_utf8_lossy(&sysroot.stdout).trim());
    let host = rustc_version(options.toolchain)?
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_string))
        .ok_or("rustc -vV names no host")?;
    let profdata = sysroot.join("lib").join("rustlib").join(host).join("bin").join("llvm-profdata");
    if !profdata.is_file() {
        return Err(format!(
            "{} is missing: install it with `rustup component add llvm-tools --toolchain {}`",
            profdata.display(),
            options.toolchain
        ));
    }
    let merged = profiles.join("merged.profdata");
    let mut merge = Command::new(&profdata);
    merge.arg("merge").arg("-o").arg(&merged).args(&raw).stdin(Stdio::null()).stdout(Stdio::null());
    std::os::unix::process::CommandExt::process_group(&mut merge, 0);
    wait(merge, "llvm-profdata", options.time_limit)?;
    Ok(merged)
}

/// Runs `command` in a process group of its own, which the time limit ends
/// whole; an error unless it succeeds, except that a training run may end
/// with any exit code (a guest's own failure still profiles the code it
/// ran).
fn wait(mut command: Command, what: &str, time_limit: std::time::Duration) -> Result<(), String> {
    let mut child = command.spawn().map_err(|e| format!("{what}: {e}"))?;
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| format!("{what}: {e}"))? {
            break status;
        }
        if started.elapsed() > time_limit {
            let group = format!("-{}", child.id());
            let _ = Command::new("kill").args(["-KILL", "--", &group]).status();
            let _ = child.wait();
            return Err(format!("{what} exceeded its time limit of {time_limit:?}"));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    if status.code().is_none() {
        return Err(format!("{what} ended by a signal ({status})"));
    }
    if what != "the training run" && !status.success() {
        return Err(format!("{what} failed ({status})"));
    }
    Ok(())
}

/// Generates the crates into `work` and compiles them there with
/// `rustflags` added; the program.
fn generate_and_compile(prepared: &Prepared, work: &Path, name: &str, rustflags: &[String], options: &Options) -> Result<PathBuf, String> {
    let Options { toolchain, target_cpu, jobs, time_limit, target: target_kind, .. } = *options;
    crate::codegen::generate_target(&prepared.wasm, work, name, &prepared.rt, &prepared.module_path, prepared.flat64, target_kind)?;
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
    // The target CPU through CARGO_ENCODED_RUSTFLAGS, which takes
    // precedence over every rustflags setting in Cargo's configuration.
    let mut flags = vec![format!("-Ctarget-cpu={target_cpu}")];
    flags.extend(rustflags.iter().cloned());
    cargo.env("CARGO_ENCODED_RUSTFLAGS", flags.join("\u{1f}"));
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
    let out_file = match target_kind {
        crate::codegen::Target::GooseBin => name.to_string(),
        crate::codegen::Target::BlotCdylib => format!("lib{}.so", name.replace('-', "_")),
    };
    Ok(target.join("release").join(out_file))
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

/// The target features `target_cpu` resolves to on this machine
/// (`rustc --print cfg`).
fn target_features(toolchain: &str, target_cpu: &str) -> Result<String, String> {
    let output = Command::new("rustc")
        .env("RUSTUP_TOOLCHAIN", toolchain)
        .args(["--print", "cfg", "-C", &format!("target-cpu={target_cpu}")])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("rustc: {e}"))?;
    if !output.status.success() {
        return Err(format!("target cpu {target_cpu}: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    let mut features = String::from_utf8_lossy(&output.stdout).into_owned();
    // `--print cfg` lists stable features only: the CPU LLVM resolves
    // `native` to covers the rest.
    let cpus = Command::new("rustc")
        .env("RUSTUP_TOOLCHAIN", toolchain)
        .args(["--print", "target-cpus", "-C", &format!("target-cpu={target_cpu}")])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("rustc: {e}"))?;
    let listing = String::from_utf8_lossy(&cpus.stdout);
    if let Some(current) = listing.lines().find(|line| line.contains("currently")) {
        features.push_str(current.trim());
    }
    Ok(features)
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
