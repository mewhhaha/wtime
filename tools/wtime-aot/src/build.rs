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
//! `--time-limit` bounds the build cooperatively across compilation, locking,
//! subprocesses, and off-thread artifact publication (when OS thread creation is
//! available; if thread creation fails with `EAGAIN`, final publication runs
//! synchronously and rolls back on deadline expiry). Because cache entries are
//! content-addressed and verified against an immutable SHA-256 sidecar written
//! as the sole final commit barrier, a timeout never leaves an unverified or
//! corrupt entry reusable (if an in-flight final sidecar rename completes after
//! timeout while rollback `unlink` is also stalled across process exit, the
//! surviving pair is already complete, immutable, and SHA-256 verified). The
//! generated crates have no build scripts and no dependencies other than the
//! runtime crate.
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
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};

use crate::sha256::Sha256;

static NEXT_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

pub(crate) const MAX_WASM_BYTES_C_RUST: usize = 256 << 20;

const USAGE: &str = "usage: wtime-aot build module.wasm --cache DIR [--toolchain NAME] [--module-path NAME] [--rt PATH] [--jobs N] [--time-limit SECONDS] [--target-cpu NAME] [--blot-cdylib] [--backend rust|c|x64|aarch64] [--spectre-mitigations] [--c-opt auto|O2|O1|O0] [--train PROGRAM-ARGUMENTS...]";

pub(crate) const SPECTRE_RUST_ERROR: &str = "--spectre-mitigations requires hardware speculation barriers, which safe Rust cannot emit; use --backend x64, --backend aarch64, or --backend c";

/// Codegen/compiler backend used by `wtime-aot build`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Rust,
    C,
    X64,
    Aarch64,
}

/// Typed build error distinguishing clean `--backend x64` admission rejections (CLI exit code `4`)
/// from invalid mitigation options (CLI exit code `2`) and other build errors (CLI exit code `1`).
#[derive(Clone, Debug)]
pub enum BuildError {
    AdmissionRejected(String),
    Usage(String),
    Failed(String),
}

impl From<String> for BuildError {
    fn from(s: String) -> Self {
        BuildError::Failed(s)
    }
}

impl From<&str> for BuildError {
    fn from(s: &str) -> Self {
        BuildError::Failed(s.to_string())
    }
}

pub fn main(arguments: Vec<String>, default_rt: PathBuf) -> Result<(), BuildError> {
    let mut arguments = arguments.into_iter();
    let (mut module, mut cache, mut toolchain, mut module_path, mut rt, mut jobs) =
        (None, None, String::from("1.99.0"), None, default_rt, None);
    let mut time_limit = std::time::Duration::from_secs(3600);
    // The program runs where it is built: by default for this machine's CPU,
    // as a JIT would compile.
    let mut target_cpu = String::from("native");
    let mut target = crate::codegen::Target::GooseBin;
    let mut backend = Backend::Rust;
    let mut spectre_mitigations = false;
    let mut c_opt = crate::cgen::COpt::Auto;
    let mut c_opt_explicit = false;
    let mut train = None;
    while let Some(argument) = arguments.next() {
        let mut value = || {
            arguments
                .next()
                .ok_or_else(|| format!("{USAGE}\n{argument} needs a value"))
        };
        match argument.as_str() {
            "--cache" => cache = Some(PathBuf::from(value()?)),
            "--toolchain" => toolchain = value()?,
            "--module-path" => module_path = Some(value()?),
            "--rt" => rt = PathBuf::from(value()?),
            "--jobs" => {
                jobs = Some(
                    value()?
                        .parse::<u32>()
                        .map_err(|_| format!("{USAGE}\n--jobs takes a number"))?,
                )
            }
            "--target-cpu" => {
                target_cpu = value()?;
                if target_cpu.is_empty()
                    || target_cpu.starts_with('-')
                    || !target_cpu
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
                {
                    return Err(format!("{USAGE}\n--target-cpu takes a CPU name").into());
                }
            }
            "--time-limit" => {
                let seconds = value()?.parse::<f64>().ok().filter(|s| *s > 0.0);
                let limit = seconds.and_then(|s| std::time::Duration::try_from_secs_f64(s).ok());
                time_limit = limit.ok_or_else(|| {
                    format!("{USAGE}\n--time-limit takes a positive number of seconds")
                })?;
            }
            "--blot-cdylib" => target = crate::codegen::Target::BlotCdylib,
            "--backend" => {
                backend = match value()?.as_str() {
                    "rust" => Backend::Rust,
                    "c" => Backend::C,
                    "x64" => Backend::X64,
                    "aarch64" => Backend::Aarch64,
                    other => return Err(format!("{USAGE}\ninvalid --backend {other:?}").into()),
                };
            }
            "--spectre-mitigations" => spectre_mitigations = true,
            "--c-opt" => {
                c_opt = crate::cgen::COpt::parse(&value()?)?;
                c_opt_explicit = true;
            }
            // Everything after --train is the training run's command line.
            "--train" => {
                train = Some(arguments.by_ref().collect::<Vec<_>>());
            }
            _ if module.is_none() && !argument.starts_with('-') => {
                module = Some(PathBuf::from(argument))
            }
            other => return Err(format!("{USAGE}\nunrecognized argument {other}").into()),
        }
    }
    if spectre_mitigations && backend == Backend::Rust {
        return Err(BuildError::Usage(SPECTRE_RUST_ERROR.into()));
    }
    let (Some(module), Some(cache)) = (module, cache) else {
        return Err(USAGE.into());
    };
    if backend == Backend::C && target != crate::codegen::Target::BlotCdylib {
        return Err(format!("--backend {:?} currently requires --blot-cdylib", backend).into());
    }
    if c_opt_explicit && backend != Backend::C {
        return Err(format!(
            "--c-opt is only supported with --backend c (got {:?})",
            backend
        )
        .into());
    }
    if matches!(backend, Backend::C | Backend::X64 | Backend::Aarch64) && train.is_some() {
        return Err(format!("--train is not supported with --backend {:?}", backend).into());
    }
    let options = Options {
        toolchain: &toolchain,
        target_cpu: &target_cpu,
        jobs,
        time_limit,
        target,
        backend,
        c_opt,
    };
    let program = match train {
        None => build_classified(
            &module,
            &cache,
            module_path.as_deref(),
            &rt,
            &options,
            spectre_mitigations,
        )?,
        Some(train) => build_trained(
            &module,
            &cache,
            module_path.as_deref(),
            &rt,
            &options,
            &train,
        )?,
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
    rt_files: Vec<(String, Vec<u8>)>,
    flat64: bool,
    key: Sha256,
    record: String,
}

/// Builds (or finds in `cache`) the program for `module`; its path.
#[allow(dead_code)]
pub fn build(
    module: &Path,
    cache: &Path,
    module_path: Option<&str>,
    rt: &Path,
    options: &Options,
) -> Result<PathBuf, String> {
    build_classified(module, cache, module_path, rt, options, false).map_err(|e| match e {
        BuildError::AdmissionRejected(s) | BuildError::Usage(s) | BuildError::Failed(s) => s,
    })
}

fn build_classified(
    module: &Path,
    cache: &Path,
    module_path: Option<&str>,
    rt: &Path,
    options: &Options,
    spectre_mitigations: bool,
) -> Result<PathBuf, BuildError> {
    let deadline = std::time::Instant::now() + options.time_limit;
    if options.backend == Backend::C {
        return Ok(build_c(
            module,
            cache,
            options,
            spectre_mitigations,
            deadline,
        )?);
    }
    if options.backend == Backend::X64 {
        return build_x64(module, cache, options, spectre_mitigations, deadline);
    }
    if options.backend == Backend::Aarch64 {
        return build_aarch64(module, cache, options, spectre_mitigations, deadline);
    }
    let prepared = prepare(module, cache, module_path, rt, options, deadline)?;
    Ok(cached(
        &prepared,
        prepared.key.clone(),
        &[],
        options,
        "",
        deadline,
    )?)
}

fn check_deadline(
    deadline: std::time::Instant,
    time_limit: std::time::Duration,
) -> Result<(), String> {
    if std::time::Instant::now() >= deadline {
        return Err(format!(
            "the build exceeded its time limit of {time_limit:?}"
        ));
    }
    Ok(())
}

/// Opens `path` as a regular file without blocking on FIFOs or special devices.
fn open_regular_file(path: &Path) -> Result<fs::File, String> {
    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::OpenOptionsExt as _;
        #[cfg(target_os = "linux")]
        const O_NONBLOCK_NOCTTY: i32 = 0o4000 | 0o400;
        #[cfg(not(target_os = "linux"))]
        const O_NONBLOCK_NOCTTY: i32 = 0x0004 | 0x20000;
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(O_NONBLOCK_NOCTTY)
            .open(path)
            .map_err(|e| format!("{}: {e}", path.display()))?
    };
    #[cfg(not(unix))]
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let meta = file
        .metadata()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!(
            "{}: input must be a regular file (refusing FIFO, device, or directory)",
            path.display()
        ));
    }
    Ok(file)
}

