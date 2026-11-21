//! The imports of a direct Goose module (goose-to-wasm) for code compiled
//! ahead of time, and the driver of such a program.
//!
//! A port of `tools/wtime-goose/src/host.rs` (itself a port of goose-to-wasm's
//! Wasmtime host): the same imports, output, exit codes and timing JSON, with
//! the module reached through `Guest` instead of the interpreter's store.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::io::{Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::float::{f32_text, f64_text};
use crate::pytext::{normalized_path, parse_float, parse_int};
use crate::{LinearMemory, R, Trap, Val};

/// What a run keeps outside the module.
pub struct State {
    input: Vec<u8>,
    token_at: usize,
    input_position: usize,
    pub out: Vec<u8>,
    pub err: Vec<u8>,
    arguments: Vec<Vec<u8>>,
    /// Set when the program ended itself with this code.
    pub exit: Option<i64>,
    /// Set by a failure of the host's own, reported by its text alone.
    pub failure: Option<String>,
    clock_origin: Instant,
    policy: Policy,
    /// Bytes written to standard output, standard error and files so far.
    written: u64,
    /// The host is inside `goose.reserve`: the module may not call the host
    /// back into another reservation (or another file read).
    reserving: bool,
}

/// The default of `Policy::output_limit`.
pub const OUTPUT_LIMIT: u64 = 256 << 20;

/// What the module may reach outside itself beyond its input, output,
/// arguments and the clock. Nothing by default: the operator grants
/// directories and environment variables (`--allow-files`, `--allow-env`).
/// A denied file is reported as missing (or unwritable), a denied variable
/// as unset, as the module would see them on a machine without them.
#[derive(Clone, Debug)]
pub struct Policy {
    /// Canonical directories whose files (and subdirectories') the module
    /// may read, write and test.
    pub directories: Vec<PathBuf>,
    /// Variables the module may read; `None` allows every variable.
    pub variables: Option<Vec<OsString>>,
    /// The most bytes the module may write to standard output, standard
    /// error and files together, and read from one file; past it, writing
    /// fails with `ResourceLimit: output limit` and a larger file looks
    /// unreadable.
    pub output_limit: u64,
}

impl Default for Policy {
    /// Nothing granted.
    fn default() -> Policy {
        Policy { directories: Vec::new(), variables: Some(Vec::new()), output_limit: OUTPUT_LIMIT }
    }
}

impl Policy {
    /// Grants the files under `directory`.
    pub fn allow_directory(&mut self, directory: &Path) -> std::io::Result<()> {
        self.directories.push(directory.canonicalize()?);
        Ok(())
    }

    /// The file the module names by `path` (relative to the working
    /// directory), resolved through symbolic links, if it lies under an
    /// allowed directory. A file that does not exist yet resolves through
    /// its parent directory, for writing; its own name must be a plain one.
    /// (Another local process swapping links between this check and the
    /// access is not guarded against.)
    fn file(&self, path: &[u8]) -> Option<PathBuf> {
        if self.directories.is_empty() {
            return None;
        }
        let directory = path.ends_with(b"/") || path.ends_with(b"/.");
        let path = Path::new(OsStr::from_bytes(path));
        // First by its text alone, so that nothing outside the granted
        // directories is even looked at: no `..`, and the path made absolute
        // against the working directory lies under a granted directory.
        if path.components().any(|component| component == Component::ParentDir) {
            return None;
        }
        let absolute = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().ok()?.join(path) };
        let lexical: PathBuf = absolute.components().filter(|component| *component != Component::CurDir).collect();
        if !self.directories.iter().any(|granted| lexical.starts_with(granted)) {
            return None;
        }
        // A trailing `/` (or `/.`) names a directory, which must exist.
        if directory && !path.is_dir() {
            return None;
        }
        let resolved = match path.canonicalize() {
            Ok(resolved) => resolved,
            // A link that cannot be resolved (one pointing at a file that
            // does not exist yet, anywhere) is refused: writing through it
            // would create its target.
            Err(_) if fs::symlink_metadata(path).is_ok() => return None,
            Err(_) => {
                let name = match path.components().next_back()? {
                    Component::Normal(name) => name,
                    _ => return None,
                };
                let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."));
                parent.canonicalize().ok()?.join(name)
            }
        };
        self.directories.iter().any(|directory| resolved.starts_with(directory)).then_some(resolved)
    }

    fn variable(&self, name: &[u8]) -> bool {
        match &self.variables {
            None => true,
            Some(names) => names.iter().any(|allowed| allowed.as_bytes() == name),
        }
    }
}

impl State {
    pub fn new(input: Vec<u8>, arguments: Vec<Vec<u8>>) -> State {
        State {
            input,
            token_at: 0,
            input_position: 0,
            out: Vec::new(),
            err: Vec::new(),
            arguments,
            exit: None,
            failure: None,
            clock_origin: Instant::now(),
            policy: Policy::default(),
            written: 0,
            reserving: false,
        }
    }

