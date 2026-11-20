//! The imports of a direct Goose module (goose-to-wasm) for code compiled
//! ahead of time, and the driver of such a program.
//!
//! A port of `tools/wtime-goose/src/host.rs` (itself a port of goose-to-wasm's
//! Wasmtime host): the same imports, output, exit codes and timing JSON, with
//! the module reached through `Guest` instead of the interpreter's store.

use std::ffi::OsStr;
use std::fs;
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
        }
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

/// A compiled module as the host and the driver see it.
pub trait Guest {
    /// Instantiates the module: memory, globals, data, start function.
    fn instantiate(state: State) -> R<Self>
    where
        Self: Sized;
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
            _ => Err(Trap::ResultMismatch),
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
    guest.state().out.extend_from_slice(text);
    Ok(Out::Nothing)
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
    match fs::read(OsStr::from_bytes(&normalized_path(&path))) {
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
    let written = options.open(OsStr::from_bytes(&path)).and_then(|mut file| file.write_all(&contents));
    Ok(Out::Integer(written.is_ok().into()))
}

fn os_stat(guest: &mut dyn Guest, values: &[Val], is: fn(&fs::Metadata) -> bool) -> R<Out> {
    let (pointer, length) = Arguments::new(values, 1).slice(guest)?;
    let path = read(guest, pointer, length)?;
    Ok(Out::Integer(fs::metadata(OsStr::from_bytes(&path)).is_ok_and(|kind| is(&kind)).into()))
}

fn os_write(guest: &mut dyn Guest, values: &[Val], error: bool) -> R<Out> {
    let (pointer, length) = Arguments::new(values, 1).slice(guest)?;
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
    let destination = match guest.reserve(control, bytes.len() as i64) {
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

const USAGE: &str = "usage: <program> [--timing-json path] [-- program arguments]";

/// Runs a compiled Goose program as `wtime-goose` runs the module it came
/// from (`module` is the module's path, the program's first argument when
/// it is given arguments): the same output, exit code and timing JSON. Guest
/// calls nest on a native stack of `stack_bytes`.
pub fn main<G: Guest>(module: &str, stack_bytes: usize) -> ! {
    let mut own: Vec<Vec<u8>> = std::env::args_os().skip(1).map(std::ffi::OsString::into_vec).collect();
    let arguments = own.iter().position(|argument| argument == b"--").map(|at| {
        let rest = own.split_off(at + 1);
        own.pop();
        rest
    });
    let mut timing_json = None;
    let mut own = own.into_iter().map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    while let Some(argument) = own.next() {
        match argument.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0)
            }
            "--timing-json" => timing_json = own.next(),
            other => {
                eprintln!("{USAGE}\nunrecognized argument {other}");
                std::process::exit(2)
            }
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
    let worker = std::thread::Builder::new().stack_size(stack_bytes).spawn(move || run::<G>(input, program));
    let outcome = match worker.map(|handle| handle.join()) {
        Ok(Ok(outcome)) => outcome,
        _ => {
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

fn run<G: Guest>(input: Vec<u8>, program: Vec<Vec<u8>>) -> Outcome {
    let started = Instant::now();
    let mut instantiated = started;
    let mut guest = None;
    let result = (|| -> R<()> {
        let g = guest.insert(G::instantiate(State::new(input, program))?);
        instantiated = Instant::now();
        g.start()
    })();
    let ended = Instant::now();
    let mut state = match guest.as_mut() {
        Some(g) => std::mem::replace(g.state(), State::new(Vec::new(), Vec::new())),
        None => State::new(Vec::new(), Vec::new()),
    };
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