/// Reads a regular file directly into a bounded `Vec<u8>` in up to 1 MiB chunks without an
/// intermediate stack copy, enforcing `max_bytes` and `deadline`.
fn read_file_bounded_direct(
    mut file: fs::File,
    prealloc: usize,
    max_bytes: usize,
    deadline: Option<std::time::Instant>,
) -> Result<Vec<u8>, BoundedReadErr> {
    const READ_CHUNK: usize = 1 << 20;
    let mut buf = Vec::new();
    if prealloc > 0 {
        buf.try_reserve_exact(prealloc).map_err(|_| {
            BoundedReadErr::Io(std::io::Error::other("out of memory reading input"))
        })?;
        buf.resize(prealloc, 0u8);
    }
    let mut pos = 0usize;
    while pos < buf.len() {
        if let Some(dl) = deadline
            && std::time::Instant::now() >= dl
        {
            return Err(BoundedReadErr::Deadline);
        }
        let end = pos.saturating_add(READ_CHUNK).min(buf.len());
        match file.read(&mut buf[pos..end]) {
            Ok(0) => {
                buf.truncate(pos);
                return Ok(buf);
            }
            Ok(n) => pos += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(BoundedReadErr::Io(e)),
        }
    }
    // Check for EOF or a file that grew past its initial metadata length.
    let mut tail = [0u8; 65_536];
    loop {
        if let Some(dl) = deadline
            && std::time::Instant::now() >= dl
        {
            return Err(BoundedReadErr::Deadline);
        }
        match file.read(&mut tail) {
            Ok(0) => break,
            Ok(n) => {
                if buf.len().saturating_add(n) > max_bytes {
                    return Err(BoundedReadErr::TooLarge);
                }
                buf.extend_from_slice(&tail[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(BoundedReadErr::Io(e)),
        }
    }
    Ok(buf)
}

enum BoundedReadErr {
    Deadline,
    TooLarge,
    Io(std::io::Error),
}

/// Reads a Wasm input file with a hard pre-allocation byte cap (checking initial file
/// metadata, reading directly in 1 MiB chunks, and enforcing `deadline` so growing/stalled inputs stay bounded).
pub(crate) fn read_bounded_wasm(
    path: &Path,
    max_bytes: usize,
    deadline: Option<std::time::Instant>,
) -> Result<Vec<u8>, String> {
    if let Some(dl) = deadline
        && std::time::Instant::now() >= dl
    {
        return Err(format!(
            "{}: exceeded build time limit while reading input",
            path.display()
        ));
    }
    let file = open_regular_file(path)?;
    let mut prealloc = 0usize;
    if let Ok(meta) = file.metadata()
        && meta.is_file()
    {
        if meta.len() > max_bytes as u64 {
            return Err(format!(
                "{}: input size ({} B) exceeds maximum module limit ({max_bytes} B)",
                path.display(),
                meta.len()
            ));
        }
        prealloc = (meta.len() as usize).min(max_bytes);
    }
    read_file_bounded_direct(file, prealloc, max_bytes, deadline).map_err(|e| match e {
        BoundedReadErr::Deadline => format!(
            "{}: exceeded build time limit while reading input",
            path.display()
        ),
        BoundedReadErr::TooLarge => format!(
            "{}: input size exceeds maximum module limit ({max_bytes} B)",
            path.display()
        ),
        BoundedReadErr::Io(io) => format!("{}: {io}", path.display()),
    })
}

/// Reads a Wasm input file under the `--backend x64` 64 MiB cap, classifying oversized modules as
/// clean `BuildError::AdmissionRejected` (exit code `4`) and I/O or deadline errors as `BuildError::Failed`.
pub(crate) fn read_bounded_wasm_for_x64(
    path: &Path,
    deadline: Option<std::time::Instant>,
) -> Result<Vec<u8>, BuildError> {
    let max_bytes = crate::x64::MAX_WASM_BYTES_X64;
    if let Some(dl) = deadline
        && std::time::Instant::now() >= dl
    {
        return Err(BuildError::Failed(format!(
            "{}: exceeded build time limit while reading input",
            path.display()
        )));
    }
    let file = open_regular_file(path).map_err(BuildError::Failed)?;
    let mut prealloc = 0usize;
    if let Ok(meta) = file.metadata()
        && meta.is_file()
    {
        if meta.len() > max_bytes as u64 {
            return Err(BuildError::AdmissionRejected(format!(
                "x64: {}: input size ({} B) exceeds x64 maximum module limit ({max_bytes} B)",
                path.display(),
                meta.len()
            )));
        }
        prealloc = (meta.len() as usize).min(max_bytes);
    }
    read_file_bounded_direct(file, prealloc, max_bytes, deadline).map_err(|e| match e {
        BoundedReadErr::Deadline => BuildError::Failed(format!(
            "{}: exceeded build time limit while reading input",
            path.display()
        )),
        BoundedReadErr::TooLarge => BuildError::AdmissionRejected(format!(
            "x64: {}: input size exceeds x64 maximum module limit ({max_bytes} B)",
            path.display()
        )),
        BoundedReadErr::Io(io) => BuildError::Failed(format!("{}: {io}", path.display())),
    })
}

/// When an outer orchestrator (such as Pen) runs `wtime-aot build` inside a dedicated process
/// group and sets `WTIME_AOT_INHERIT_PGID=1`, keep compiler/linker children in that same process
/// group so a single outer process-group kill terminates the entire build tree atomically.
pub(crate) fn inherit_pgid() -> bool {
    std::env::var_os("WTIME_AOT_INHERIT_PGID").is_some_and(|v| v == "1")
}

#[cfg(unix)]
fn send_sigkill(target: &str) {
    let kill_bin = if Path::new("/bin/kill").is_file() {
        "/bin/kill"
    } else {
        "/usr/bin/kill"
    };
    let _ = Command::new(kill_bin)
        .env_clear()
        .args(["-KILL", "--", target])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(unix)]
pub(crate) fn kill_process_group(pgid: u32) {
    send_sigkill(&format!("-{pgid}"));
}

#[cfg(all(unix, target_os = "linux"))]
fn kill_linux_descendants(root_pid: u32) {
    let mut visited = std::collections::HashSet::new();
    let mut stack = vec![root_pid];
    let mut order = Vec::new();
    while let Some(pid) = stack.pop() {
        if !visited.insert(pid) || visited.len() > 4096 {
            continue;
        }
        let task_dir = format!("/proc/{pid}/task");
        let Ok(entries) = fs::read_dir(&task_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let children_path = entry.path().join("children");
            if let Ok(text) = fs::read_to_string(&children_path) {
                for tok in text.split_whitespace() {
                    if let Ok(cpid) = tok.parse::<u32>() {
                        order.push(cpid);
                        stack.push(cpid);
                    }
                }
            }
        }
    }
    for cpid in order.into_iter().rev() {
        send_sigkill(&cpid.to_string());
    }
}

/// Terminates `child` and its descendant build tree without killing our own process group when
/// `WTIME_AOT_INHERIT_PGID=1` is active.
pub(crate) fn terminate_child_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        if !inherit_pgid() {
            kill_process_group(child.id());
        } else {
            #[cfg(target_os = "linux")]
            kill_linux_descendants(child.id());
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub(crate) struct RemoveFileOnDrop(pub(crate) PathBuf);

impl Drop for RemoveFileOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Creates a unique owner-only (`0o600`, `O_CREAT | O_EXCL`) temporary file for capture/probes.
pub(crate) fn create_private_temp_file(
    dir: Option<&Path>,
    prefix: &str,
    suffix: &str,
) -> Result<(fs::File, PathBuf), String> {
    let base = dir
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    for _ in 0..32 {
        let seq = NEXT_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = base.join(format!(
            ".wtime-aot-{prefix}-{}-{seq}{suffix}",
            std::process::id()
        ));
        let mut opts = fs::OpenOptions::new();
        opts.read(true).write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        match opts.open(&path) {
            Ok(f) => return Ok((f, path)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Err(format!(
        "failed to create unique private temp file under {}",
        base.display()
    ))
}

const MAX_CAPTURE_BYTES: usize = 16 << 20;
const POST_EXIT_DRAIN_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// Returns an absolute-only, empty-entry-free `PATH` so subprocess execution (`cc`, `ld`, `as`,
/// `rustc`, `cargo`, `ld.so`) resolves bare names identically to `find_in_path` with zero CWD lookup.
pub(crate) fn sanitized_path_env() -> Option<std::ffi::OsString> {
    let raw = std::env::var_os("PATH")?;
    let dirs: Vec<PathBuf> = std::env::split_paths(&raw)
        .filter(|d| !d.as_os_str().is_empty() && d.is_absolute())
        .collect();
    if dirs.is_empty() {
        return None;
    }
    std::env::join_paths(dirs).ok()
}

pub(crate) enum PipeCaptureErr {
    Overflow,
    Io(String),
}

pub(crate) fn spawn_bounded_pipe_reader<R: std::io::Read + Send + 'static>(
    mut reader: R,
    limit: usize,
    overflowed: Arc<AtomicBool>,
) -> Result<mpsc::Receiver<Result<Vec<u8>, PipeCaptureErr>>, String> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("wtime-aot-pipe".into())
        .spawn(move || {
            let mut out = Vec::new();
            let mut chunk = [0u8; 8192];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) => {
                        let _ = tx.send(Ok(out));
                        return;
                    }
                    Ok(n) => {
                        let new_len = out.len().saturating_add(n);
                        if new_len > limit {
                            overflowed.store(true, Ordering::Release);
                            drop(reader);
                            let _ = tx.send(Err(PipeCaptureErr::Overflow));
                            return;
                        }
                        if new_len > out.capacity() {
                            // Never overshoot `limit` in allocated capacity.
                            let target = out.capacity().saturating_mul(2).max(new_len).min(limit);
                            out.reserve_exact(target - out.len());
                        }
                        out.extend_from_slice(&chunk[..n]);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        let _ = tx.send(Err(PipeCaptureErr::Io(e.to_string())));
                        return;
                    }
                }
            }
        })
        .map_err(|e| format!("spawning subprocess pipe reader thread: {e}"))?;
    Ok(rx)
}

pub(crate) fn recv_bounded_pipe(
    rx: mpsc::Receiver<Result<Vec<u8>, PipeCaptureErr>>,
    deadline: Option<std::time::Instant>,
    label: &str,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let wait = match deadline {
        Some(dl) => dl
            .checked_duration_since(std::time::Instant::now())
            .ok_or_else(|| format!("{label}: exceeded build time limit"))?
            .min(POST_EXIT_DRAIN_GRACE),
        None => POST_EXIT_DRAIN_GRACE,
    };
    let res = match rx.recv_timeout(wait) {
        Ok(r) => r,
        Err(RecvTimeoutError::Timeout) => {
            if let Some(dl) = deadline
                && std::time::Instant::now() >= dl
            {
                return Err(format!("{label}: exceeded build time limit"));
            }
            return Err(format!(
                "{label}: subprocess output pipe did not close after exit (orphaned descendant holding pipe open)"
            ));
        }
        Err(RecvTimeoutError::Disconnected) => {
            return Err(format!("{label}: subprocess output reader disconnected"));
        }
    };
    match res {
        Ok(buf) => Ok(buf),
        Err(PipeCaptureErr::Overflow) => Err(format!(
            "{label}: exceeded subprocess output cap ({limit} bytes)"
        )),
        Err(PipeCaptureErr::Io(e)) => Err(format!("{label}: reading subprocess output: {e}")),
    }
}

/// Runs `cmd`, capturing stdout/stderr through bounded in-memory pipes (zero disk amplification,
/// hard `MAX_CAPTURE_BYTES` per-stream ceiling with exact `reserve_exact` capacity cap and
/// immediate `SIGPIPE` + tree kill on overflow, concurrent draining preventing pipe deadlocks,
/// and bounded post-exit `recv_timeout` preventing descendant EOF hangs) while propagating every
/// read failure.
pub(crate) fn run_cmd_capture_with_deadline(
    mut cmd: Command,
    label: &str,
    deadline: Option<std::time::Instant>,
) -> Result<std::process::Output, String> {
    if let Some(dl) = deadline
        && std::time::Instant::now() >= dl
    {
        return Err(format!("{label}: exceeded build time limit"));
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    if !inherit_pgid() {
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    }
    let mut child = cmd.spawn().map_err(|e| format!("{label}: {e}"))?;
    let overflowed = Arc::new(AtomicBool::new(false));
    let Some(stdout_pipe) = child.stdout.take() else {
        terminate_child_tree(&mut child);
        return Err(format!("{label}: missing stdout pipe"));
    };
    let Some(stderr_pipe) = child.stderr.take() else {
        terminate_child_tree(&mut child);
        return Err(format!("{label}: missing stderr pipe"));
    };
    let rx_out =
        match spawn_bounded_pipe_reader(stdout_pipe, MAX_CAPTURE_BYTES, Arc::clone(&overflowed)) {
            Ok(rx) => rx,
            Err(e) => {
                terminate_child_tree(&mut child);
                return Err(format!("{label}: {e}"));
            }
        };
    let rx_err =
        match spawn_bounded_pipe_reader(stderr_pipe, MAX_CAPTURE_BYTES, Arc::clone(&overflowed)) {
            Ok(rx) => rx,
            Err(e) => {
                terminate_child_tree(&mut child);
                return Err(format!("{label}: {e}"));
            }
        };
    let status = loop {
        if overflowed.load(Ordering::Acquire) {
            terminate_child_tree(&mut child);
            return Err(format!(
                "{label}: exceeded subprocess output cap ({MAX_CAPTURE_BYTES} bytes)"
            ));
        }
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {
                if let Some(dl) = deadline
                    && std::time::Instant::now() >= dl
                {
                    terminate_child_tree(&mut child);
                    return Err(format!("{label}: exceeded build time limit"));
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(e) => {
                terminate_child_tree(&mut child);
                return Err(format!("{label}: {e}"));
            }
        }
    };
    if overflowed.load(Ordering::Acquire) {
        return Err(format!(
            "{label}: exceeded subprocess output cap ({MAX_CAPTURE_BYTES} bytes)"
        ));
    }
    let stdout = recv_bounded_pipe(rx_out, deadline, label, MAX_CAPTURE_BYTES)?;
    let stderr = recv_bounded_pipe(rx_err, deadline, label, MAX_CAPTURE_BYTES)?;
    if let Some(dl) = deadline
        && std::time::Instant::now() >= dl
    {
        return Err(format!("{label}: exceeded build time limit"));
    }
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// Returns the current effective UID on Unix, failing closed if unavailable.
/// On Linux this reads `/proc/thread-self/status` (falling back to `/proc/self/status` only on
/// `NotFound`) and parses `Uid:` field 2 (`euid`), staying exact for the calling thread even
/// under `PR_SET_DUMPABLE=0` (where `/proc/self` metadata reports `st_uid == 0`).
#[cfg(unix)]
pub(crate) fn current_euid() -> Result<u32, String> {
    #[cfg(target_os = "linux")]
    {
        let status = match fs::read_to_string("/proc/thread-self/status") {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::read_to_string("/proc/self/status")
                    .map_err(|e2| format!("reading /proc/self/status for effective uid: {e2}"))?
            }
            Err(e) => {
                return Err(format!(
                    "reading /proc/thread-self/status for effective uid: {e}"
                ));
            }
        };
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("Uid:") {
                let euid_str = rest
                    .split_whitespace()
                    .nth(1)
                    .ok_or_else(|| "malformed Uid line in proc status".to_string())?;
                return euid_str
                    .parse::<u32>()
                    .map_err(|e| format!("parsing effective uid from proc status: {e}"));
            }
        }
        Err("missing Uid line in proc status".to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let out = Command::new("/usr/bin/id")
            .env_clear()
            .arg("-u")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("running /usr/bin/id -u for effective uid: {e}"))?;
        if !out.status.success() {
            return Err(format!("/usr/bin/id -u failed ({})", out.status));
        }
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<u32>()
            .map_err(|e| format!("parsing effective uid from /usr/bin/id -u: {e}"))
    }
}

#[cfg(unix)]
fn validate_trusted_ancestor(anc: &Path, target: &Path, euid: u32) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let am = fs::symlink_metadata(anc).map_err(|e| format!("{}: {e}", anc.display()))?;
    if am.file_type().is_symlink() || !am.file_type().is_dir() {
        return Err(format!(
            "{}: cache ancestor {} is not a real directory",
            target.display(),
            anc.display()
        ));
    }
    let uid = am.uid();
    if uid != 0 && uid != euid {
        return Err(format!(
            "{}: cache ancestor {} is owned by untrusted uid {uid} (expected 0 or {euid})",
            target.display(),
            anc.display()
        ));
    }
    let m = am.permissions().mode();
    // Reject non-sticky group/world-writable ancestors that would allow directory replacement.
    if (m & 0o022) != 0 && (m & 0o1000) == 0 {
        return Err(format!(
            "{}: cache ancestor {} is writable without sticky bit (remediation: run `chmod 700` or pass `--cache` to an owner-private directory)",
            target.display(),
            anc.display()
        ));
    }
    Ok(())
}

/// Enforces an owner-controlled (`0o700`), symlink-free private directory hierarchy on Unix
/// BEFORE performing any directory creation or `chmod`, returning the verified canonical path.
/// Fails closed if effective UID cannot be determined, `path` contains `..` or targets `/`,
/// traverses a non-root symlink, has untrusted/non-sticky-writable ancestors, targets a foreign-
/// owned or pre-existing group/world-writable directory, or fails `0o700` creation/tightening.
pub(crate) fn ensure_private_dir(path: &Path) -> Result<PathBuf, String> {
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path).map_err(|e| format!("{}: {e}", path.display()))?;
        path.canonicalize()
            .map_err(|e| format!("{}: {e}", path.display()))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        let euid = current_euid()?;
        let abs_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|e| format!("resolving current_dir for {}: {e}", path.display()))?
                .join(path)
        };
        let mut existing = PathBuf::new();
        let mut missing: Vec<std::ffi::OsString> = Vec::new();
        let mut in_missing = false;
        for comp in abs_path.components() {
            match comp {
                Component::CurDir => continue,
                Component::ParentDir => {
                    return Err(format!(
                        "{}: cache path must not contain '..' components",
                        path.display()
                    ));
                }
                Component::RootDir => {
                    if in_missing {
                        return Err(format!("{}: malformed cache path", path.display()));
                    }
                    existing.push(comp.as_os_str());
                }
                Component::Prefix(_) => {
                    return Err(format!("{}: unsupported path prefix", path.display()));
                }
                Component::Normal(part) => {
                    if in_missing {
                        missing.push(part.to_os_string());
                        continue;
                    }
                    let candidate = existing.join(part);
                    match fs::symlink_metadata(&candidate) {
                        Ok(m) => {
                            if m.file_type().is_symlink() {
                                if m.uid() != 0 {
                                    return Err(format!(
                                        "{}: cache hierarchy traverses non-root symlink {}",
                                        path.display(),
                                        candidate.display()
                                    ));
                                }
                            } else if !m.file_type().is_dir() {
                                return Err(format!(
                                    "{}: cache path component {} is not a directory",
                                    path.display(),
                                    candidate.display()
                                ));
                            }
                            existing = candidate;
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            in_missing = true;
                            missing.push(part.to_os_string());
                        }
                        Err(e) => return Err(format!("{}: {e}", candidate.display())),
                    }
                }
            }
        }
        if existing.as_os_str().is_empty() {
            return Err(format!("{}: empty cache path", path.display()));
        }
        let canon_existing = existing
            .canonicalize()
            .map_err(|e| format!("{}: {e}", existing.display()))?;
        if missing.is_empty() {
            if canon_existing.parent().is_none() {
                return Err(format!(
                    "{}: refusing to use filesystem root as cache directory",
                    path.display()
                ));
            }
            let sym = fs::symlink_metadata(&existing)
                .map_err(|e| format!("{}: {e}", existing.display()))?;
            if sym.file_type().is_symlink() || !sym.file_type().is_dir() {
                return Err(format!(
                    "{}: cache path must be a real directory (not a symlink)",
                    path.display()
                ));
            }
            if sym.uid() != euid {
                return Err(format!(
                    "{}: cache directory owner uid {} does not match effective uid {euid}",
                    path.display(),
                    sym.uid()
                ));
            }
            if sym.permissions().mode() & 0o022 != 0 {
                return Err(format!(
                    "{}: refusing pre-existing group/world-writable cache directory (mode {:o})",
                    path.display(),
                    sym.permissions().mode() & 0o7777
                ));
            }
            for anc in canon_existing.ancestors().skip(1) {
                validate_trusted_ancestor(anc, path, euid)?;
            }
            if sym.permissions().mode() & 0o077 != 0 {
                fs::set_permissions(&canon_existing, fs::Permissions::from_mode(0o700)).map_err(
                    |e| {
                        format!(
                            "{}: cannot enforce owner-only 0700 cache permissions: {e}",
                            path.display()
                        )
                    },
                )?;
            }
            let post = fs::symlink_metadata(&canon_existing)
                .map_err(|e| format!("{}: {e}", canon_existing.display()))?;
            if post.file_type().is_symlink()
                || !post.file_type().is_dir()
                || post.uid() != euid
                || post.permissions().mode() & 0o077 != 0
                || post.dev() != sym.dev()
                || post.ino() != sym.ino()
            {
                return Err(format!(
                    "{}: cache directory failed post-validation identity/mode checks",
                    path.display()
                ));
            }
            return Ok(canon_existing);
        }

        // Creating missing tail: validate the entire existing ancestor chain (including `canon_existing`) BEFORE creating anything.
        for anc in canon_existing.ancestors() {
            validate_trusted_ancestor(anc, path, euid)?;
        }
        let mut cur = canon_existing;
        for part in missing {
            cur.push(&part);
            let mut db = fs::DirBuilder::new();
            db.mode(0o700);
            match db.create(&cur) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let pre = fs::symlink_metadata(&cur)
                        .map_err(|e| format!("{}: {e}", cur.display()))?;
                    if pre.file_type().is_symlink()
                        || !pre.file_type().is_dir()
                        || pre.uid() != euid
                        || pre.permissions().mode() & 0o022 != 0
                    {
                        return Err(format!(
                            "{}: concurrently created cache dir failed ownership/mode checks",
                            cur.display()
                        ));
                    }
                    if pre.permissions().mode() & 0o077 != 0 {
                        fs::set_permissions(&cur, fs::Permissions::from_mode(0o700)).map_err(
                            |e| format!("{}: cannot enforce 0700 permissions: {e}", cur.display()),
                        )?;
                    }
                }
                Err(e) => return Err(format!("{}: {e}", cur.display())),
            }
            let post = fs::symlink_metadata(&cur).map_err(|e| format!("{}: {e}", cur.display()))?;
            if post.file_type().is_symlink()
                || !post.file_type().is_dir()
                || post.uid() != euid
                || post.permissions().mode() & 0o077 != 0
            {
                return Err(format!(
                    "{}: created cache directory failed 0700 ownership checks",
                    cur.display()
                ));
            }
        }
        let canon_leaf = cur
            .canonicalize()
            .map_err(|e| format!("{}: {e}", cur.display()))?;
        if canon_leaf != cur {
            return Err(format!(
                "{}: cache directory changed path during creation",
                cur.display()
            ));
        }
        Ok(canon_leaf)
    }
}