    /// The state with `policy` in force.
    pub fn with_policy(mut self, policy: Policy) -> State {
        self.policy = policy;
        self
    }

    fn token(&mut self) -> R<&[u8]> {
        let space = |byte: &u8| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
        let rest = &self.input[self.token_at..];
        let Some(start) = rest.iter().position(|byte| !space(byte)) else {
            self.token_at = self.input.len();
            return Err(Trap::StopIteration);
        };
        let start = self.token_at + start;
        let end = self.input[start..].iter().position(space).map_or(self.input.len(), |length| start + length);
        self.token_at = end;
        Ok(&self.input[start..end])
    }
}

thread_local! {
    static STACK_FLOOR: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The lowest stack address guest code may use on this thread (0 where
/// `main` did not start it): generated functions trap with
/// `ResourceLimit: call depth exhausted` below it.
pub fn stack_floor() -> usize {
    STACK_FLOOR.with(|floor| floor.get())
}

/// Instantiates a compiled module (see `main`).
pub type Instantiate<G> = fn(&mut State) -> R<G>;

/// A compiled module as the host and the driver see it.
pub trait Guest {
    /// The exported memory, if any.
    fn memory(&mut self) -> Option<&mut dyn LinearMemory>;
    fn state(&mut self) -> &mut State;
    /// The exported `goose.reserve(control, count) -> address`, if any.
    fn reserve(&mut self, control: i64, count: i64) -> Option<R<i64>>;
    /// Runs the exported `_start`.
    fn start(&mut self) -> R<()>;
}

/// What a handler returns, fitted by the caller to the import's result type.
pub enum Out {
    Nothing,
    Integer(i64),
    Float(f64),
}

impl Out {
    pub fn i32(self) -> R<i32> {
        match self {
            Out::Integer(value) => Ok(value as i32),
            _ => Err(Trap::ResultMismatch),
        }
    }
    pub fn i64(self) -> R<i64> {
        match self {
            Out::Integer(value) => Ok(value),
            _ => Err(Trap::ResultMismatch),
        }
    }
    pub fn f32(self) -> R<f32> {
        match self {
            Out::Float(value) => Ok(value as f32),
            _ => Err(Trap::ResultMismatch),
        }
    }
    pub fn f64(self) -> R<f64> {
        match self {
            Out::Float(value) => Ok(value),
            _ => Err(Trap::ResultMismatch),
        }
    }
    pub fn none(self) -> R<()> {
        match self {
            Out::Nothing => Ok(()),
            _ => Err(Trap::ExtraResults),
        }
    }
}

pub type Handler = fn(&mut dyn Guest, &[Val]) -> R<Out>;

fn fail(guest: &mut dyn Guest, text: impl Into<String>) -> Trap {
    let text = text.into();
    guest.state().failure.get_or_insert(text);
    Trap::Host
}

fn read(guest: &mut dyn Guest, pointer: i128, length: i128) -> R<Vec<u8>> {
    if pointer < 0 || length < 0 {
        return Err(fail(guest, "negative host address"));
    }
    let Some(memory) = guest.memory() else {
        return Err(fail(guest, "host memory is unavailable during module initialization"));
    };
    let (Ok(pointer), Ok(length)) = (u64::try_from(pointer), u64::try_from(length)) else {
        return Err(fail(guest, "host write is outside exported memory"));
    };
    match memory.read(pointer, length) {
        Ok(bytes) => Ok(bytes),
        Err(_) => Err(fail(guest, "host write is outside exported memory")),
    }
}

fn write(guest: &mut dyn Guest, pointer: i128, bytes: &[u8]) -> R<()> {
    if pointer < 0 {
        return Err(fail(guest, "negative host address"));
    }
    let Some(memory) = guest.memory() else {
        return Err(fail(guest, "host memory is unavailable during module initialization"));
    };
    let Ok(pointer) = u64::try_from(pointer) else {
        return Err(fail(guest, "host write is outside exported memory"));
    };
    match memory.write(pointer, bytes) {
        Ok(()) => Ok(()),
        Err(_) => Err(fail(guest, "host write is outside exported memory")),
    }
}

fn write64(guest: &mut dyn Guest, pointer: i64, value: i128) -> R<()> {
    let Ok(value) = u64::try_from(value) else {
        return Err(fail(guest, "'Q' format requires 0 <= number <= 18446744073709551615"));
    };
    write(guest, pointer.into(), &value.to_le_bytes())
}

fn print(guest: &mut dyn Guest, text: &[u8]) -> R<Out> {
    charge(guest, text.len() as u64)?;
    guest.state().out.extend_from_slice(text);
    Ok(Out::Nothing)
}

/// Counts `n` more bytes of output (standard output, standard error, files)
/// against the output limit, before they are read from the module.
fn charge(guest: &mut dyn Guest, n: u64) -> R<()> {
    let state = guest.state();
    match state.written.checked_add(n).filter(|&total| total <= state.policy.output_limit) {
        Some(total) => {
            state.written = total;
            Ok(())
        }
        None => Err(fail(guest, "ResourceLimit: output limit")),
    }
}

fn exit(guest: &mut dyn Guest, values: &[Val]) -> R<Out> {
    let code = integer(guest, values, 0)?;
    guest.state().exit = Some(code);
    Err(Trap::Exit)
}

/// The handler of import `module.name`, if this host has one.
pub fn handler(module: &str, name: &str) -> Option<Handler> {
    let handler: Handler = match (module, name) {
        ("goose", "write") => |g, values| {
            let (pointer, length) = (integer(g, values, 0)?, integer(g, values, 1)?);
            let text = read(g, address(pointer), address(length))?;
            print(g, &text)
        },
        ("goose", "print_i64") => |g, values| {
            let text = integer(g, values, 0)?.to_string();
            print(g, text.as_bytes())
        },
        ("goose", "print_u64") => |g, values| {
            let text = (integer(g, values, 0)? as u64).to_string();
            print(g, text.as_bytes())
        },
        ("goose", "print_f64") => |g, values| {
            let text = f64_text(float(g, values, 0)?);
            print(g, text.as_bytes())
        },
        ("goose", "print_f32") => |g, values| {
            let text = f32_text(float(g, values, 0)? as f32);
            print(g, text.as_bytes())
        },
        ("goose", "format_f64") => |g, values| {
            let text = f64_text(float(g, values, 1)?);
            format_float(g, values, text)
        },
        ("goose", "format_f32") => |g, values| {
            let text = f32_text(float(g, values, 1)? as f32);
            format_float(g, values, text)
        },
        ("goose", "print_bool") => |g, values| {
            let truth = truth(g, values, 0)?;
            print(g, if truth { b"true" } else { b"false" })
        },
        ("goose", "newline") => |g, _| print(g, b"\n"),
        ("goose", "abort") => |g, values| {
            let (pointer, length, line) = (integer(g, values, 0)?, integer(g, values, 1)?, integer(g, values, 2)?);
            let message = read(g, address(pointer), address(length))?;
            Err(fail(g, format!("goose runtime error: {} (line {line})", String::from_utf8_lossy(&message))))
        },
        ("goose", "exit") | ("wasi_snapshot_preview1", "proc_exit") => exit,
        ("env", "bench_input") => |g, _| {
            let parsed = g.state().token().map(parse_int)?;
            parsed.map(Out::Integer).map_err(|text| fail(g, text))
        },
        ("env", "bench_input_f64") => |g, _| {
            let parsed = g.state().token().map(parse_float)?;
            parsed.map(Out::Float).map_err(|text| fail(g, text))
        },
        ("env", "bench_clock") | ("env", "gs_os_clock_ns") => |g, _| Ok(Out::Integer(monotonic_ns(g))),
        ("env", "bench_barrier") => |g, values| match values.first() {
            Some(Val::F32(_) | Val::F64(_)) => Ok(Out::Float(float(g, values, 0)?)),
            _ => Ok(Out::Integer(integer(g, values, 0)?)),
        },
        ("env", "bench_sink") | ("env", "emscripten_notify_memory_growth") | ("env", "gs_os_flush_stdout") => {
            |_, _| Ok(Out::Nothing)
        }
        ("env", "gs_os_read_file") => os_read_file,
        ("env", "gs_os_write_file") => |g, values| os_write_file(g, values, false),
        ("env", "gs_os_append_file") => |g, values| os_write_file(g, values, true),
        ("env", "gs_os_file_exists") => |g, values| os_stat(g, values, |kind| kind.is_file()),
        ("env", "gs_os_is_dir") => |g, values| os_stat(g, values, |kind| kind.is_dir()),
        ("env", "gs_os_write_stdout") => |g, values| os_write(g, values, false),
        ("env", "gs_os_write_stderr") => |g, values| os_write(g, values, true),
        ("env", "gs_os_arg_count") => |g, _| Ok(Out::Integer(g.state().arguments.len() as i64)),
        ("env", "gs_os_arg") => os_arg,
        ("env", "gs_os_getenv") => os_getenv,
        ("env", "gs_os_resource_dir") => |_, _| Ok(Out::Integer(0)),
        ("env", "gs_os_time_ns") => |_, _| Ok(Out::Integer(wall_ns())),
        ("env", "gs_find_any") => find_any,
        ("env", "gs_find_pair") => find_pair,
        ("env", "system") => |_, _| Ok(Out::Integer(-1)),
        ("wasi_snapshot_preview1", "args_sizes_get") => |g, values| {
            let (count, size) = (integer(g, values, 0)?, integer(g, values, 1)?);
            write64(g, count, 1)?;
            write64(g, size, 8)?;
            Ok(Out::Integer(0))
        },
        ("wasi_snapshot_preview1", "args_get") => |g, values| {
            let (argv, buffer) = (integer(g, values, 0)?, integer(g, values, 1)?);
            write64(g, argv, buffer.into())?;
            write(g, buffer.into(), b"program\0")?;
            Ok(Out::Integer(0))
        },
        ("wasi_snapshot_preview1", "environ_sizes_get") => |g, values| {
            let (count, size) = (integer(g, values, 0)?, integer(g, values, 1)?);
            write64(g, count, 0)?;
            write64(g, size, 0)?;
            Ok(Out::Integer(0))
        },
        ("wasi_snapshot_preview1", "environ_get") => |_, _| Ok(Out::Integer(0)),
        ("wasi_snapshot_preview1", "clock_time_get") => |g, values| {
            let value = match integer(g, values, 0)? {
                0 => wall_ns(),
                1 => monotonic_ns(g),
                _ => return Ok(Out::Integer(28)),
            };
            let out = integer(g, values, 2)?;
            write64(g, out, value.into())?;
            Ok(Out::Integer(0))
        },
        ("wasi_snapshot_preview1", "fd_write") => fd_write,
        ("wasi_snapshot_preview1", "fd_read") => fd_read,
        ("wasi_snapshot_preview1", "fd_close") => {
            |g, values| Ok(Out::Integer(if (0..=2).contains(&integer(g, values, 0)?) { 0 } else { 8 }))
        }
        ("wasi_snapshot_preview1", "fd_seek") => {
            |g, values| Ok(Out::Integer(if (0..=2).contains(&integer(g, values, 0)?) { 70 } else { 8 }))
        }
        _ => return None,
    };
    Some(handler)
}

// Arguments.

fn integer(guest: &mut dyn Guest, values: &[Val], index: usize) -> R<i64> {
    match values.get(index) {
        Some(Val::I32(value)) => Ok(i64::from(*value)),
        Some(Val::I64(value)) => Ok(*value),
        Some(_) => Err(fail(guest, "host import declared with a parameter that is not an integer")),
        None => Err(fail(guest, "host import declared with too few parameters")),
    }
}

fn float(guest: &mut dyn Guest, values: &[Val], index: usize) -> R<f64> {
    match values.get(index) {
        Some(Val::F32(value)) => Ok(f64::from(*value)),
        Some(Val::F64(value)) => Ok(*value),
        Some(_) => Err(fail(guest, "host import declared with a parameter that is not a float")),
        None => Err(fail(guest, "host import declared with too few parameters")),
    }
}

fn truth(guest: &mut dyn Guest, values: &[Val], index: usize) -> R<bool> {
    match values.get(index) {
        Some(Val::F32(_) | Val::F64(_)) => Ok(float(guest, values, index)? != 0.0),
        _ => Ok(integer(guest, values, index)? != 0),
    }
}

/// The arguments of an import that takes slices: one packed i64 (pointer in
/// the low 32 bits, count in the high) in a 32-bit module, two i64 in a
/// 64-bit one.
struct Arguments<'v> {
    values: &'v [Val],
    at: usize,
    wide: bool,
}

impl<'v> Arguments<'v> {
    fn new(values: &'v [Val], kinds: usize) -> Arguments<'v> {
        Arguments { values, at: 0, wide: values.len() > kinds }
    }

    fn integer(&mut self, guest: &mut dyn Guest) -> R<i64> {
        self.at += 1;
        integer(guest, self.values, self.at - 1)
    }

    fn slice(&mut self, guest: &mut dyn Guest) -> R<(i128, i128)> {
        if self.wide {
            Ok((self.integer(guest)?.into(), self.integer(guest)?.into()))
        } else {
            let packed = self.integer(guest)? as u64;
            Ok(((packed & 0xffff_ffff).into(), (packed >> 32).into()))
        }
    }
}

fn address(value: i64) -> i128 {
    if value < 0 { (value & 0xffff_ffff).into() } else { value.into() }
}

// The goose module.

fn format_float(guest: &mut dyn Guest, values: &[Val], text: String) -> R<Out> {
    if text.len() > 32 {
        return Err(fail(guest, "formatted float exceeds host buffer"));
    }
    let pointer = address(integer(guest, values, 0)?);
    // The whole 32-byte buffer must be inside memory, as the other hosts check.
    read(guest, pointer, 32)?;
    write(guest, pointer, text.as_bytes())?;
    Ok(Out::Integer(text.len() as i64))
}

// The stdlib's os externs.

fn os_read_file(guest: &mut dyn Guest, values: &[Val]) -> R<Out> {
    let mut arguments = Arguments::new(values, 2);
    let (pointer, length) = arguments.slice(guest)?;
    let control = arguments.integer(guest)?;
    let path = read(guest, pointer, length)?;
    if path.contains(&0) {
        return Err(fail(guest, "embedded null byte"));
    }
    if guest.state().reserving {
        return Err(fail(guest, "goose.reserve called back into the host"));
    }
    let Some(file) = guest.state().policy.file(&normalized_path(&path)) else { return Ok(Out::Integer(0)) };
    // At most the output limit: a larger file is reported unreadable.
    let limit = guest.state().policy.output_limit;
    let read = fs::File::open(file).and_then(|file| {
        let mut contents = Vec::new();
        file.take(limit.saturating_add(1)).read_to_end(&mut contents)?;
        Ok(contents)
    });
    match read.ok().filter(|contents| contents.len() as u64 <= limit).ok_or(()) {
        Ok(contents) => {
            append_bytes(guest, control, &contents)?;
            Ok(Out::Integer(1))
        }
        Err(_) => Ok(Out::Integer(0)),
    }
}

fn os_write_file(guest: &mut dyn Guest, values: &[Val], append: bool) -> R<Out> {
    let mut arguments = Arguments::new(values, 2);
    let (path_pointer, path_length) = arguments.slice(guest)?;
    let (pointer, length) = arguments.slice(guest)?;
    let path = read(guest, path_pointer, path_length)?;
    charge(guest, u64::try_from(length).unwrap_or(u64::MAX))?;
    let contents = read(guest, pointer, length)?;
    if path.contains(&0) {
        return Err(fail(guest, "embedded null byte"));
    }
    let mut options = fs::OpenOptions::new();
    if append {
        options.append(true).create(true);
    } else {
        options.write(true).create(true).truncate(true);
    }
    let Some(file) = guest.state().policy.file(&path) else { return Ok(Out::Integer(0)) };
    // The checked path is opened without following a link in its last
    // component, should one appear there after the check.
    no_follow(&mut options);
    let written = options.open(file).and_then(|mut file| file.write_all(&contents));
    Ok(Out::Integer(written.is_ok().into()))
}

/// O_NOFOLLOW, where its value is known.
fn no_follow(options: &mut fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    #[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "x86")))]
    options.custom_flags(0o400000);
    #[cfg(all(target_os = "linux", any(target_arch = "aarch64", target_arch = "arm")))]
    options.custom_flags(0o100000);
    #[cfg(target_os = "macos")]
    options.custom_flags(0x100);
    let _ = options;
}

fn os_stat(guest: &mut dyn Guest, values: &[Val], is: fn(&fs::Metadata) -> bool) -> R<Out> {
    let (pointer, length) = Arguments::new(values, 1).slice(guest)?;
    let path = read(guest, pointer, length)?;
    let Some(file) = guest.state().policy.file(&path) else { return Ok(Out::Integer(0)) };
    Ok(Out::Integer(fs::metadata(file).is_ok_and(|kind| is(&kind)).into()))
}

fn os_write(guest: &mut dyn Guest, values: &[Val], error: bool) -> R<Out> {
    let (pointer, length) = Arguments::new(values, 1).slice(guest)?;
    charge(guest, u64::try_from(length).unwrap_or(u64::MAX))?;
    let text = read(guest, pointer, length)?;
    let state = guest.state();
    if error { &mut state.err } else { &mut state.out }.extend_from_slice(&text);
    Ok(Out::Nothing)
}

fn os_arg(guest: &mut dyn Guest, values: &[Val]) -> R<Out> {
    let (index, control) = (integer(guest, values, 0)?, integer(guest, values, 1)?);
    let argument = usize::try_from(index).ok().and_then(|index| guest.state().arguments.get(index).cloned());
    if let Some(argument) = argument {
        append_bytes(guest, control, &argument)?;
    }
    Ok(Out::Nothing)
}

fn os_getenv(guest: &mut dyn Guest, values: &[Val]) -> R<Out> {
    let mut arguments = Arguments::new(values, 2);
    let (pointer, length) = arguments.slice(guest)?;
    let control = arguments.integer(guest)?;
    let name = read(guest, pointer, length)?;
    if !guest.state().policy.variable(&name) {
        return Ok(Out::Integer(0));
    }
    match std::env::var_os(OsStr::from_bytes(&name)) {
        Some(value) => {
            append_bytes(guest, control, &value.into_vec())?;
            Ok(Out::Integer(1))
        }
        None => Ok(Out::Integer(0)),
    }
}

/// Appends to the growable byte array whose control is at `control` through
/// the module's exported `goose.reserve(control, count) -> address`.
fn append_bytes(guest: &mut dyn Guest, control: i64, bytes: &[u8]) -> R<()> {
    // goose.reserve runs module code, which could call the host back into
    // here (or into a file read) while this host buffer is held: refused,
    // so that host buffers do not pile up and the host cannot recurse.
    if guest.state().reserving {
        return Err(fail(guest, "goose.reserve called back into the host"));
    }
    guest.state().reserving = true;
    let reserved = guest.reserve(control, bytes.len() as i64);
    guest.state().reserving = false;
    let destination = match reserved {
        None => return Err(fail(guest, "module does not export goose.reserve")),
        Some(destination) => destination?,
    };
    write(guest, address(destination), bytes)
}

// std's byte-set searches: a set's members are its first 256 bytes.

fn find_any(guest: &mut dyn Guest, values: &[Val]) -> R<Out> {
    let mut arguments = Arguments::new(values, 2);
    let (pointer, length) = arguments.slice(guest)?;
    let members = address(arguments.integer(guest)?);
    let text = read(guest, pointer, length)?;
    let members = read(guest, members, 256)?;
    let found = text.iter().position(|&byte| members[usize::from(byte)] != 0);
    Ok(Out::Integer(found.map_or(-1, |index| index as i64)))
}

fn find_pair(guest: &mut dyn Guest, values: &[Val]) -> R<Out> {
    let mut arguments = Arguments::new(values, 4);
    let (pointer, length) = arguments.slice(guest)?;
    let first = address(arguments.integer(guest)?);
    let distance = arguments.integer(guest)?;
    let second = address(arguments.integer(guest)?);
    let text = read(guest, pointer, length)?;
    let first = read(guest, first, 256)?;
    let second = read(guest, second, 256)?;
    let Ok(distance) = usize::try_from(distance) else {
        return Err(fail(guest, "negative gs_find_pair distance is not supported by this host"));
    };
    let found = (0..text.len().saturating_sub(distance))
        .find(|&index| first[usize::from(text[index])] != 0 && second[usize::from(text[index + distance])] != 0);
    Ok(Out::Integer(found.map_or(-1, |index| index as i64)))
}

// The WASI subset of the C oracle's standalone profile.

fn iovecs(guest: &mut dyn Guest, pointer: i64, count: i64) -> R<Vec<(u64, u64)>> {
    let data = read(guest, pointer.into(), i128::from(count) * 16)?;
    let word = |bytes: &[u8]| u64::from_le_bytes(bytes.try_into().unwrap_or([0; 8]));
    Ok(data.chunks_exact(16).map(|pair| (word(&pair[..8]), word(&pair[8..]))).collect())
}

fn fd_write(guest: &mut dyn Guest, values: &[Val]) -> R<Out> {
    let (fd, pointer, count, written) =
        (integer(guest, values, 0)?, integer(guest, values, 1)?, integer(guest, values, 2)?, integer(guest, values, 3)?);
    if fd != 1 && fd != 2 {
        return Ok(Out::Integer(8));
    }
    let mut total: i128 = 0;
    for (start, size) in iovecs(guest, pointer, count)? {
        charge(guest, size)?;
        let text = read(guest, start.into(), size.into())?;
        let state = guest.state();
        if fd == 1 { &mut state.out } else { &mut state.err }.extend_from_slice(&text);
        total += i128::from(size);
    }
    write64(guest, written, total)?;
    Ok(Out::Integer(0))
}

fn fd_read(guest: &mut dyn Guest, values: &[Val]) -> R<Out> {
    let (fd, pointer, count, read_at) =
        (integer(guest, values, 0)?, integer(guest, values, 1)?, integer(guest, values, 2)?, integer(guest, values, 3)?);
    if fd != 0 {
        return Ok(Out::Integer(8));
    }
    let mut total = 0;
    for (start, size) in iovecs(guest, pointer, count)? {
        let chunk = {
            let state = guest.state();
            let rest = &state.input[state.input_position..];
            let taken = rest.len().min(usize::try_from(size).unwrap_or(usize::MAX));
            let chunk = rest[..taken].to_vec();
            state.input_position += taken;
            chunk
        };
        write(guest, start.into(), &chunk)?;
        total += chunk.len();
        if (chunk.len() as u64) < size {
            break;
        }
    }
    write64(guest, read_at, total as i128)?;
    Ok(Out::Integer(0))
}

// Clocks.

fn monotonic_ns(guest: &mut dyn Guest) -> i64 {
    // Monotonic from an arbitrary origin, as CLOCK_MONOTONIC is.
    guest.state().clock_origin.elapsed().as_nanos() as i64 + 1_000_000_000
}

fn wall_ns() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_nanos() as i64)
}

