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
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};

use crate::sha256::Sha256;

static NEXT_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

const MAX_WASM_BYTES_C_RUST: usize = 256 << 20;

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
                if target_cpu.is_empty()
                    || target_cpu.starts_with('-')
                    || !target_cpu.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
                {
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
    let deadline = std::time::Instant::now() + options.time_limit;
    if options.backend == Backend::C {
        return build_c(module, cache, options, deadline);
    }
    let prepared = prepare(module, cache, module_path, rt, options, deadline)?;
    cached(&prepared, prepared.key.clone(), &[], options, "", deadline)
}

fn check_deadline(deadline: std::time::Instant, time_limit: std::time::Duration) -> Result<(), String> {
    if std::time::Instant::now() >= deadline {
        return Err(format!("the build exceeded its time limit of {time_limit:?}"));
    }
    Ok(())
}

/// Reads a Wasm input file with a hard pre-allocation byte cap (checking both initial file
/// metadata AND streaming `Read::take(max_bytes + 1)` so growing files/pipes stay bounded).
pub(crate) fn read_bounded_wasm(path: &Path, max_bytes: usize) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Ok(meta) = file.metadata()
        && meta.is_file()
        && meta.len() > max_bytes as u64
    {
        return Err(format!(
            "{}: input size ({} B) exceeds maximum module limit ({max_bytes} B)",
            path.display(),
            meta.len()
        ));
    }
    let mut buf = Vec::new();
    file.take((max_bytes as u64).saturating_add(1))
        .read_to_end(&mut buf)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if buf.len() > max_bytes {
        return Err(format!(
            "{}: input size exceeds maximum module limit ({max_bytes} B)",
            path.display()
        ));
    }
    Ok(buf)
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

/// Reaps any lingering descendants in `pid`'s dedicated process group after `child` exits.
pub(crate) fn reap_child_group_after_exit(pid: u32) {
    #[cfg(unix)]
    if !inherit_pgid() {
        kill_process_group(pid);
    }
    #[cfg(not(unix))]
    let _ = pid;
}

pub(crate) struct RemoveFileOnDrop(pub(crate) PathBuf);

impl Drop for RemoveFileOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Creates a unique owner-only (`0o600`, `O_CREAT | O_EXCL`) temporary file for capture/probes.
pub(crate) fn create_private_temp_file(dir: Option<&Path>, prefix: &str, suffix: &str) -> Result<(fs::File, PathBuf), String> {
    let base = dir.map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir);
    for _ in 0..32 {
        let seq = NEXT_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = base.join(format!(".wtime-aot-{prefix}-{}-{seq}{suffix}", std::process::id()));
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
    Err(format!("failed to create unique private temp file under {}", base.display()))
}

const MAX_CAPTURE_BYTES: usize = 16 << 20;

pub(crate) enum PipeCaptureErr {
    Overflow,
    Io(String),
}

pub(crate) fn spawn_bounded_pipe_reader<R: std::io::Read + Send + 'static>(
    mut reader: R,
    limit: usize,
    overflowed: Arc<AtomicBool>,
) -> mpsc::Receiver<Result<Vec<u8>, PipeCaptureErr>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => {
                    let _ = tx.send(Ok(out));
                    return;
                }
                Ok(n) => {
                    if out.len().saturating_add(n) > limit {
                        overflowed.store(true, Ordering::Release);
                        drop(reader);
                        let _ = tx.send(Err(PipeCaptureErr::Overflow));
                        return;
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
    });
    rx
}

pub(crate) fn recv_bounded_pipe(
    rx: mpsc::Receiver<Result<Vec<u8>, PipeCaptureErr>>,
    deadline: Option<std::time::Instant>,
    label: &str,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let res = match deadline {
        Some(dl) => {
            let rem = dl
                .checked_duration_since(std::time::Instant::now())
                .ok_or_else(|| format!("{label}: exceeded build time limit"))?;
            match rx.recv_timeout(rem) {
                Ok(r) => r,
                Err(RecvTimeoutError::Timeout) => return Err(format!("{label}: exceeded build time limit")),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(format!("{label}: subprocess output reader disconnected"));
                }
            }
        }
        None => rx
            .recv()
            .map_err(|_| format!("{label}: subprocess output reader disconnected"))?,
    };
    match res {
        Ok(buf) => Ok(buf),
        Err(PipeCaptureErr::Overflow) => Err(format!("{label}: exceeded subprocess output cap ({limit} bytes)")),
        Err(PipeCaptureErr::Io(e)) => Err(format!("{label}: reading subprocess output: {e}")),
    }
}