/// Marks a newly staged native file immutable (`mode`, e.g. `0o555` for `.so` or `0o444` for `.o`) on Unix.
pub(crate) fn mark_file_immutable(path: &Path, mode: u32) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|e| format!("{}: cannot mark artifact read-only: {e}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

/// Marks a newly staged native artifact immutable (`0o555`) before atomic publication on Unix.
fn mark_artifact_immutable(path: &Path) -> Result<(), String> {
    mark_file_immutable(path, 0o555)
}

/// Atomically writes `digest_file` containing `sha` with owner-only creation (`0o600`) and
/// immutable `0o444` permissions before rename so `umask 0002` never creates group-writable
/// sidecars (propagating all errors).
pub(crate) fn publish_digest_sidecar(
    entry: &Path,
    artifact_name: &str,
    sha: &str,
    digest_file: &Path,
) -> Result<(), String> {
    use std::io::Write as _;
    let seq = NEXT_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let staged_sha = entry.join(format!(
        "{artifact_name}.sha256.{}.{seq}",
        std::process::id()
    ));
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let write_res = opts
        .open(&staged_sha)
        .and_then(|mut f| f.write_all(format!("{sha}\n").as_bytes()))
        .map_err(|e| format!("{}: {e}", staged_sha.display()));
    if let Err(e) = write_res
        .and_then(|()| mark_file_immutable(&staged_sha, 0o444))
        .and_then(|()| {
            fs::rename(&staged_sha, digest_file)
                .map_err(|e| format!("{}: {e}", digest_file.display()))
        })
    {
        let _ = fs::remove_file(&staged_sha);
        return Err(e);
    }
    Ok(())
}

/// RAII guard for an advisory cache lock file whose `Drop` closes the fd off-thread under
/// `deadline` so early error returns, admission rejections, and lock-wait timeouts never block
/// past `--time-limit` on a stalled `close(2)`. Call `release_checked` on success paths to also
/// enforce that lock release completes before `deadline`, or `into_inner` when transferring the
/// underlying `fs::File` to a dedicated off-thread publisher.
pub(crate) struct BoundedLockGuard {
    file: Option<fs::File>,
    deadline: Option<std::time::Instant>,
}

impl BoundedLockGuard {
    /// Transfers ownership of the raw `fs::File` out of the guard (disarming `Drop`).
    fn into_inner(mut self) -> fs::File {
        self.file.take().expect("lock file already consumed")
    }

    /// Releases the lock file off-thread under `self.deadline` and enforces that release completes
    /// before `deadline` (returning a time-limit error otherwise, even under `EAGAIN`).
    fn release_checked(mut self, time_limit: std::time::Duration) -> Result<(), String> {
        let Some(file) = self.file.take() else {
            return Ok(());
        };
        let Some(deadline) = self.deadline else {
            drop(file);
            return Ok(());
        };
        if !drop_file_off_thread(file, deadline) {
            return Err(format!(
                "the build exceeded its time limit of {time_limit:?}"
            ));
        }
        check_deadline(deadline, time_limit)
    }
}

impl Drop for BoundedLockGuard {
    fn drop(&mut self) {
        let Some(file) = self.file.take() else {
            return;
        };
        let Some(deadline) = self.deadline else {
            drop(file);
            return;
        };
        let _ = drop_file_off_thread(file, deadline);
    }
}

/// Drops `file` on a detached helper thread and waits at most until `deadline`, returning `true`
/// if `close(2)` finished before `deadline` and `false` if the wait timed out. On thread-creation
/// failure (`EAGAIN`), falls back to synchronous `drop(file)`.
fn drop_file_off_thread(file: fs::File, deadline: std::time::Instant) -> bool {
    let done = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let done_bg = Arc::clone(&done);
    let file_slot = Arc::new(std::sync::Mutex::new(Some(file)));
    let file_bg = Arc::clone(&file_slot);
    if std::thread::Builder::new()
        .spawn(move || {
            let f = file_bg.lock().unwrap_or_else(|g| g.into_inner()).take();
            drop(f);
            let (lk, cv) = &*done_bg;
            *lk.lock().unwrap_or_else(|g| g.into_inner()) = true;
            cv.notify_all();
        })
        .is_ok()
    {
        let rem = deadline.saturating_duration_since(std::time::Instant::now());
        if rem.is_zero() {
            return false;
        }
        let (lk, cv) = &*done;
        let guard = lk.lock().unwrap_or_else(|g| g.into_inner());
        let (guard, _) = cv
            .wait_timeout_while(guard, rem, |d| !*d)
            .unwrap_or_else(|e| e.into_inner());
        *guard
    } else {
        let f = file_slot.lock().unwrap_or_else(|g| g.into_inner()).take();
        drop(f);
        std::time::Instant::now() < deadline
    }
}

/// Acquires an advisory lock (`exclusive` or shared) on `lock_path` within `deadline`.
fn lock_file_mode_with_deadline(
    lock_path: &Path,
    exclusive: bool,
    deadline: Option<std::time::Instant>,
) -> Result<BoundedLockGuard, String> {
    let lock_file = fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|e| format!("{}: {e}", lock_path.display()))?;
    let guard = BoundedLockGuard {
        file: Some(lock_file),
        deadline,
    };
    loop {
        let f = guard.file.as_ref().expect("lock file present");
        let res = if exclusive {
            f.try_lock()
        } else {
            f.try_lock_shared()
        };
        match res {
            Ok(()) => return Ok(guard),
            Err(fs::TryLockError::WouldBlock) => {
                if let Some(dl) = deadline
                    && std::time::Instant::now() >= dl
                {
                    return Err(format!(
                        "timed out waiting for publication lock {}",
                        lock_path.display()
                    ));
                }
                let sleep_for = match deadline {
                    Some(dl) => dl
                        .saturating_duration_since(std::time::Instant::now())
                        .min(std::time::Duration::from_millis(10)),
                    None => std::time::Duration::from_millis(10),
                };
                if !sleep_for.is_zero() {
                    std::thread::sleep(sleep_for);
                }
            }
            Err(fs::TryLockError::Error(e)) => {
                return Err(format!("locking {}: {e}", lock_path.display()));
            }
        }
    }
}