// The driver.

const USAGE: &str = "usage: <program> [--timing-json path] [--allow-files DIR]... [--allow-env NAME|*]... \
[--time-limit SECONDS] [--memory-limit-pages N] [--output-limit BYTES] [--stack-limit-mib N] [-- program arguments]";

/// Runs a compiled Goose program as `wtime-goose` runs the module it came
/// from (`module` is the module's path, the program's first argument when
/// it is given arguments): the same output, exit code and timing JSON. Guest
/// calls nest on a native stack of `stack_bytes`.
///
/// `instantiate` instantiates the module: memory, globals, segments and
/// start function. The instance takes the state; if instantiation fails,
/// the state holds what the module wrote and the host's failure.
pub fn main<G: Guest + 'static>(module: &str, mut stack_bytes: usize, instantiate: Instantiate<G>) -> ! {
    let mut own: Vec<Vec<u8>> = std::env::args_os().skip(1).map(std::ffi::OsString::into_vec).collect();
    let arguments = own.iter().position(|argument| argument == b"--").map(|at| {
        let rest = own.split_off(at + 1);
        own.pop();
        rest
    });
    let mut timing_json = None;
    let mut policy = Policy::default();
    let mut time_limit = None;
    let usage = |text: &str| -> ! {
        eprintln!("{USAGE}\n{text}");
        std::process::exit(2)
    };
    let mut own = own.into_iter().map(OsString::from_vec);
    while let Some(argument) = own.next() {
        let mut value = |name: &str| own.next().unwrap_or_else(|| usage(&format!("{name} needs a value")));
        match argument.to_string_lossy().as_ref() {
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0)
            }
            "--timing-json" => timing_json = Some(value("--timing-json").to_string_lossy().into_owned()),
            "--allow-files" => {
                let directory = value("--allow-files");
                if let Err(error) = policy.allow_directory(Path::new(&directory)) {
                    usage(&format!("--allow-files {}: {error}", directory.to_string_lossy()))
                }
            }
            "--allow-env" => match value("--allow-env") {
                all if all == "*" => policy.variables = None,
                name => {
                    if let Some(names) = &mut policy.variables {
                        names.push(name)
                    }
                }
            },
            "--time-limit" => {
                let seconds = value("--time-limit").to_string_lossy().parse::<f64>().ok().filter(|s| *s > 0.0);
                match seconds.and_then(|s| std::time::Duration::try_from_secs_f64(s).ok()) {
                    Some(limit) => time_limit = Some(limit),
                    None => usage("--time-limit takes a positive number of seconds"),
                }
            }
            "--output-limit" => match value("--output-limit").to_string_lossy().parse::<u64>() {
                Ok(bytes) => policy.output_limit = bytes,
                Err(_) => usage("--output-limit takes a number of bytes"),
            },
            "--stack-limit-mib" => match value("--stack-limit-mib").to_string_lossy().parse::<usize>() {
                Ok(mib @ 16..=65536) => stack_bytes = mib << 20,
                _ => usage("--stack-limit-mib takes a number of MiB from 16 to 65536"),
            },
            "--memory-limit-pages" => match value("--memory-limit-pages").to_string_lossy().parse::<u64>() {
                Ok(pages) => crate::set_memory_limit_pages(pages),
                Err(_) => usage("--memory-limit-pages takes a number of 64 KiB pages"),
            },
            other => usage(&format!("unrecognized argument {other}")),
        }
    }
    let mut input = Vec::new();
    let _ = std::io::stdin().lock().read_to_end(&mut input);
    let program = match arguments {
        None => vec![b"program".to_vec()],
        Some(arguments) => {
            let mut program = vec![module.as_bytes().to_vec()];
            program.extend(arguments);
            program
        }
    };
    // The program runs on a thread of its own, which sends its outcome back;
    // past the time limit the process ends without it.
    let (send, receive) = std::sync::mpsc::channel();
    let worker = std::thread::Builder::new().stack_size(stack_bytes).spawn(move || {
        // Guest calls trap before they come within the margin of the stack's
        // end: large frames cannot overflow it while calls stay below the
        // depth limit.
        let top = 0u8;
        let here = &top as *const u8 as usize;
        let margin = (stack_bytes / 8).max(4 << 20);
        STACK_FLOOR.with(|floor| floor.set(here.saturating_sub(stack_bytes).saturating_add(margin)));
        let _ = send.send(run(input, program, instantiate, policy));
    });
    if worker.is_err() {
        eprintln!("wtime-aot Goose host: the program's thread failed");
        std::process::exit(1)
    }
    let received = match time_limit {
        Some(limit) => receive.recv_timeout(limit).map_err(|error| error == std::sync::mpsc::RecvTimeoutError::Timeout),
        None => receive.recv().map_err(|_| false),
    };
    let outcome = match received {
        Ok(outcome) => outcome,
        Err(true) => {
            eprintln!("ResourceLimit: time limit");
            std::process::exit(1)
        }
        Err(false) => {
            eprintln!("wtime-aot Goose host: the program's thread failed");
            std::process::exit(1)
        }
    };
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(&outcome.stdout).and_then(|()| stdout.flush());
    if !outcome.stderr.is_empty() {
        let _ = std::io::stderr().lock().write_all((outcome.stderr.clone() + "\n").as_bytes());
    }
    if let Some(path) = timing_json {
        let json = format!(
            "{{\n  \"engine\": \"wtime-aot\",\n  \"compile_instantiate_ns\": {},\n  \"compile_ns\": 0,\n  \"start_function_ns\": {},\n  \"engine_total_ns\": {}\n}}\n",
            outcome.instantiate_ns, outcome.start_ns, outcome.instantiate_ns + outcome.start_ns
        );
        if let Err(error) = fs::write(&path, json) {
            eprintln!("{path}: {error}");
        }
    }
    std::process::exit(outcome.exit_code as i32)
}