/// Runs `cmd`, capturing stdout/stderr through bounded in-memory pipes (zero disk amplification,
/// hard `MAX_CAPTURE_BYTES` ceiling with immediate `SIGPIPE` + tree kill on overflow, concurrent
/// draining preventing pipe deadlocks, and deadline-bounded `recv_timeout` preventing descendant
/// EOF hangs) while propagating every read failure.
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
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    if !inherit_pgid() {
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    }
    let mut child = cmd.spawn().map_err(|e| format!("{label}: {e}"))?;
    let pid = child.id();
    let overflowed = Arc::new(AtomicBool::new(false));
    let rx_out = spawn_bounded_pipe_reader(
        child.stdout.take().ok_or_else(|| format!("{label}: missing stdout pipe"))?,
        MAX_CAPTURE_BYTES,
        Arc::clone(&overflowed),
    );
    let rx_err = spawn_bounded_pipe_reader(
        child.stderr.take().ok_or_else(|| format!("{label}: missing stderr pipe"))?,
        MAX_CAPTURE_BYTES,
        Arc::clone(&overflowed),
    );
    let status = loop {
        if overflowed.load(Ordering::Acquire) {
            terminate_child_tree(&mut child);
            return Err(format!("{label}: exceeded subprocess output cap ({MAX_CAPTURE_BYTES} bytes)"));
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
    reap_child_group_after_exit(pid);
    if overflowed.load(Ordering::Acquire) {
        return Err(format!("{label}: exceeded subprocess output cap ({MAX_CAPTURE_BYTES} bytes)"));
    }
    let stdout = recv_bounded_pipe(rx_out, deadline, label, MAX_CAPTURE_BYTES)?;
    let stderr = recv_bounded_pipe(rx_err, deadline, label, MAX_CAPTURE_BYTES)?;
    if let Some(dl) = deadline
        && std::time::Instant::now() >= dl
    {
        return Err(format!("{label}: exceeded build time limit"));
    }
    Ok(std::process::Output { status, stdout, stderr })
}

/// Returns the current effective UID on Unix, failing closed if unavailable.
/// On Linux this reads `/proc/thread-self/status` (falling back to `/proc/self/status` only on
/// `NotFound`) and parses `Uid:` field 2 (`euid`), staying exact for the calling thread even
/// under `PR_SET_DUMPABLE=0` (where `/proc/self` metadata reports `st_uid == 0`).
#[cfg(unix)]
fn current_euid() -> Result<u32, String> {
    #[cfg(target_os = "linux")]
    {
        let status = match fs::read_to_string("/proc/thread-self/status") {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => fs::read_to_string("/proc/self/status")
                .map_err(|e2| format!("reading /proc/self/status for effective uid: {e2}"))?,
            Err(e) => return Err(format!("reading /proc/thread-self/status for effective uid: {e}")),
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
        path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))
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
                    return Err(format!("{}: cache path must not contain '..' components", path.display()));
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
        let canon_existing = existing.canonicalize().map_err(|e| format!("{}: {e}", existing.display()))?;
        if missing.is_empty() {
            if canon_existing.parent().is_none() {
                return Err(format!("{}: refusing to use filesystem root as cache directory", path.display()));
            }
            let sym = fs::symlink_metadata(&existing).map_err(|e| format!("{}: {e}", existing.display()))?;
            if sym.file_type().is_symlink() || !sym.file_type().is_dir() {
                return Err(format!("{}: cache path must be a real directory (not a symlink)", path.display()));
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
                fs::set_permissions(&canon_existing, fs::Permissions::from_mode(0o700))
                    .map_err(|e| format!("{}: cannot enforce owner-only 0700 cache permissions: {e}", path.display()))?;
            }
            let post = fs::symlink_metadata(&canon_existing).map_err(|e| format!("{}: {e}", canon_existing.display()))?;
            if post.file_type().is_symlink()
                || !post.file_type().is_dir()
                || post.uid() != euid
                || post.permissions().mode() & 0o077 != 0
                || post.dev() != sym.dev()
                || post.ino() != sym.ino()
            {
                return Err(format!("{}: cache directory failed post-validation identity/mode checks", path.display()));
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
                    let pre = fs::symlink_metadata(&cur).map_err(|e| format!("{}: {e}", cur.display()))?;
                    if pre.file_type().is_symlink()
                        || !pre.file_type().is_dir()
                        || pre.uid() != euid
                        || pre.permissions().mode() & 0o022 != 0
                    {
                        return Err(format!("{}: concurrently created cache dir failed ownership/mode checks", cur.display()));
                    }
                    if pre.permissions().mode() & 0o077 != 0 {
                        fs::set_permissions(&cur, fs::Permissions::from_mode(0o700))
                            .map_err(|e| format!("{}: cannot enforce 0700 permissions: {e}", cur.display()))?;
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
                return Err(format!("{}: created cache directory failed 0700 ownership checks", cur.display()));
            }
        }
        let canon_leaf = cur.canonicalize().map_err(|e| format!("{}: {e}", cur.display()))?;
        if canon_leaf != cur {
            return Err(format!("{}: cache directory changed path during creation", cur.display()));
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
pub(crate) fn publish_digest_sidecar(entry: &Path, artifact_name: &str, sha: &str, digest_file: &Path) -> Result<(), String> {
    use std::io::Write as _;
    let seq = NEXT_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let staged_sha = entry.join(format!("{artifact_name}.sha256.{}.{seq}", std::process::id()));
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
        .and_then(|()| fs::rename(&staged_sha, digest_file).map_err(|e| format!("{}: {e}", digest_file.display())))
    {
        let _ = fs::remove_file(&staged_sha);
        return Err(e);
    }
    Ok(())
}

/// Acquires an advisory lock (`exclusive` or shared) on `lock_path` within `deadline`.
fn lock_file_mode_with_deadline(
    lock_path: &Path,
    exclusive: bool,
    deadline: Option<std::time::Instant>,
) -> Result<fs::File, String> {
    let lock_file = fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|e| format!("{}: {e}", lock_path.display()))?;
    loop {
        let res = if exclusive {
            lock_file.try_lock()
        } else {
            lock_file.try_lock_shared()
        };
        match res {
            Ok(()) => return Ok(lock_file),
            Err(fs::TryLockError::WouldBlock) => {
                if let Some(dl) = deadline
                    && std::time::Instant::now() >= dl
                {
                    return Err(format!("timed out waiting for publication lock {}", lock_path.display()));
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(fs::TryLockError::Error(e)) => {
                return Err(format!("locking {}: {e}", lock_path.display()));
            }
        }
    }
}

/// Acquires an exclusive advisory lock on `lock_path` within `deadline`.
pub(crate) fn lock_file_with_deadline(lock_path: &Path, deadline: Option<std::time::Instant>) -> Result<fs::File, String> {
    lock_file_mode_with_deadline(lock_path, true, deadline)
}

/// Acquires a shared advisory lock on `lock_path` within `deadline`.
pub(crate) fn lock_file_shared_with_deadline(
    lock_path: &Path,
    deadline: Option<std::time::Instant>,
) -> Result<fs::File, String> {
    lock_file_mode_with_deadline(lock_path, false, deadline)
}

/// Acquires an exclusive publication lock on `entry/.publish.lock` within `deadline`, returning
/// the lock file and the verified canonical entry directory.
fn lock_cache_entry(entry: &Path, deadline: std::time::Instant) -> Result<(fs::File, PathBuf), String> {
    let canon_entry = ensure_private_dir(entry)?;
    let lock_path = canon_entry.join(".publish.lock");
    let lock_file = lock_file_with_deadline(&lock_path, Some(deadline))?;
    Ok((lock_file, canon_entry))
}

/// Verifies `artifact` is a non-symlink immutable regular file owned by `euid` matching its
/// `.sha256` sidecar, returning the exact verified bytes so callers avoid any post-check TOCTOU.
pub(crate) fn verified_cached_artifact_bytes(artifact: &Path, digest_file: &Path) -> Option<Vec<u8>> {
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
    (crate::sha256::hex(&bytes) == expected.trim()).then_some(bytes)
}

/// Verifies `artifact` is a non-symlink immutable regular file matching its `.sha256` sidecar.
fn verified_cached_artifact(artifact: &Path, digest_file: &Path) -> bool {
    verified_cached_artifact_bytes(artifact, digest_file).is_some()
}

/// Fast C99 + parallel `clang` + `ld.lld` build path for Blot ABI 8 cdylibs.
fn build_c(module: &Path, cache: &Path, options: &Options, deadline: std::time::Instant) -> Result<PathBuf, String> {
    check_deadline(deadline, options.time_limit)?;
    let wasm = read_bounded_wasm(module, MAX_WASM_BYTES_C_RUST)?;
    check_deadline(deadline, options.time_limit)?;
    let cache = ensure_private_dir(cache)?;
    let generator = std::env::current_exe().map_err(|e| format!("this generator: {e}"))?;
    let gen_bytes = fs::read(&generator).map_err(|e| format!("{}: {e}", generator.display()))?;
    let cc_info = crate::cgen::detect_cc_with_deadline(options.target_cpu, Some(&cache), Some(deadline))?;
    let cc_target = crate::cgen::cc_target_fingerprint_with_deadline(&cc_info, options.target_cpu, Some(deadline))?;
    check_deadline(deadline, options.time_limit)?;

    let mut key = Sha256::new();
    field(&mut key, "wtime-aot build c 7", &[]);
    field(&mut key, "wasm", &wasm);
    field(&mut key, "generator", &gen_bytes);
    field(&mut key, "cc", cc_info.version.as_bytes());
    field(&mut key, "cc sha256", cc_info.cc_sha256.as_bytes());
    field(&mut key, "sys headers sha256", cc_info.sys_headers_sha256.as_bytes());
    field(&mut key, "linker sha256", cc_info.linker_sha256.as_bytes());
    field(&mut key, "link env sha256", cc_info.link_env_sha256.as_bytes());
    field(&mut key, "target cpu", options.target_cpu.as_bytes());
    field(&mut key, "cc target fingerprint", cc_target.as_bytes());
    field(&mut key, "cet return", &[cc_info.has_cet_return as u8]);
    field(&mut key, "stack clash", &[cc_info.has_stack_clash as u8]);
    field(&mut key, "c opt", options.c_opt.tag().as_bytes());
    let key_hex = key.hex();

    let (_lock, entry) = lock_cache_entry(&cache.join(&key_hex), deadline)?;
    check_deadline(deadline, options.time_limit)?;
    let program = entry.join("library.so");
    let digest_file = entry.join("library.so.sha256");
    if verified_cached_artifact(&program, &digest_file) {
        check_deadline(deadline, options.time_limit)?;
        return Ok(program);
    }
    let _ = fs::remove_file(&program);
    let _ = fs::remove_file(&digest_file);
    check_deadline(deadline, options.time_limit)?;
    let seq = NEXT_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let staged = entry.join(format!("library.so.{}.{seq}", std::process::id()));
    let res = crate::cgen::build_c_blot_cdylib_with_info(
        &wasm,
        &cache,
        options.target_cpu,
        options.jobs,
        options.c_opt,
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
    let stored = work_root.join(format!("profile-{digest}-{}-{seq}.profdata", std::process::id()));
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
    let read = |path: &Path| fs::read(path).map_err(|e| format!("{}: {e}", path.display()));
    let wasm = read_bounded_wasm(module, MAX_WASM_BYTES_C_RUST)?;
    check_deadline(deadline, options.time_limit)?;
    // The program reports the module's own name, not where the operator
    // keeps it.
    let module_path = match module_path {
        Some(path) => path.to_string(),
        None => module.file_name().map_or("module.wasm".into(), |name| name.to_string_lossy().into_owned()),
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
    let version = rustc_version(options.toolchain, Some(deadline))?;
    field(&mut key, "toolchain", version.as_bytes());
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
    let (_lock, entry) = lock_cache_entry(&cache.join(&key), deadline)?;
    check_deadline(deadline, options.time_limit)?;
    let artifact_name = match options.target {
        crate::codegen::Target::GooseBin => "program",
        crate::codegen::Target::BlotCdylib => "library.so",
    };
    let program = entry.join(artifact_name);
    let digest_file = entry.join(format!("{artifact_name}.sha256"));
    if verified_cached_artifact(&program, &digest_file) {
        check_deadline(deadline, options.time_limit)?;
        return Ok(program);
    }
    let _ = fs::remove_file(&program);
    let _ = fs::remove_file(&digest_file);
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
    let result = generate_and_compile(prepared, &work, &name, rustflags, options, deadline).and_then(|built| {
        check_deadline(deadline, options.time_limit)?;
        let staged = entry.join(format!("{artifact_name}.{}.{seq}", std::process::id()));
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
    let mut sysroot_cmd = clean_rustc_command(options.toolchain);
    sysroot_cmd.args(["--print", "sysroot"]);
    let sysroot = run_cmd_capture_with_deadline(sysroot_cmd, "rustc --print sysroot", Some(deadline))?;
    if !sysroot.status.success() {
        return Err(format!("rustc --print sysroot failed ({})", sysroot.status));
    }
    let sysroot = PathBuf::from(String::from_utf8_lossy(&sysroot.stdout).trim());
    let host = rustc_version(options.toolchain, Some(deadline))?
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
    let pid = child.id();
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
    reap_child_group_after_exit(pid);
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
    let Options { toolchain, target_cpu, jobs, time_limit, target: target_kind, .. } = *options;
    check_deadline(deadline, time_limit)?;
    crate::codegen::generate_target(&prepared.wasm, work, name, &prepared.rt, &prepared.module_path, prepared.flat64, target_kind)?;
    check_deadline(deadline, time_limit)?;
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
    if let Some(jobs) = jobs.filter(|&j| j > 0) {
        cargo.args(["--jobs", &jobs.to_string()]);
    }
    // Run in a dedicated process group unless an outer caller already manages our group.
    #[cfg(unix)]
    if !inherit_pgid() {
        std::os::unix::process::CommandExt::process_group(&mut cargo, 0);
    }
    let mut child = cargo.spawn().map_err(|e| format!("cargo: {e}"))?;
    let pid = child.id();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    terminate_child_tree(&mut child);
                    return Err(format!("the build exceeded its time limit of {:?}", time_limit));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => {
                terminate_child_tree(&mut child);
                return Err(format!("cargo: {e}"));
            }
        }
    };
    reap_child_group_after_exit(pid);
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

fn clean_rustc_command(toolchain: &str) -> Command {
    let mut cmd = Command::new("rustc");
    cmd.env_clear();
    for variable in ["PATH", "HOME", "RUSTUP_HOME", "CARGO_HOME"] {
        if let Some(value) = std::env::var_os(variable) {
            cmd.env(variable, value);
        }
    }
    cmd.env("RUSTUP_TOOLCHAIN", toolchain);
    cmd
}

/// The target features `target_cpu` resolves to on this machine
/// (`rustc --print cfg`).
fn target_features(
    toolchain: &str,
    target_cpu: &str,
    deadline: Option<std::time::Instant>,
) -> Result<String, String> {
    let mut cfg_cmd = clean_rustc_command(toolchain);
    cfg_cmd.args(["--print", "cfg", "-C", &format!("target-cpu={target_cpu}")]);
    let output = run_cmd_capture_with_deadline(cfg_cmd, "rustc --print cfg", deadline)?;
    if !output.status.success() {
        return Err(format!("target cpu {target_cpu}: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    let mut features = String::from_utf8_lossy(&output.stdout).into_owned();
    // `--print cfg` lists stable features only: the CPU LLVM resolves
    // `native` to covers the rest.
    let mut cpus_cmd = clean_rustc_command(toolchain);
    cpus_cmd.args(["--print", "target-cpus", "-C", &format!("target-cpu={target_cpu}")]);
    let cpus = run_cmd_capture_with_deadline(cpus_cmd, "rustc --print target-cpus", deadline)?;
    let listing = String::from_utf8_lossy(&cpus.stdout);
    if let Some(current) = listing.lines().find(|line| line.contains("currently")) {
        features.push_str(current.trim());
    }
    Ok(features)
}

/// `rustc -vV` of the toolchain.
fn rustc_version(
    toolchain: &str,
    deadline: Option<std::time::Instant>,
) -> Result<String, String> {
    let mut cmd = clean_rustc_command(toolchain);
    cmd.arg("-vV");
    let output = run_cmd_capture_with_deadline(cmd, "rustc -vV", deadline)?;
    if !output.status.success() {
        return Err(format!("toolchain {toolchain}: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
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
    let base = std::env::temp_dir().canonicalize().expect("canonicalize temp_dir");
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
            Err(e) => panic!("failed to create fresh test root {}: {e}", candidate.display()),
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
        assert!(!meta.file_type().is_symlink(), "test output {} must not be a symlink", path.display());
        let canon = path.canonicalize().expect("canonicalize existing test path");
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
    let canon_existing = existing.canonicalize().expect("canonicalize existing parent");
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
            let mode = fs::symlink_metadata(&sha_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o444, "sidecar must be immutable 0o444 regardless of umask");
        }
        assert_eq!(verified_cached_artifact_bytes(&art, &sha_path).as_deref(), Some(payload.as_slice()));
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
}