/// Acquires an exclusive advisory lock on `lock_path` within `deadline`.
pub(crate) fn lock_file_with_deadline(
    lock_path: &Path,
    deadline: Option<std::time::Instant>,
) -> Result<BoundedLockGuard, String> {
    lock_file_mode_with_deadline(lock_path, true, deadline)
}

/// Acquires a shared advisory lock on `lock_path` within `deadline`.
pub(crate) fn lock_file_shared_with_deadline(
    lock_path: &Path,
    deadline: Option<std::time::Instant>,
) -> Result<BoundedLockGuard, String> {
    lock_file_mode_with_deadline(lock_path, false, deadline)
}

/// Acquires an exclusive publication lock on `entry/.publish.lock` within `deadline`, returning
/// the bounded lock guard and the verified canonical entry directory.
fn lock_cache_entry(
    entry: &Path,
    deadline: std::time::Instant,
) -> Result<(BoundedLockGuard, PathBuf), String> {
    let canon_entry = ensure_private_dir(entry)?;
    let lock_path = canon_entry.join(".publish.lock");
    let lock_guard = lock_file_with_deadline(&lock_path, Some(deadline))?;
    Ok((lock_guard, canon_entry))
}

/// Verifies `artifact` is a non-symlink immutable regular file owned by `euid` matching its
/// `.sha256` sidecar, returning the exact verified bytes so callers avoid any post-check TOCTOU.
pub(crate) fn verified_cached_artifact_bytes(
    artifact: &Path,
    digest_file: &Path,
) -> Option<Vec<u8>> {
    let art_meta = fs::symlink_metadata(artifact).ok()?;
    if art_meta.file_type().is_symlink() || !art_meta.file_type().is_file() {
        return None;
    }
    let sha_meta = fs::symlink_metadata(digest_file).ok()?;
    if sha_meta.file_type().is_symlink() || !sha_meta.file_type().is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if art_meta.permissions().mode() & 0o222 != 0 {
            return None;
        }
        if sha_meta.permissions().mode() & 0o022 != 0 {
            return None;
        }
        let euid = current_euid().ok()?;
        if art_meta.uid() != euid || sha_meta.uid() != euid {
            return None;
        }
    }
    let expected = fs::read_to_string(digest_file).ok()?;
    let bytes = fs::read(artifact).ok()?;
    crate::sha256::verify_artifact_sidecar(&bytes, expected.trim()).then_some(bytes)
}

/// Verifies `artifact` is a non-symlink immutable regular file matching its `.sha256` sidecar.
fn verified_cached_artifact(artifact: &Path, digest_file: &Path) -> bool {
    verified_cached_artifact_bytes(artifact, digest_file).is_some()
}

mod generator_fingerprint {
    include!(concat!(env!("OUT_DIR"), "/generator_fingerprint.rs"));
}

/// Fast C99 + parallel `clang` + `ld.lld` build path for Blot ABI 8 cdylibs.
fn build_c(
    module: &Path,
    cache: &Path,
    options: &Options,
    spectre_mitigations: bool,
    deadline: std::time::Instant,
) -> Result<PathBuf, String> {
    check_deadline(deadline, options.time_limit)?;
    let wasm = read_bounded_wasm(module, MAX_WASM_BYTES_C_RUST, Some(deadline))?;
    check_deadline(deadline, options.time_limit)?;
    let cache = ensure_private_dir(cache)?;
    let cc_info =
        crate::cgen::detect_cc_with_deadline(options.target_cpu, Some(&cache), Some(deadline))?;
    let cc_target = crate::cgen::cc_target_fingerprint_with_deadline(
        &cc_info,
        options.target_cpu,
        Some(deadline),
    )?;
    check_deadline(deadline, options.time_limit)?;

    let mut key = Sha256::new();
    field(&mut key, "wtime-aot build c 9", &[]);
    field(
        &mut key,
        "spectre mitigations",
        &[u8::from(spectre_mitigations)],
    );
    field(&mut key, "wasm", &wasm);
    field(
        &mut key,
        "generator build sha256",
        &generator_fingerprint::GENERATOR_BUILD_SHA256,
    );
    field(&mut key, "cc", cc_info.version.as_bytes());
    field(&mut key, "cc sha256", cc_info.cc_sha256.as_bytes());
    field(
        &mut key,
        "sys headers sha256",
        cc_info.sys_headers_sha256.as_bytes(),
    );
    field(&mut key, "linker sha256", cc_info.linker_sha256.as_bytes());
    field(
        &mut key,
        "link env sha256",
        cc_info.link_env_sha256.as_bytes(),
    );
    field(&mut key, "target cpu", options.target_cpu.as_bytes());
    field(&mut key, "cc target fingerprint", cc_target.as_bytes());
    field(&mut key, "cet return", &[cc_info.has_cet_return as u8]);
    field(&mut key, "stack clash", &[cc_info.has_stack_clash as u8]);
    field(&mut key, "c opt", options.c_opt.tag().as_bytes());
    let key_hex = key.hex();

    let (publish_lock, entry) = lock_cache_entry(&cache.join(&key_hex), deadline)?;
    check_deadline(deadline, options.time_limit)?;
    let program = entry.join("library.so");
    let digest_file = entry.join("library.so.sha256");
    if verified_cached_artifact(&program, &digest_file) {
        sweep_stale_staged_artifacts_bounded(&entry, deadline);
        publish_lock.release_checked(options.time_limit)?;
        return Ok(program);
    }
    for stale in [&digest_file, &program] {
        if let Err(e) = fs::remove_file(stale)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(format!("{}: {e}", stale.display()));
        }
    }
    sweep_stale_staged_artifacts_bounded(&entry, deadline);
    check_deadline(deadline, options.time_limit)?;
    let seq = NEXT_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let staged = entry.join(format!("library.so.{}.{seq}", std::process::id()));
    let res = crate::cgen::build_c_blot_cdylib_with_info(
        &wasm,
        &cache,
        options.target_cpu,
        options.jobs,
        options.c_opt,
        spectre_mitigations,
        &cc_info,
        &cc_target,
        Some(deadline),
        &staged,
    );
    match res {
        Ok(()) => {
            check_deadline(deadline, options.time_limit)?;
            // Note: `build_c_blot_cdylib_with_info` already ran `verify_cc_unchanged(&cc_info)`
            // right after linking and before publishing `c-obj/` shards.
            let so_bytes = fs::read(&staged).map_err(|e| format!("{}: {e}", staged.display()))?;
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            if cc_info.has_cet_return && !crate::cgen::elf64_has_x86_shstk_property(&so_bytes) {
                let _ = fs::remove_file(&staged);
                return Err(format!(
                    "{}: linked DSO is missing required GNU_PROPERTY_X86_FEATURE_1_SHSTK property",
                    staged.display()
                ));
            }
            let so_sha = crate::sha256::artifact_sidecar_digest(&so_bytes);
            if let Err(e) = mark_artifact_immutable(&staged)
                .and_then(|()| {
                    fs::rename(&staged, &program).map_err(|e| format!("{}: {e}", program.display()))
                })
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
                    "key {key_hex}\nbackend c\nc opt {}\ncc {}\ncc sha256 {}\nsys headers sha256 {}\nlinker {}\nlinker sha256 {}\nlink env sha256 {}\ncet return {}\nstack clash {}\ntarget cpu {}\ncc target {cc_target}\nwasm sha256 {}\nso sha256 {so_sha}\n",
                    options.c_opt.tag(),
                    cc_info.version,
                    cc_info.cc_sha256,
                    cc_info.sys_headers_sha256,
                    cc_info.canon_linker.display(),
                    cc_info.linker_sha256,
                    cc_info.link_env_sha256,
                    cc_info.has_cet_return,
                    cc_info.has_stack_clash,
                    options.target_cpu,
                    crate::sha256::hex(&wasm)
                ),
            );
            publish_lock.release_checked(options.time_limit)?;
            Ok(program)
        }
        Err(e) => {
            let _ = fs::remove_file(&staged);
            Err(e)
        }
    }
}