struct Outcome {
    stdout: Vec<u8>,
    stderr: String,
    exit_code: i64,
    instantiate_ns: u128,
    start_ns: u128,
}

fn run<G: Guest>(input: Vec<u8>, program: Vec<Vec<u8>>, instantiate: Instantiate<G>, policy: Policy) -> Outcome {
    let started = Instant::now();
    let mut instantiated = started;
    let mut guest = None;
    let mut state = State::new(input, program).with_policy(policy);
    let result = (|| -> R<()> {
        let g = guest.insert(instantiate(&mut state)?);
        instantiated = Instant::now();
        g.start()
    })();
    let ended = Instant::now();
    if let Some(g) = guest.as_mut() {
        state = std::mem::replace(g.state(), State::new(Vec::new(), Vec::new()));
    }
    let (exit_code, error) = match (result, state.exit, state.failure.take()) {
        (_, Some(code), _) => (code, String::new()),
        (Ok(()), None, _) => (0, String::new()),
        (Err(_), None, Some(failure)) => (1, failure),
        (Err(error), None, None) => (1, error.message().to_string()),
    };
    let outcome = Outcome {
        stdout: std::mem::take(&mut state.out),
        stderr: String::from_utf8_lossy(&state.err).into_owned() + &error,
        exit_code,
        instantiate_ns: (instantiated - started).as_nanos(),
        start_ns: (ended - instantiated).as_nanos(),
    };
    // The process exits next: tearing down the memory would only spend time.
    std::mem::forget(guest);
    outcome
}