/// Pure-Rust Wasm -> Linux AArch64 ELF64 cached build path (`--backend aarch64`).
fn build_aarch64(
    module: &Path,
    cache: &Path,
    options: &Options,
    spectre_mitigations: bool,
    deadline: std::time::Instant,
) -> Result<PathBuf, BuildError> {
    use std::io::Write as _;
    check_deadline(deadline, options.time_limit)?;
    let wasm = read_bounded_wasm_for_x64(module, Some(deadline))?;
    check_deadline(deadline, options.time_limit)?;
    let cache = ensure_private_dir(cache)?;
    let gen_sha = generator_fingerprint::GENERATOR_BUILD_SHA256;
    let wasm_sha = crate::sha256::parallel_merkle_digest(&wasm)
        .unwrap_or_else(|| crate::sha256::digest(&wasm));
    let a64_target = match options.target {
        crate::codegen::Target::BlotCdylib => crate::x64::module::X64Target::BlotCdylib,
        crate::codegen::Target::GooseBin => crate::x64::module::X64Target::StandaloneExec,
    };
    let artifact_name = match options.target {
        crate::codegen::Target::BlotCdylib => "library.so",
        crate::codegen::Target::GooseBin => "program",
    };
    let digest_name = match options.target {
        crate::codegen::Target::BlotCdylib => "library.so.sha256",
        crate::codegen::Target::GooseBin => "program.sha256",
    };
    let mut key = Sha256::new();
    field(&mut key, "wtime-aot build aarch64 2", &[]);
    field(
        &mut key,
        "spectre mitigations",
        &[u8::from(spectre_mitigations)],
    );
    field(&mut key, "target", artifact_name.as_bytes());
    field(&mut key, "wasm digest", &wasm_sha);
    field(&mut key, "generator build sha256", &gen_sha);
    let key_hex = key.hex();

    let (publish_lock, entry) = lock_cache_entry(&cache.join(&key_hex), deadline)?;
    check_deadline(deadline, options.time_limit)?;
    let program = entry.join(artifact_name);
    let digest_file = entry.join(digest_name);
    if verified_cached_artifact(&program, &digest_file) {
        sweep_stale_staged_artifacts_bounded(&entry, deadline);
        publish_lock.release_checked(options.time_limit)?;
        return Ok(program);
    }
    for stale in [&digest_file, &program] {
        if let Err(e) = fs::remove_file(stale)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(BuildError::Failed(format!("{}: {e}", stale.display())));
        }
    }
    let compile_opts = crate::aarch64::A64CompileOptions {
        target: a64_target,
        optimize: true,
        jobs: options.jobs.unwrap_or(0) as usize,
        deadline: Some(deadline),
        spectre_mitigations,
        inc_cache_dir: Some(cache.join("a64-inc")),
        gen_fingerprint: gen_sha,
    };
    let elf_bytes =
        crate::aarch64::compile_wasm_to_aarch64(&wasm, &compile_opts).map_err(|e| match e {
            crate::x64::X64CompileError::AdmissionRejected(s) => BuildError::AdmissionRejected(s),
            crate::x64::X64CompileError::Failed(s) => BuildError::Failed(s),
        })?;
    check_deadline(deadline, options.time_limit)?;
    let seq = NEXT_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let staged = entry.join(format!("{artifact_name}.{}.{seq}", std::process::id()));
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let sha_hex = crate::sha256::artifact_sidecar_digest(&elf_bytes);
    let mut staged_created = false;
    let mut program_published = false;
    let publish_res = opts
        .open(&staged)
        .map_err(|e| format!("{}: {e}", staged.display()))
        .and_then(|mut f| {
            staged_created = true;
            f.write_all(&elf_bytes)
                .map_err(|e| format!("{}: {e}", staged.display()))
        })
        .and_then(|()| mark_file_immutable(&staged, 0o555))
        .and_then(|()| {
            fs::rename(&staged, &program).map_err(|e| format!("{}: {e}", program.display()))?;
            staged_created = false;
            program_published = true;
            Ok(())
        })
        .and_then(|()| publish_digest_sidecar(&entry, artifact_name, &sha_hex, &digest_file));
    if let Err(e) = publish_res {
        if program_published {
            let _ = fs::remove_file(&digest_file);
            let _ = fs::remove_file(&program);
        }
        if staged_created {
            let _ = fs::remove_file(&staged);
        }
        return Err(BuildError::Failed(e));
    }
    publish_lock.release_checked(options.time_limit)?;
    Ok(program)
}

/// Pure-Rust Wasm -> Linux x86-64 ELF64 `.so` cached build path (`--backend x64`).
fn build_x64(
    module: &Path,
    cache: &Path,
    options: &Options,
    spectre_mitigations: bool,
    deadline: std::time::Instant,
) -> Result<PathBuf, BuildError> {
    use std::io::Write as _;
    let timings = std::env::var_os("WTIME_AOT_TIMINGS").is_some_and(|v| v == "1");
    let t_b0 = std::time::Instant::now();
    check_deadline(deadline, options.time_limit)?;
    // Validate CPU syntax and host platform BEFORE cache lookup or file hashing.
    let (isa_profile, isa) = crate::x64::resolve_x64_isa(options.target_cpu)?;
    let wasm = read_bounded_wasm_for_x64(module, Some(deadline))?;
    let d_read = t_b0.elapsed();
    let t_b1 = std::time::Instant::now();
    check_deadline(deadline, options.time_limit)?;
    let cache = ensure_private_dir(cache)?;
    let gen_sha = generator_fingerprint::GENERATOR_BUILD_SHA256;
    let wasm_sha = crate::x64::inc_cache::parallel_payload_digest(&wasm)
        .unwrap_or_else(|| crate::sha256::digest(&wasm));
    let d_hash = t_b1.elapsed();
    let t_b2 = std::time::Instant::now();
    check_deadline(deadline, options.time_limit)?;

    let verify = cfg!(debug_assertions)
        || std::env::var_os("WTIME_AOT_VERIFY").is_some_and(|v| {
            v.to_str()
                .is_some_and(|s| s == "1" || s.eq_ignore_ascii_case("true"))
        });

    let x64_target = match options.target {
        crate::codegen::Target::BlotCdylib => crate::x64::module::X64Target::BlotCdylib,
        crate::codegen::Target::GooseBin => crate::x64::module::X64Target::StandaloneExec,
    };
    let artifact_name = match options.target {
        crate::codegen::Target::BlotCdylib => "library.so",
        crate::codegen::Target::GooseBin => "program",
    };
    let digest_name = match options.target {
        crate::codegen::Target::BlotCdylib => "library.so.sha256",
        crate::codegen::Target::GooseBin => "program.sha256",
    };

    // Key strictly by resolved internal ISA profile (`x64-v2` vs `x64-v3`), never raw `"native"`.
    // In release builds, bind explicit `WTIME_AOT_VERIFY=1` into the cache key so an earlier
    // unverified cache hit can never bypass IR/regalloc verification when verification is requested.
    let mut key = Sha256::new();
    field(&mut key, "wtime-aot build x64 4", &[]);
    field(
        &mut key,
        "spectre mitigations",
        &[u8::from(spectre_mitigations)],
    );
    field(&mut key, "target", artifact_name.as_bytes());
    field(&mut key, "wasm digest", &wasm_sha);
    field(&mut key, "generator build sha256", &gen_sha);
    field(&mut key, "isa profile", isa_profile.as_bytes());
    if verify && !cfg!(debug_assertions) {
        field(&mut key, "verify", b"1");
    }
    let key_hex = key.hex();

    let entry_path = cache.join(&key_hex);
    let entry_existed = fs::symlink_metadata(&entry_path).is_ok();
    let (publish_lock, entry) = lock_cache_entry(&entry_path, deadline)?;
    check_deadline(deadline, options.time_limit)?;
    let program = entry.join(artifact_name);
    let digest_file = entry.join(digest_name);
    let t_ver = std::time::Instant::now();
    if entry_existed && verified_cached_artifact(&program, &digest_file) {
        let d_ver = t_ver.elapsed();
        let t_sw = std::time::Instant::now();
        sweep_stale_staged_artifacts_bounded(&entry, deadline);
        let d_sw = t_sw.elapsed();
        let t_un = std::time::Instant::now();
        publish_lock.release_checked(options.time_limit)?;
        if timings {
            eprintln!(
                "wtime-aot x64 warm: read={:.2}ms hash={:.2}ms lock={:.2}ms verify={:.2}ms sweep={:.2}ms unlock={:.2}ms",
                d_read.as_secs_f64() * 1000.0,
                d_hash.as_secs_f64() * 1000.0,
                t_ver.duration_since(t_b2).as_secs_f64() * 1000.0,
                d_ver.as_secs_f64() * 1000.0,
                d_sw.as_secs_f64() * 1000.0,
                t_un.elapsed().as_secs_f64() * 1000.0,
            );
        }
        return Ok(program);
    }
    if entry_existed {
        for stale in [&digest_file, &program] {
            if let Err(e) = fs::remove_file(stale)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                return Err(BuildError::Failed(format!("{}: {e}", stale.display())));
            }
        }
    }
    check_deadline(deadline, options.time_limit)?;

    let compile_opts = crate::x64::X64CompileOptions {
        target: x64_target,
        isa,
        jobs: options.jobs.unwrap_or(0) as usize,
        deadline: Some(deadline),
        spectre_mitigations,
        verify,
        inc_cache_dir: Some(cache.clone()),
        cache_key_salt: Some(gen_sha),
        ..Default::default()
    };
    let (so_bytes, pending_inc_save) = crate::x64::compile_wasm_to_x64_so_with_pending_save(
        &wasm,
        &compile_opts,
    )
    .map_err(|e| match e {
        crate::x64::X64CompileError::AdmissionRejected(s) => BuildError::AdmissionRejected(s),
        crate::x64::X64CompileError::Failed(s) => BuildError::Failed(s),
    })?;
    check_deadline(deadline, options.time_limit)?;

    enum PublishPhase {
        InProgress,
        Completed(Result<(), String>),
        TimedOut,
    }

    let t_write = std::time::Instant::now();
    let seq = NEXT_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let staged = entry.join(format!("{artifact_name}.{}.{seq}", std::process::id()));
    let key_file = entry.join("key.txt");
    let key_txt_prefix = format!(
        "key {key_hex}\nbackend x64\nisa profile {isa_profile}\ntarget cpu {}\nwasm sha256 {}\nso sha256 ",
        options.target_cpu,
        crate::sha256::to_hex(&wasm_sha)
    );
    let time_limit = options.time_limit;
    let abandoned = Arc::new(AtomicBool::new(false));
    let state = Arc::new((
        std::sync::Mutex::new(PublishPhase::InProgress),
        std::sync::Condvar::new(),
    ));

    let cleanup_entry =
        |entry: &Path, staged: &Path, program: &Path, digest_file: &Path, key_file: &Path| {
            // Invalidate the committed pair (`digest_file`, `program`) first before removing
            // staging/metadata files.
            let _ = fs::remove_file(digest_file);
            let _ = fs::remove_file(program);
            let _ = fs::remove_file(staged);
            let _ = fs::remove_file(key_file);
            sweep_stale_staged_artifacts_inner(entry, true, None);
        };

    let staged_name = format!("{artifact_name}.{}.{seq}", std::process::id());
    let lock_pub = Arc::new(publish_lock.into_inner());
    let lock_weak = Arc::downgrade(&lock_pub);

    // Move the sole strong `Arc<fs::File>` (`lock_pub`) into `publish_task`; the main thread
    // retains only `Weak<fs::File>` (`lock_weak`) and never upgrades it on the main thread (so a
    // failed cleanup spawn under `EAGAIN` can never move the final `close(2)` onto the main
    // thread). All filesystem I/O—including closing `.publish.lock`—runs strictly OUTSIDE the
    // `state` mutex and BEFORE `Completed(Ok(()))` is published, so a stalled `close(2)` cannot
    // block `wait_timeout_while` or report exit 0 past `--time-limit`.
    // `publish_digest_sidecar` is the final atomic commit barrier (zero writes follow it):
    // any timeout before its `rename` completes leaves no reusable pair, and on a post-timeout
    // `rename` completion we attempt bounded rollback while guaranteeing that any surviving
    // pair is already immutable, content-addressed, and SHA-256 verified.
    let publish_task = {
        let entry = entry.clone();
        let staged = staged.clone();
        let program = program.clone();
        let digest_file = digest_file.clone();
        let key_file = key_file.clone();
        let staged_name = staged_name.clone();
        let abandoned_bg = Arc::clone(&abandoned);
        let state_bg = Arc::clone(&state);
        move || {
            let check_active = || -> Result<(), String> {
                if abandoned_bg.load(Ordering::Acquire) {
                    return Err(format!(
                        "the build exceeded its time limit of {time_limit:?}"
                    ));
                }
                check_deadline(deadline, time_limit)
            };
            let mut opts = fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
            // Overlap staging file write + chmod with sidecar digest computation over `so_bytes`.
            let (write_res, so_sha) = std::thread::scope(|s| {
                let h_sha = std::thread::Builder::new()
                    .spawn_scoped(s, || crate::sha256::artifact_sidecar_digest(&so_bytes))
                    .ok();
                let wr = opts
                    .open(&staged)
                    .and_then(|mut f| f.write_all(&so_bytes))
                    .map_err(|e| format!("{}: {e}", staged.display()))
                    .and_then(|()| mark_artifact_immutable(&staged));
                let sha = h_sha
                    .and_then(|h| h.join().ok())
                    .unwrap_or_else(|| crate::sha256::artifact_sidecar_digest(&so_bytes));
                (wr, sha)
            });
            // Write diagnostic `key.txt` and sweep any stale staging files (excluding our
            // active `staged_name`) BEFORE publishing `library.so` and its digest sidecar so
            // `publish_digest_sidecar` is the sole final commit barrier.
            let publish_res = write_res
                .and_then(|()| check_active())
                .and_then(|()| {
                    let _ = fs::write(&key_file, format!("{key_txt_prefix}{so_sha}\n"));
                    if entry_existed {
                        sweep_stale_staged_artifacts_inner(&entry, true, Some(&staged_name));
                    }
                    check_active()
                })
                .and_then(|()| {
                    fs::rename(&staged, &program).map_err(|e| format!("{}: {e}", program.display()))
                })
                .and_then(|()| check_active())
                .and_then(|()| publish_digest_sidecar(&entry, artifact_name, &so_sha, &digest_file))
                .and_then(|()| check_active());

            let (lk, cv) = &*state_bg;
            match publish_res {
                Ok(()) => {
                    let phase = lk.lock().unwrap_or_else(|g| g.into_inner());
                    if matches!(*phase, PublishPhase::TimedOut) {
                        drop(phase);
                        // Main thread already timed out while we still hold `lock_pub`:
                        // roll back under `lock_pub`, then close `lock_pub`.
                        cleanup_entry(&entry, &staged, &program, &digest_file, &key_file);
                        drop(lock_pub);
                        return;
                    }
                    drop(phase);
                    // Close `.publish.lock` OUTSIDE the `state` mutex and BEFORE recording
                    // `Completed(Ok(()))` so a stalled `close(2)` cannot report exit 0 past
                    // `--time-limit` or block `wait_timeout_while`.
                    drop(lock_pub);
                    let final_res = check_active();
                    let mut phase = lk.lock().unwrap_or_else(|g| g.into_inner());
                    if matches!(*phase, PublishPhase::InProgress) {
                        *phase = PublishPhase::Completed(final_res);
                        cv.notify_all();
                    }
                }
                Err(e) => {
                    // Roll back outside `state` mutex while STILL holding `lock_pub`, then
                    // close `lock_pub` before publishing `Completed(Err(e))`.
                    cleanup_entry(&entry, &staged, &program, &digest_file, &key_file);
                    drop(lock_pub);
                    let mut phase = lk.lock().unwrap_or_else(|g| g.into_inner());
                    if matches!(*phase, PublishPhase::InProgress) {
                        *phase = PublishPhase::Completed(Err(e));
                        cv.notify_all();
                    }
                }
            }
        }
    };

    // Run final staging/rename/sidecar publication off-thread cooperatively under `--time-limit`
    // (allowing up to 5 ms post-launch cleanup wait after timeout). If OS thread creation fails
    // (`EAGAIN`), falls back synchronously on the current thread—where blocking filesystem
    // syscalls (including `.publish.lock` close) must return before `check_active` records
    // deadline expiry. In both paths publication never reports success past `deadline`, and
    // rollbacks stay under `publish_lock`.
    let task_slot = Arc::new(std::sync::Mutex::new(Some(publish_task)));
    let task_bg = Arc::clone(&task_slot);
    let spawned = std::thread::Builder::new().spawn(move || {
        if let Some(f) = task_bg.lock().unwrap_or_else(|g| g.into_inner()).take() {
            f();
        }
    });
    match spawned {
        Ok(_) => {
            drop(task_slot);
            let rem = deadline.saturating_duration_since(std::time::Instant::now());
            let (lk, cv) = &*state;
            let guard = lk.lock().unwrap_or_else(|g| g.into_inner());
            let (mut guard, _) = cv
                .wait_timeout_while(guard, rem, |p| matches!(*p, PublishPhase::InProgress))
                .unwrap_or_else(|e| e.into_inner());
            let prev = std::mem::replace(&mut *guard, PublishPhase::TimedOut);
            match prev {
                PublishPhase::Completed(Ok(())) => {
                    *guard = PublishPhase::Completed(Ok(()));
                    drop(guard);
                    check_deadline(deadline, options.time_limit)?;
                }
                PublishPhase::Completed(Err(e)) => {
                    *guard = PublishPhase::Completed(Err(e.clone()));
                    drop(guard);
                    return Err(BuildError::Failed(e));
                }
                PublishPhase::InProgress | PublishPhase::TimedOut => {
                    abandoned.store(true, Ordering::Release);
                    drop(guard);
                    // Move ONLY `Weak<fs::File>` into the cleanup closure and upgrade INSIDE the
                    // spawned worker: if `spawn` fails with `EAGAIN`, dropping the closure drops
                    // only `Weak` and never closes `.publish.lock` on the main thread. Inside the
                    // worker, `upgrade()` succeeds iff `publish_task` still owns `lock_pub`,
                    // keeping `.publish.lock` held across rollback and skipping unlocked cleanup
                    // after commit.
                    let cleanup_cap =
                        std::time::Instant::now() + std::time::Duration::from_millis(5);
                    if lock_weak.strong_count() > 0 {
                        let clean_done =
                            Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
                        let clean_bg = Arc::clone(&clean_done);
                        let entry_c = entry.clone();
                        let staged_c = staged.clone();
                        let program_c = program.clone();
                        let digest_c = digest_file.clone();
                        let key_c = key_file.clone();
                        if std::thread::Builder::new()
                            .spawn(move || {
                                if let Some(lock_clean) = lock_weak.upgrade() {
                                    cleanup_entry(
                                        &entry_c, &staged_c, &program_c, &digest_c, &key_c,
                                    );
                                    drop(lock_clean);
                                }
                                let (clk, ccv) = &*clean_bg;
                                *clk.lock().unwrap_or_else(|g| g.into_inner()) = true;
                                ccv.notify_all();
                            })
                            .is_ok()
                        {
                            let wait_clean =
                                cleanup_cap.saturating_duration_since(std::time::Instant::now());
                            if !wait_clean.is_zero() {
                                let (clk, ccv) = &*clean_done;
                                let cg = clk.lock().unwrap_or_else(|g| g.into_inner());
                                drop(ccv.wait_timeout_while(cg, wait_clean, |d| !*d));
                            }
                        }
                    }
                    return Err(BuildError::Failed(format!(
                        "the build exceeded its time limit of {:?}",
                        options.time_limit
                    )));
                }
            }
        }
        Err(_) => {
            // Persistent `EAGAIN`: `publish_task` holds the sole `lock_pub`, closes it before
            // the final `check_active()`, and rolls back on deadline expiry before commit.
            if let Some(f) = task_slot.lock().unwrap_or_else(|g| g.into_inner()).take() {
                f();
            }
            let (lk, _) = &*state;
            let mut guard = lk.lock().unwrap_or_else(|g| g.into_inner());
            match std::mem::replace(&mut *guard, PublishPhase::TimedOut) {
                PublishPhase::Completed(Ok(())) => {
                    drop(guard);
                    check_deadline(deadline, options.time_limit)?;
                }
                PublishPhase::Completed(Err(e)) => return Err(BuildError::Failed(e)),
                PublishPhase::InProgress | PublishPhase::TimedOut => {
                    return Err(BuildError::Failed(format!(
                        "the build exceeded its time limit of {:?}",
                        options.time_limit
                    )));
                }
            }
        }
    }
    let d_write = t_write.elapsed();
    let t_wait = std::time::Instant::now();
    if let Some(pending) = pending_inc_save {
        crate::x64::wait_pending_inc_cache_save(pending, Some(deadline));
    }
    if timings {
        eprintln!(
            "wtime-aot x64 build: read={:.2}ms hash={:.2}ms write={:.2}ms wait_inc={:.2}ms",
            d_read.as_secs_f64() * 1000.0,
            d_hash.as_secs_f64() * 1000.0,
            d_write.as_secs_f64() * 1000.0,
            t_wait.elapsed().as_secs_f64() * 1000.0,
        );
    }
    Ok(program)
}

/// Best-effort bounded wrapper around `sweep_stale_staged_artifacts` so a stalled cache
/// directory read on warm hits or cold misses never exhausts `--time-limit` or blocks a valid
/// hit. Only ever unlinks dead-PID/stale staging files (never live artifacts or live PIDs).
fn sweep_stale_staged_artifacts_bounded(entry: &Path, deadline: std::time::Instant) {
    let rem = deadline.saturating_duration_since(std::time::Instant::now());
    let budget = rem
        .saturating_sub(std::time::Duration::from_millis(200))
        .min(std::time::Duration::from_millis(100));
    if budget.is_zero() {
        return;
    }
    let cap = std::time::Instant::now() + budget;
    let done = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let done_bg = Arc::clone(&done);
    let entry_buf = entry.to_path_buf();
    if std::thread::Builder::new()
        .spawn(move || {
            // Never unlink `my_pid`'s staging files from a potentially detached pre-build sweep
            // so a slow directory read cannot race with this process's own active staging later.
            sweep_stale_staged_artifacts_inner(&entry_buf, false, None);
            let (lk, cv) = &*done_bg;
            *lk.lock().unwrap_or_else(|g| g.into_inner()) = true;
            cv.notify_all();
        })
        .is_ok()
    {
        let wait = cap.saturating_duration_since(std::time::Instant::now());
        if !wait.is_zero() {
            let (lk, cv) = &*done;
            let guard = lk.lock().unwrap_or_else(|g| g.into_inner());
            drop(cv.wait_timeout_while(guard, wait, |d| !*d));
        }
    }
}