#[cfg(test)]
mod tests {
    use super::Policy;
    use std::path::Path;

    #[test]
    fn files_stay_inside_granted_directories() {
        let root = std::env::temp_dir().join(format!("wtime-aot-policy-{}", std::process::id()));
        let (jail, outside) = (root.join("jail"), root.join("outside"));
        std::fs::create_dir_all(jail.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(jail.join("in.txt"), b"in").unwrap();
        std::fs::write(outside.join("secret.txt"), b"secret").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), jail.join("to_secret")).unwrap();
        std::os::unix::fs::symlink(outside.join("new.txt"), jail.join("dangling")).unwrap();
        std::os::unix::fs::symlink(&outside, jail.join("to_outside")).unwrap();
        let mut policy = Policy::default();
        let at = |p: &Path| p.as_os_str().as_encoded_bytes().to_vec();
        assert_eq!(policy.file(&at(&jail.join("in.txt"))), None, "nothing granted");
        policy.allow_directory(&jail).unwrap();
        let jail = jail.canonicalize().unwrap();
        assert_eq!(policy.file(&at(&jail.join("in.txt"))), Some(jail.join("in.txt")));
        assert_eq!(policy.file(&at(&jail.join("new.txt"))), Some(jail.join("new.txt")), "a new file inside");
        assert_eq!(policy.file(&at(&jail.join("sub/../in.txt"))), None, "no `..`, even inside");
        assert_eq!(policy.file(&at(&jail.join("sub/./../sub"))), None);
        assert_eq!(policy.file(&at(&jail.join("./sub/."))), Some(jail.join("sub")));
        for escape in ["../outside/secret.txt", "sub/../../outside/secret.txt", "to_secret", "dangling", "to_outside/secret.txt", "to_outside/new.txt", ".."] {
            assert_eq!(policy.file(&at(&jail.join(escape))), None, "{escape}");
        }
        assert_eq!(policy.file(b"/etc/passwd"), None);
        // Nothing outside is looked at on the way: a walk out and back in
        // is refused whether or not the outside exists.
        let back_in = format!("{}/../jail/in.txt", outside.display());
        assert_eq!(policy.file(back_in.as_bytes()), None);
        let through_proc = format!("/proc/1/../..{}", jail.join("in.txt").display());
        assert_eq!(policy.file(through_proc.as_bytes()), None);
        assert_eq!(policy.file(&at(&jail.join("in.txt/"))), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn variables_are_granted_by_name() {
        let mut policy = Policy::default();
        assert!(!policy.variable(b"HOME"));
        policy.variables = Some(vec!["HOME".into()]);
        assert!(policy.variable(b"HOME") && !policy.variable(b"PATH"));
        policy.variables = None;
        assert!(policy.variable(b"PATH"));
    }
}