/// Removes orphaned `library.so.<pid>.<seq>` and `library.so.sha256.<pid>.<seq>` staging files
/// from `entry`. On Linux (`/proc/self` present), a live foreign PID's staging file is NEVER
/// deleted regardless of age; only exited PIDs (or `my_pid` when `include_my_pid` is true)
/// are removed. On platforms without `/proc`, falls back to a 60 s age threshold.
fn sweep_stale_staged_artifacts_inner(
    entry: &Path,
    include_my_pid: bool,
    exclude_name: Option<&str>,
) {
    let Ok(rd) = fs::read_dir(entry) else {
        return;
    };
    let my_pid = std::process::id();
    let has_proc = Path::new("/proc/self").exists();
    let now = std::time::SystemTime::now();
    for de in rd.flatten() {
        let name = de.file_name();
        let Some(s) = name.to_str() else {
            continue;
        };
        if exclude_name == Some(s) {
            continue;
        }
        let Some(rest) = s
            .strip_prefix("library.so.sha256.")
            .or_else(|| s.strip_prefix("library.so."))
            .or_else(|| s.strip_prefix("program.sha256."))
            .or_else(|| s.strip_prefix("program."))
        else {
            continue;
        };
        let Some((pid_s, seq_s)) = rest.split_once('.') else {
            continue;
        };
        if pid_s.is_empty()
            || seq_s.is_empty()
            || !pid_s.bytes().all(|b| b.is_ascii_digit())
            || !seq_s.bytes().all(|b| b.is_ascii_digit())
        {
            continue;
        }
        let Ok(pid) = pid_s.parse::<u32>() else {
            continue;
        };
        if !include_my_pid && pid == my_pid {
            continue;
        }
        let dead_or_stale = if include_my_pid && pid == my_pid {
            true
        } else if has_proc {
            !Path::new(&format!("/proc/{pid}")).exists()
        } else {
            de.metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|mt| now.duration_since(mt).ok())
                .is_some_and(|age| age >= std::time::Duration::from_secs(60))
        };
        if dead_or_stale {
            let _ = fs::remove_file(de.path());
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
    let deadline = std::time::Instant::now() + options.time_limit;
    let prepared = prepare(module, cache, module_path, rt, options, deadline)?;
    let mut key = prepared.key.clone();
    field(&mut key, "profile-generate", &[]);
    let instrumented = cached(
        &prepared,
        key,
        &["-Cprofile-generate".into()],
        options,
        "instrumented for profiling\n",
        deadline,
    )?;

    // The training run, in a directory of its own for the raw profiles.
    let seq = NEXT_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let work_root = ensure_private_dir(&prepared.cache.join("work"))?;
    let profiles = work_root.join(format!("profile-{}-{seq}", std::process::id()));
    let _ = fs::remove_dir_all(&profiles);
    let profiles = ensure_private_dir(&profiles)?;
    let result = train_and_merge(&instrumented, train, &profiles, options, deadline);
    let profile = result
        .and_then(|merged| fs::read(&merged).map_err(|e| format!("{}: {e}", merged.display())));
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
    let stored = work_root.join(format!(
        "profile-{digest}-{}-{seq}.profdata",
        std::process::id()
    ));
    fs::write(&stored, &profile).map_err(|e| format!("{}: {e}", stored.display()))?;
    let _ = fs::remove_dir_all(&profiles);
    let flag = format!("-Cprofile-use={}", stored.display());
    let note = format!("profile-guided: profile sha256 {digest}, training arguments {train:?}\n");
    let result = cached(&prepared, key, &[flag], options, &note, deadline);
    let _ = fs::remove_file(&stored);
    result
}

fn field(key: &mut Sha256, name: &str, bytes: &[u8]) {
    key.update(name.as_bytes());
    key.update(&(bytes.len() as u64).to_le_bytes());
    key.update(bytes);
}

/// Reads and checks what a build starts from.
fn prepare(
    module: &Path,
    cache: &Path,
    module_path: Option<&str>,
    rt: &Path,
    options: &Options,
    deadline: std::time::Instant,
) -> Result<Prepared, String> {
    check_deadline(deadline, options.time_limit)?;
    let wasm = read_bounded_wasm(module, MAX_WASM_BYTES_C_RUST, Some(deadline))?;
    check_deadline(deadline, options.time_limit)?;
    // The program reports the module's own name, not where the operator
    // keeps it.
    let module_path = match module_path {
        Some(path) => path.to_string(),
        None => module.file_name().map_or("module.wasm".into(), |name| {
            name.to_string_lossy().into_owned()
        }),
    };
    let cache = ensure_private_dir(cache)?;
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
    let rt = rt
        .canonicalize()
        .map_err(|e| format!("{}: {e}", rt.display()))?;
    let rt_files = runtime_sources(&rt)?;

    // The key: everything the program depends on (including snapshotted rt sources,
    // cargo version, and any active CARGO_HOME/HOME cargo config).
    let mut key = Sha256::new();
    field(&mut key, "wtime-aot build 3", &[]);
    field(&mut key, "wasm", &wasm);
    field(&mut key, "module path", module_path.as_bytes());
    let generator = std::env::current_exe().map_err(|e| format!("this generator: {e}"))?;
    field(
        &mut key,
        "generator build sha256",
        &generator_fingerprint::GENERATOR_BUILD_SHA256,
    );
    for (name, bytes) in &rt_files {
        field(&mut key, name, bytes);
    }
    let version = rustc_version(options.toolchain, Some(deadline))?;
    field(&mut key, "toolchain", version.as_bytes());
    let cargo_ver = cargo_version(options.toolchain, Some(deadline))?;
    field(&mut key, "cargo", cargo_ver.as_bytes());
    for (cfg_name, cfg_bytes) in active_cargo_configs()? {
        field(&mut key, &format!("cargo config {cfg_name}"), &cfg_bytes);
    }
    // The CPU and the features it resolves to here: a program built for one
    // machine's CPU is not reused for another's.
    field(&mut key, "target cpu", options.target_cpu.as_bytes());
    field(
        &mut key,
        "target features",
        target_features(options.toolchain, options.target_cpu, Some(deadline))?.as_bytes(),
    );
    check_deadline(deadline, options.time_limit)?;
    // A 64-bit memory is flat (allocated whole at its maximum, accessed
    // without a slow path) when this machine grants its maximum.
    let flat64 = match crate::codegen::memory64_max(&wasm)? {
        Some(pages) => {
            usize::try_from(pages.min(1 << 20).saturating_mul(65536)).is_ok_and(|bytes| {
                let mut probe: Vec<u8> = Vec::new();
                probe.try_reserve_exact(bytes).is_ok()
            })
        }
        None => false,
    };
    field(&mut key, "flat memory64", &[flat64 as u8]);
    let target_tag = match options.target {
        crate::codegen::Target::GooseBin => "goose-bin",
        crate::codegen::Target::BlotCdylib => "blot-cdylib",
    };
    field(&mut key, "target", target_tag.as_bytes());
    let record = format!(
        "wasm sha256 {}\nmodule path {module_path:?}\ngenerator {}\nruntime {}\ntoolchain {}\ncargo {}\ntarget cpu {}\ntarget {target_tag}\nflat memory64 {flat64}\n",
        crate::sha256::hex(&wasm),
        generator.display(),
        rt.display(),
        version.trim().replace('\n', "; "),
        cargo_ver.trim(),
        options.target_cpu,
    );
    Ok(Prepared {
        wasm,
        module_path,
        cache,
        rt_files,
        flat64,
        key,
        record,
    })
}

/// The program cached under `key`, built with `rustflags` if not there yet.
fn cached(
    prepared: &Prepared,
    key: Sha256,
    rustflags: &[String],
    options: &Options,
    note: &str,
    deadline: std::time::Instant,
) -> Result<PathBuf, String> {
    let key = key.hex();
    let cache = &prepared.cache;
    check_deadline(deadline, options.time_limit)?;
    let (publish_lock, entry) = lock_cache_entry(&cache.join(&key), deadline)?;
    check_deadline(deadline, options.time_limit)?;
    let artifact_name = match options.target {
        crate::codegen::Target::GooseBin => "program",
        crate::codegen::Target::BlotCdylib => "library.so",
    };
    let program = entry.join(artifact_name);
    let digest_file = entry.join(format!("{artifact_name}.sha256"));
    if verified_cached_artifact(&program, &digest_file) {
        publish_lock.release_checked(options.time_limit)?;
        return Ok(program);
    }
    for stale in [&digest_file, &program] {
        if let Err(e) = fs::remove_file(stale)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(format!("{}: {e}", stale.display()));
        }
    }
    check_deadline(deadline, options.time_limit)?;

    // Generate and compile in a directory of this build's own, target
    // directory included, then move the program into place: builds of the
    // same or of different modules share nothing while they run.
    let name = format!("aot-{key}");
    let seq = NEXT_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let work_root = ensure_private_dir(&cache.join("work"))?;
    let work = work_root.join(format!("{key}-{}-{seq}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    let work = ensure_private_dir(&work)?;
    let result = generate_and_compile(prepared, &work, &name, rustflags, options, deadline)
        .and_then(|built| {
            check_deadline(deadline, options.time_limit)?;
            let staged = entry.join(format!("{artifact_name}.{}.{seq}", std::process::id()));
            fs::copy(&built, &staged).map_err(|e| format!("{}: {e}", built.display()))?;
            let bytes = fs::read(&staged).map_err(|e| format!("{}: {e}", staged.display()))?;
            let sha = crate::sha256::artifact_sidecar_digest(&bytes);
            if let Err(e) = mark_artifact_immutable(&staged)
                .and_then(|()| {
                    fs::rename(&staged, &program).map_err(|e| format!("{}: {e}", program.display()))
                })
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
    let _ = fs::write(
        entry.join("key.txt"),
        format!(
            "key {key}\n{}artifact sha256 {sha}\n{note}",
            prepared.record
        ),
    );
    publish_lock.release_checked(options.time_limit)?;
    Ok(program)
}

/// Runs the instrumented program with `train` and merges the profile it
/// writes into `profiles`; the merged profile's path.
fn train_and_merge(
    instrumented: &Path,
    train: &[String],
    profiles: &Path,
    options: &Options,
    deadline: std::time::Instant,
) -> Result<PathBuf, String> {
    // The program's own sandbox applies: it reaches files and variables
    // only as `train` grants. Its output is not needed.
    let mut run = Command::new(instrumented);
    run.env_clear()
        .env("LLVM_PROFILE_FILE", profiles.join("%m-%p.profraw"))
        .args(train)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    if !inherit_pgid() {
        std::os::unix::process::CommandExt::process_group(&mut run, 0);
    }
    wait(run, "the training run", deadline, options.time_limit)?;
    let raw: Vec<PathBuf> = fs::read_dir(profiles)
        .map_err(|e| format!("{}: {e}", profiles.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|e| e == "profraw"))
        .collect();
    if raw.is_empty() {
        return Err("the training run wrote no profile".into());
    }
    let mut sysroot_cmd = clean_rustc_command(options.toolchain)?;
    sysroot_cmd.args(["--print", "sysroot"]);
    let sysroot =
        run_cmd_capture_with_deadline(sysroot_cmd, "rustc --print sysroot", Some(deadline))?;
    if !sysroot.status.success() {
        return Err(format!("rustc --print sysroot failed ({})", sysroot.status));
    }
    let sysroot = PathBuf::from(String::from_utf8_lossy(&sysroot.stdout).trim());
    let host = rustc_version(options.toolchain, Some(deadline))?
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_string))
        .ok_or("rustc -vV names no host")?;
    let profdata = sysroot
        .join("lib")
        .join("rustlib")
        .join(host)
        .join("bin")
        .join("llvm-profdata");
    if !profdata.is_file() {
        return Err(format!(
            "{} is missing: install it with `rustup component add llvm-tools --toolchain {}`",
            profdata.display(),
            options.toolchain
        ));
    }
    let merged = profiles.join("merged.profdata");
    let mut merge = Command::new(&profdata);
    merge.env_clear();
    if let Some(path) = sanitized_path_env() {
        merge.env("PATH", path);
    }
    for var in ["HOME", "TMPDIR"] {
        if let Some(val) = std::env::var_os(var) {
            merge.env(var, val);
        }
    }
    merge
        .arg("merge")
        .arg("-o")
        .arg(&merged)
        .args(&raw)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    #[cfg(unix)]
    if !inherit_pgid() {
        std::os::unix::process::CommandExt::process_group(&mut merge, 0);
    }
    wait(merge, "llvm-profdata", deadline, options.time_limit)?;
    Ok(merged)
}

/// Runs `command` (in its own process group unless `WTIME_AOT_INHERIT_PGID=1`), cancelling its
/// entire child tree if `deadline` expires; an error unless it succeeds, except that a training
/// run may end with any exit code (a guest's own failure still profiles the code it ran).
fn wait(
    mut command: Command,
    what: &str,
    deadline: std::time::Instant,
    time_limit: std::time::Duration,
) -> Result<(), String> {
    if std::time::Instant::now() >= deadline {
        return Err(format!("{what} exceeded its time limit of {time_limit:?}"));
    }
    let mut child = command.spawn().map_err(|e| format!("{what}: {e}"))?;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    terminate_child_tree(&mut child);
                    return Err(format!("{what} exceeded its time limit of {time_limit:?}"));
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(e) => {
                terminate_child_tree(&mut child);
                return Err(format!("{what}: {e}"));
            }
        }
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
fn generate_and_compile(
    prepared: &Prepared,
    work: &Path,
    name: &str,
    rustflags: &[String],
    options: &Options,
    deadline: std::time::Instant,
) -> Result<PathBuf, String> {
    let Options {
        toolchain,
        target_cpu,
        jobs,
        time_limit,
        target: target_kind,
        ..
    } = *options;
    check_deadline(deadline, time_limit)?;
    // Materialize the exact hashed `rt` snapshot as a sibling directory `work/rt`
    // beside the generated workspace `work/crate` so `rt/Cargo.toml`'s `[workspace]`
    // never nests inside the generated crate's `[workspace]`, with zero TOCTOU.
    let rt_snap = work.join("rt");
    let crate_dir = work.join("crate");
    for (rel, bytes) in &prepared.rt_files {
        let dest = rt_snap.join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        fs::write(&dest, bytes).map_err(|e| format!("{}: {e}", dest.display()))?;
    }
    crate::codegen::generate_target_with_deadline(
        &prepared.wasm,
        &crate_dir,
        name,
        &rt_snap,
        &prepared.module_path,
        prepared.flat64,
        target_kind,
        Some(deadline),
    )?;
    check_deadline(deadline, time_limit)?;
    let target = work.join("target");
    let cargo_bin = crate::cgen::find_in_path("cargo")
        .ok_or_else(|| "no executable `cargo` found in absolute PATH entries".to_string())?;
    let mut cargo = Command::new(cargo_bin);
    cargo
        .env_clear()
        .current_dir(&crate_dir)
        .stdin(Stdio::null());
    if let Some(path) = sanitized_path_env() {
        cargo.env("PATH", path);
    }
    for variable in ["HOME", "RUSTUP_HOME", "CARGO_HOME"] {
        if let Some(value) = std::env::var_os(variable) {
            cargo.env(variable, value);
        }
    }
    // rustc recurses on nested code: a stack of its own size for that.
    cargo
        .env("RUSTUP_TOOLCHAIN", toolchain)
        .env("CARGO_TARGET_DIR", &target)
        .env("RUST_MIN_STACK", "268435456");
    // The target CPU through CARGO_ENCODED_RUSTFLAGS, which takes
    // precedence over every rustflags setting in Cargo's configuration.
    let mut flags = vec![format!("-Ctarget-cpu={target_cpu}")];
    flags.extend(rustflags.iter().cloned());
    cargo.env("CARGO_ENCODED_RUSTFLAGS", flags.join("\u{1f}"));
    cargo.args(["build", "--release", "--offline", "--quiet"]);
    if let Some(jobs) = jobs.filter(|&j| j > 0) {
        cargo.args(["--jobs", &jobs.to_string()]);
    }
    // Run in a dedicated process group unless an outer caller already manages our group.
    #[cfg(unix)]
    if !inherit_pgid() {
        std::os::unix::process::CommandExt::process_group(&mut cargo, 0);
    }
    let mut child = cargo.spawn().map_err(|e| format!("cargo: {e}"))?;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    terminate_child_tree(&mut child);
                    return Err(format!(
                        "the build exceeded its time limit of {:?}",
                        time_limit
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => {
                terminate_child_tree(&mut child);
                return Err(format!("cargo: {e}"));
            }
        }
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
                pending.push(
                    entry
                        .map_err(|e| format!("{}: {e}", path.display()))?
                        .path(),
                );
            }
        } else {
            let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let name = path
                .strip_prefix(rt)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            files.push((name, bytes));
        }
    }
    files.sort();
    Ok(files)
}

fn clean_rustc_command(toolchain: &str) -> Result<Command, String> {
    let rustc_bin = crate::cgen::find_in_path("rustc")
        .ok_or_else(|| "no executable `rustc` found in absolute PATH entries".to_string())?;
    let mut cmd = Command::new(rustc_bin);
    cmd.env_clear();
    if let Some(path) = sanitized_path_env() {
        cmd.env("PATH", path);
    }
    for variable in ["HOME", "RUSTUP_HOME", "CARGO_HOME"] {
        if let Some(value) = std::env::var_os(variable) {
            cmd.env(variable, value);
        }
    }
    cmd.env("RUSTUP_TOOLCHAIN", toolchain);
    Ok(cmd)
}

/// The target features `target_cpu` resolves to on this machine
/// (`rustc --print cfg`).
fn target_features(
    toolchain: &str,
    target_cpu: &str,
    deadline: Option<std::time::Instant>,
) -> Result<String, String> {
    let mut cfg_cmd = clean_rustc_command(toolchain)?;
    cfg_cmd.args(["--print", "cfg", "-C", &format!("target-cpu={target_cpu}")]);
    let output = run_cmd_capture_with_deadline(cfg_cmd, "rustc --print cfg", deadline)?;
    if !output.status.success() {
        return Err(format!(
            "target cpu {target_cpu}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let mut features = String::from_utf8_lossy(&output.stdout).into_owned();
    // `--print cfg` lists stable features only: the CPU LLVM resolves
    // `native` to covers the rest.
    let mut cpus_cmd = clean_rustc_command(toolchain)?;
    cpus_cmd.args([
        "--print",
        "target-cpus",
        "-C",
        &format!("target-cpu={target_cpu}"),
    ]);
    let cpus = run_cmd_capture_with_deadline(cpus_cmd, "rustc --print target-cpus", deadline)?;
    let listing = String::from_utf8_lossy(&cpus.stdout);
    if let Some(current) = listing.lines().find(|line| line.contains("currently")) {
        features.push_str(current.trim());
    }
    Ok(features)
}

/// `rustc -vV` of the toolchain.
fn rustc_version(toolchain: &str, deadline: Option<std::time::Instant>) -> Result<String, String> {
    let mut cmd = clean_rustc_command(toolchain)?;
    cmd.arg("-vV");
    let output = run_cmd_capture_with_deadline(cmd, "rustc -vV", deadline)?;
    if !output.status.success() {
        return Err(format!(
            "toolchain {toolchain}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `cargo --version` of the toolchain.
fn cargo_version(toolchain: &str, deadline: Option<std::time::Instant>) -> Result<String, String> {
    let cargo_bin = crate::cgen::find_in_path("cargo")
        .ok_or_else(|| "no executable `cargo` found in absolute PATH entries".to_string())?;
    let mut cmd = Command::new(cargo_bin);
    cmd.env_clear();
    if let Some(path) = sanitized_path_env() {
        cmd.env("PATH", path);
    }
    for variable in ["HOME", "RUSTUP_HOME", "CARGO_HOME"] {
        if let Some(value) = std::env::var_os(variable) {
            cmd.env(variable, value);
        }
    }
    cmd.env("RUSTUP_TOOLCHAIN", toolchain).arg("--version");
    let output = run_cmd_capture_with_deadline(cmd, "cargo --version", deadline)?;
    if !output.status.success() {
        return Err(format!(
            "cargo --version ({toolchain}): {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Collects any active `CARGO_HOME` or `$HOME/.cargo` configuration files so they are hashed
/// into the Rust backend cache key.
fn active_cargo_configs() -> Result<Vec<(String, Vec<u8>)>, String> {
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")));
    let mut out = Vec::new();
    if let Some(home) = cargo_home {
        for name in ["config", "config.toml"] {
            let p = home.join(name);
            if p.is_file() {
                let bytes = fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
                out.push((name.to_string(), bytes));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) struct TestTempRoot(pub(crate) PathBuf);

#[cfg(test)]
impl Drop for TestTempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
pub(crate) fn create_fresh_test_root(prefix: &str) -> TestTempRoot {
    let base = std::env::temp_dir()
        .canonicalize()
        .expect("canonicalize temp_dir");
    for _ in 0..64 {
        let seq = NEXT_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let candidate = base.join(format!("{prefix}-{}-{seq}-{nanos}", std::process::id()));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&candidate) {
            Ok(()) => {
                let canon = ensure_private_dir(&candidate).expect("validate fresh test root");
                return TestTempRoot(canon);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => panic!(
                "failed to create fresh test root {}: {e}",
                candidate.display()
            ),
        }
    }
    panic!("exhausted attempts creating fresh test root for {prefix}");
}

#[cfg(test)]
pub(crate) fn assert_inside_test_root(root: &Path, path: &Path) {
    assert!(
        !path.components().any(|c| matches!(c, Component::ParentDir)),
        "test path {} must not contain parent traversal",
        path.display()
    );
    if let Ok(meta) = fs::symlink_metadata(path) {
        assert!(
            !meta.file_type().is_symlink(),
            "test output {} must not be a symlink",
            path.display()
        );
        let canon = path
            .canonicalize()
            .expect("canonicalize existing test path");
        assert!(
            canon.starts_with(root),
            "existing test output {} resolved outside fresh root {}",
            path.display(),
            root.display()
        );
    }
    let mut existing = path.parent().expect("test path has parent");
    while !existing.exists() {
        existing = existing.parent().expect("ancestor exists");
    }
    let canon_existing = existing
        .canonicalize()
        .expect("canonicalize existing parent");
    assert!(
        canon_existing.starts_with(root),
        "test output {} resolved outside fresh root {}",
        path.display(),
        root.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_digest_sidecar_is_immutable_and_rejects_group_writable() {
        let root = create_fresh_test_root("wtime-aot-sidecar-test");
        let dir = &root.0;
        let art = dir.join("library.so");
        let sha_path = dir.join("library.so.sha256");
        assert_inside_test_root(dir, &art);
        assert_inside_test_root(dir, &sha_path);
        let payload = b"dummy-verified-artifact";
        fs::write(&art, payload).unwrap();
        mark_artifact_immutable(&art).unwrap();
        let sha = crate::sha256::hex(payload);
        publish_digest_sidecar(dir, "library.so", &sha, &sha_path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::symlink_metadata(&sha_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(
                mode, 0o444,
                "sidecar must be immutable 0o444 regardless of umask"
            );
        }
        assert_eq!(
            verified_cached_artifact_bytes(&art, &sha_path).as_deref(),
            Some(payload.as_slice())
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&sha_path, fs::Permissions::from_mode(0o664)).unwrap();
            assert!(
                verified_cached_artifact_bytes(&art, &sha_path).is_none(),
                "group-writable 0o664 sidecar must be rejected"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn read_bounded_wasm_rejects_fifo_without_blocking() {
        let root = create_fresh_test_root("wtime-aot-fifo-test");
        let fifo = root.0.join("input.wasm");
        assert_inside_test_root(&root.0, &fifo);
        let status = Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo must execute");
        assert!(status.success(), "mkfifo failed");
        let started = std::time::Instant::now();
        let err = read_bounded_wasm(
            &fifo,
            64 << 20,
            Some(started + std::time::Duration::from_secs(5)),
        )
        .expect_err("FIFO must be rejected immediately");
        assert!(
            err.contains("regular file"),
            "unexpected FIFO rejection message: {err}"
        );
        let err_x64 =
            read_bounded_wasm_for_x64(&fifo, Some(started + std::time::Duration::from_secs(5)))
                .expect_err("x64 reader must reject FIFO immediately");
        assert!(matches!(err_x64, BuildError::Failed(_)));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "FIFO open/check blocked unexpectedly: {:?}",
            started.elapsed()
        );
    }
}
