//! The imports of a direct Goose module (goose-to-wasm), on wtime.
//!
//! A port of goose-to-wasm's `host/rust/src/host.rs` (its Wasmtime host) to
//! the wtime embedding API, so that both engines run the same module with the
//! same imports, output and exit codes. Each import is defined with the type
//! the module declares for it: an address is an i32 in a 32-bit memory and an
//! i64 in a 64-bit one, and a slice is one packed i64 or two i64.

use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use wtime_runtime::{Caller, Error, ErrorKind, External, ExternalType, Func, Memory, Module, RunOutcome, Store, Value};
use wtime_types::{CompositeType, FuncType, ValType};
use wtime_validate::DecodedModule;

use crate::float::{f32_text, f64_text};
use crate::pytext::{normalized_path, parse_float, parse_int};

/// What a run keeps outside the module.
pub struct State {
    input: Vec<u8>,
    token_at: usize,
    input_position: usize,
    pub out: Vec<u8>,
    pub err: Vec<u8>,
    arguments: Vec<Vec<u8>>,
    pub memory: Option<Memory>,
    pub reserve: Option<Func>,
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
            memory: None,
            reserve: None,
            exit: None,
            failure: None,
            clock_origin: Instant::now(),
        }
    }

    fn token(&mut self) -> Result<&[u8], Error> {
        let space = |byte: &u8| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
        let rest = &self.input[self.token_at..];
        let Some(start) = rest.iter().position(|byte| !space(byte)) else {
            self.token_at = self.input.len();
            return Err(Error::new(ErrorKind::Host, "generator raised StopIteration"));
        };
        let start = self.token_at + start;
        let end = self.input[start..].iter().position(space).map_or(self.input.len(), |length| start + length);
        self.token_at = end;
        Ok(&self.input[start..end])
    }
}

pub type Shared = Arc<Mutex<State>>;

/// Everything a handler can reach: the store, through the caller, and the
/// host state.
struct Cx<'a, 'b> {
    caller: &'a mut Caller<'b>,
    state: &'a Mutex<State>,
}

impl Cx<'_, '_> {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("host state lock")
    }

    fn fail(&self, text: impl Into<String>) -> Error {
        let text = text.into();
        self.state().failure.get_or_insert_with(|| text.clone());
        Error::new(ErrorKind::Host, text)
    }

    fn memory(&self) -> Result<Memory, Error> {
        self.state().memory.ok_or_else(|| self.fail("host memory is unavailable during module initialization"))
    }

    fn read(&mut self, pointer: i128, length: i128) -> Result<Vec<u8>, Error> {
        if pointer < 0 || length < 0 {
            return Err(self.fail("negative host address"));
        }
        let memory = self.memory()?;
        let length = usize::try_from(length).map_err(|_| self.fail("host write is outside exported memory"))?;
        self.caller
            .store()
            .read_memory(memory, pointer as u64, length)
            .map_err(|_| self.fail("host write is outside exported memory"))
    }

    fn write(&mut self, pointer: i128, bytes: &[u8]) -> Result<(), Error> {
        if pointer < 0 {
            return Err(self.fail("negative host address"));
        }
        let memory = self.memory()?;
        self.caller
            .store()
            .write_memory(memory, pointer as u64, bytes)
            .map_err(|_| self.fail("host write is outside exported memory"))
    }

    fn write64(&mut self, pointer: i64, value: i128) -> Result<(), Error> {
        let value =
            u64::try_from(value).map_err(|_| self.fail("'Q' format requires 0 <= number <= 18446744073709551615"))?;
        self.write(pointer.into(), &value.to_le_bytes())
    }

    fn print(&mut self, text: &[u8]) -> Result<Out, Error> {
        self.state().out.extend_from_slice(text);
        Ok(Out::Nothing)
    }
}

enum Out {
    Nothing,
    Integer(i64),
    Float(f64),
}

type Handler = fn(&mut Cx<'_, '_>, &[Value]) -> Result<Out, Error>;

/// The function type of each function import of `decoded`, by its module and
/// name.
fn import_types(decoded: &DecodedModule) -> Vec<(String, String, wtime_types::TypeIdx, FuncType)> {
    let types: Vec<&CompositeType> = decoded.type_groups.iter().flat_map(|group| &group.types).map(|t| &t.composite).collect();
    decoded
        .imports
        .iter()
        .filter_map(|import| {
            let (wtime_validate::ImportKind::Func(index) | wtime_validate::ImportKind::ExactFunc(index)) = import.kind
            else {
                return None;
            };
            let CompositeType::Func(ty) = types.get(index.as_usize())? else { return None };
            Some((import.module.clone(), import.name.clone(), index, ty.clone()))
        })
        .collect()
}

/// Creates a host function for every import of `module` (decoded as
/// `decoded`), as `(module, name, function)` for the linker.
pub fn define_imports(
    store: &mut Store,
    module: &Module,
    decoded: &DecodedModule,
    state: &Shared,
) -> Result<Vec<(String, String, Func)>, String> {
    if decoded.imports.len() != import_types(decoded).len() {
        return Err("direct Goose modules import only functions".into());
    }
    let mut defined = Vec::new();
    for (module_name, name, index, ty) in import_types(decoded) {
        let Some(handler) = handler_for(&module_name, &name) else {
            return Err(format!("unsupported direct host import: {module_name}.{name}"));
        };
        let declared = ty.results.clone();
        let state = Arc::clone(state);
        let function = store
            .create_module_host_func(module, index, move |caller, values| {
                let mut cx = Cx { caller, state: &state };
                let out = handler(&mut cx, values)?;
                let result = match (out, declared.as_slice()) {
                    (Out::Nothing, []) => vec![],
                    (_, []) => return Err(cx.fail("callback produced results when it shouldn't")),
                    (Out::Integer(value), [ValType::I32]) => vec![Value::I32(value as i32)],
                    (Out::Integer(value), [ValType::I64]) => vec![Value::I64(value)],
                    (Out::Float(value), [ValType::F64]) => vec![Value::F64(value.to_bits())],
                    (Out::Float(value), [ValType::F32]) => vec![Value::F32((value as f32).to_bits())],
                    _ => return Err(cx.fail("host import result does not fit its declared type")),
                };
                Ok(RunOutcome::Returned(result))
            })
            .map_err(|error| error.to_string())?;
        defined.push((module_name, name, function));
    }
    Ok(defined)
}

fn exit(cx: &mut Cx<'_, '_>, values: &[Value]) -> Result<Out, Error> {
    let code = integer(cx, values, 0)?;
    cx.state().exit = Some(code);
    Err(Error::new(ErrorKind::Host, "program exit"))
}

fn handler_for(module: &str, name: &str) -> Option<Handler> {
    let handler: Handler = match (module, name) {
        ("goose", "write") => |cx, values| {
            let (pointer, length) = (integer(cx, values, 0)?, integer(cx, values, 1)?);
            let text = cx.read(address(pointer), address(length))?;
            cx.print(&text)
        },
        ("goose", "print_i64") => |cx, values| {
            let text = integer(cx, values, 0)?.to_string();
            cx.print(text.as_bytes())
        },
        ("goose", "print_u64") => |cx, values| {
            let text = (integer(cx, values, 0)? as u64).to_string();
            cx.print(text.as_bytes())
        },
        ("goose", "print_f64") => |cx, values| {
            let text = f64_text(float(cx, values, 0)?);
            cx.print(text.as_bytes())
        },
        ("goose", "print_f32") => |cx, values| {
            let text = f32_text(float(cx, values, 0)? as f32);
            cx.print(text.as_bytes())
        },
        ("goose", "format_f64") => |cx, values| {
            let text = f64_text(float(cx, values, 1)?);
            format_float(cx, values, text)
        },
        ("goose", "format_f32") => |cx, values| {
            let text = f32_text(float(cx, values, 1)? as f32);
            format_float(cx, values, text)
        },
        ("goose", "print_bool") => |cx, values| {
            let truth = truth(cx, values, 0)?;
            cx.print(if truth { b"true" } else { b"false" })
        },
        ("goose", "newline") => |cx, _| cx.print(b"\n"),
        ("goose", "abort") => |cx, values| {
            let (pointer, length, line) = (integer(cx, values, 0)?, integer(cx, values, 1)?, integer(cx, values, 2)?);
            let message = cx.read(address(pointer), address(length))?;
            Err(cx.fail(format!("goose runtime error: {} (line {line})", String::from_utf8_lossy(&message))))
        },
        ("goose", "exit") | ("wasi_snapshot_preview1", "proc_exit") => exit,
        ("env", "bench_input") => |cx, _| {
            let parsed = parse_int(cx.state().token()?);
            parsed.map(Out::Integer).map_err(|text| cx.fail(text))
        },
        ("env", "bench_input_f64") => |cx, _| {
            let parsed = parse_float(cx.state().token()?);
            parsed.map(Out::Float).map_err(|text| cx.fail(text))
        },
        ("env", "bench_clock") | ("env", "gs_os_clock_ns") => |cx, _| Ok(Out::Integer(monotonic_ns(cx))),
        ("env", "bench_barrier") => |cx, values| match values.first() {
            Some(Value::F32(_) | Value::F64(_)) => Ok(Out::Float(float(cx, values, 0)?)),
            _ => Ok(Out::Integer(integer(cx, values, 0)?)),
        },
        ("env", "bench_sink") | ("env", "emscripten_notify_memory_growth") | ("env", "gs_os_flush_stdout") => {
            |_, _| Ok(Out::Nothing)
        }
        ("env", "gs_os_read_file") => os_read_file,
        ("env", "gs_os_write_file") => |cx, values| os_write_file(cx, values, false),
        ("env", "gs_os_append_file") => |cx, values| os_write_file(cx, values, true),
        ("env", "gs_os_file_exists") => |cx, values| os_stat(cx, values, |kind| kind.is_file()),
        ("env", "gs_os_is_dir") => |cx, values| os_stat(cx, values, |kind| kind.is_dir()),
        ("env", "gs_os_write_stdout") => |cx, values| os_write(cx, values, false),
        ("env", "gs_os_write_stderr") => |cx, values| os_write(cx, values, true),
        ("env", "gs_os_arg_count") => |cx, _| Ok(Out::Integer(cx.state().arguments.len() as i64)),
        ("env", "gs_os_arg") => os_arg,
        ("env", "gs_os_getenv") => os_getenv,
        ("env", "gs_os_resource_dir") => |_, _| Ok(Out::Integer(0)),
        ("env", "gs_os_time_ns") => |_, _| Ok(Out::Integer(wall_ns())),
        ("env", "gs_find_any") => find_any,
        ("env", "gs_find_pair") => find_pair,
        ("env", "system") => |_, _| Ok(Out::Integer(-1)),
        ("wasi_snapshot_preview1", "args_sizes_get") => |cx, values| {
            let (count, size) = (integer(cx, values, 0)?, integer(cx, values, 1)?);
            cx.write64(count, 1)?;
            cx.write64(size, 8)?;
            Ok(Out::Integer(0))
        },
        ("wasi_snapshot_preview1", "args_get") => |cx, values| {
            let (argv, buffer) = (integer(cx, values, 0)?, integer(cx, values, 1)?);
            cx.write64(argv, buffer.into())?;
            cx.write(buffer.into(), b"program\0")?;
            Ok(Out::Integer(0))
        },
        ("wasi_snapshot_preview1", "environ_sizes_get") => |cx, values| {
            let (count, size) = (integer(cx, values, 0)?, integer(cx, values, 1)?);
            cx.write64(count, 0)?;
            cx.write64(size, 0)?;
            Ok(Out::Integer(0))
        },
        ("wasi_snapshot_preview1", "environ_get") => |_, _| Ok(Out::Integer(0)),
        ("wasi_snapshot_preview1", "clock_time_get") => |cx, values| {
            let value = match integer(cx, values, 0)? {
                0 => wall_ns(),
                1 => monotonic_ns(cx),
                _ => return Ok(Out::Integer(28)),
            };
            let out = integer(cx, values, 2)?;
            cx.write64(out, value.into())?;
            Ok(Out::Integer(0))
        },
        ("wasi_snapshot_preview1", "fd_write") => fd_write,
        ("wasi_snapshot_preview1", "fd_read") => fd_read,
        ("wasi_snapshot_preview1", "fd_close") => |cx, values| {
            Ok(Out::Integer(if (0..=2).contains(&integer(cx, values, 0)?) { 0 } else { 8 }))
        },
        ("wasi_snapshot_preview1", "fd_seek") => |cx, values| {
            Ok(Out::Integer(if (0..=2).contains(&integer(cx, values, 0)?) { 70 } else { 8 }))
        },
        _ => return None,
    };
    Some(handler)
}

// Arguments.

fn integer(cx: &Cx<'_, '_>, values: &[Value], index: usize) -> Result<i64, Error> {
    match values.get(index) {
        Some(Value::I32(value)) => Ok(i64::from(*value)),
        Some(Value::I64(value)) => Ok(*value),
        Some(_) => Err(cx.fail("host import declared with a parameter that is not an integer")),
        None => Err(cx.fail("host import declared with too few parameters")),
    }
}

fn float(cx: &Cx<'_, '_>, values: &[Value], index: usize) -> Result<f64, Error> {
    match values.get(index) {
        Some(Value::F32(bits)) => Ok(f64::from(f32::from_bits(*bits))),
        Some(Value::F64(bits)) => Ok(f64::from_bits(*bits)),
        Some(_) => Err(cx.fail("host import declared with a parameter that is not a float")),
        None => Err(cx.fail("host import declared with too few parameters")),
    }
}

fn truth(cx: &Cx<'_, '_>, values: &[Value], index: usize) -> Result<bool, Error> {
    match values.get(index) {
        Some(Value::F32(_) | Value::F64(_)) => Ok(float(cx, values, index)? != 0.0),
        _ => Ok(integer(cx, values, index)? != 0),
    }
}

/// The arguments of an import that takes slices: one packed i64 (pointer in
/// the low 32 bits, count in the high) in a 32-bit module, two i64 in a
/// 64-bit one.
struct Arguments<'v> {
    values: &'v [Value],
    at: usize,
    wide: bool,
}

impl<'v> Arguments<'v> {
    fn new(values: &'v [Value], kinds: usize) -> Arguments<'v> {
        Arguments { values, at: 0, wide: values.len() > kinds }
    }

    fn integer(&mut self, cx: &Cx<'_, '_>) -> Result<i64, Error> {
        self.at += 1;
        integer(cx, self.values, self.at - 1)
    }

    fn slice(&mut self, cx: &Cx<'_, '_>) -> Result<(i128, i128), Error> {
        if self.wide {
            Ok((self.integer(cx)?.into(), self.integer(cx)?.into()))
        } else {
            let packed = self.integer(cx)? as u64;
            Ok(((packed & 0xffff_ffff).into(), (packed >> 32).into()))
        }
    }
}

fn address(value: i64) -> i128 {
    if value < 0 { (value & 0xffff_ffff).into() } else { value.into() }
}

// The goose module.

fn format_float(cx: &mut Cx<'_, '_>, values: &[Value], text: String) -> Result<Out, Error> {
    if text.len() > 32 {
        return Err(cx.fail("formatted float exceeds host buffer"));
    }
    let pointer = address(integer(cx, values, 0)?);
    // The whole 32-byte buffer must be inside memory, as the other hosts check.
    cx.read(pointer, 32)?;
    cx.write(pointer, text.as_bytes())?;
    Ok(Out::Integer(text.len() as i64))
}

// The stdlib's os externs.

fn os_read_file(cx: &mut Cx<'_, '_>, values: &[Value]) -> Result<Out, Error> {
    let mut arguments = Arguments::new(values, 2);
    let (pointer, length) = arguments.slice(cx)?;
    let control = arguments.integer(cx)?;
    let path = cx.read(pointer, length)?;
    if path.contains(&0) {
        return Err(cx.fail("embedded null byte"));
    }
    match fs::read(OsStr::from_bytes(&normalized_path(&path))) {
        Ok(contents) => {
            append_bytes(cx, control, &contents)?;
            Ok(Out::Integer(1))
        }
        Err(_) => Ok(Out::Integer(0)),
    }
}

fn os_write_file(cx: &mut Cx<'_, '_>, values: &[Value], append: bool) -> Result<Out, Error> {
    let mut arguments = Arguments::new(values, 2);
    let (path_pointer, path_length) = arguments.slice(cx)?;
    let (pointer, length) = arguments.slice(cx)?;
    let path = cx.read(path_pointer, path_length)?;
    let contents = cx.read(pointer, length)?;
    if path.contains(&0) {
        return Err(cx.fail("embedded null byte"));
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

fn os_stat(cx: &mut Cx<'_, '_>, values: &[Value], is: fn(&fs::Metadata) -> bool) -> Result<Out, Error> {
    let (pointer, length) = Arguments::new(values, 1).slice(cx)?;
    let path = cx.read(pointer, length)?;
    Ok(Out::Integer(fs::metadata(OsStr::from_bytes(&path)).is_ok_and(|kind| is(&kind)).into()))
}

fn os_write(cx: &mut Cx<'_, '_>, values: &[Value], error: bool) -> Result<Out, Error> {
    let (pointer, length) = Arguments::new(values, 1).slice(cx)?;
    let text = cx.read(pointer, length)?;
    let mut state = cx.state();
    if error { &mut state.err } else { &mut state.out }.extend_from_slice(&text);
    Ok(Out::Nothing)
}

fn os_arg(cx: &mut Cx<'_, '_>, values: &[Value]) -> Result<Out, Error> {
    let (index, control) = (integer(cx, values, 0)?, integer(cx, values, 1)?);
    let argument = usize::try_from(index).ok().and_then(|index| cx.state().arguments.get(index).cloned());
    if let Some(argument) = argument {
        append_bytes(cx, control, &argument)?;
    }
    Ok(Out::Nothing)
}

fn os_getenv(cx: &mut Cx<'_, '_>, values: &[Value]) -> Result<Out, Error> {
    let mut arguments = Arguments::new(values, 2);
    let (pointer, length) = arguments.slice(cx)?;
    let control = arguments.integer(cx)?;
    let name = cx.read(pointer, length)?;
    match std::env::var_os(OsStr::from_bytes(&name)) {
        Some(value) => {
            append_bytes(cx, control, &value.into_vec())?;
            Ok(Out::Integer(1))
        }
        None => Ok(Out::Integer(0)),
    }
}

/// Appends to the growable byte array whose control is at `control` through
/// the module's exported `goose.reserve(control, count) -> address`.
fn append_bytes(cx: &mut Cx<'_, '_>, control: i64, bytes: &[u8]) -> Result<(), Error> {
    let Some(reserve) = cx.state().reserve else {
        return Err(cx.fail("module does not export goose.reserve"));
    };
    let ty = match cx.caller.store().external_type(External::Func(reserve))? {
        ExternalType::Func(ty) => ty,
        _ => return Err(cx.fail("goose.reserve is not (address, address) -> address")),
    };
    let fitted = |kind: &ValType, value: i64| match kind {
        ValType::I32 => Some(Value::I32(value as i32)),
        ValType::I64 => Some(Value::I64(value)),
        _ => None,
    };
    let arguments = match (ty.params.as_slice(), ty.results.len()) {
        ([first, second], 1) => [fitted(first, control), fitted(second, bytes.len() as i64)],
        _ => [None, None],
    };
    let [Some(first), Some(second)] = arguments else {
        return Err(cx.fail("goose.reserve is not (address, address) -> address"));
    };
    let destination = match cx.caller.invoke(reserve, &[first, second])? {
        RunOutcome::Returned(values) => match values.as_slice() {
            [Value::I32(destination)] => address((*destination).into()),
            [Value::I64(destination)] => address(*destination),
            _ => return Err(cx.fail("goose.reserve is not (address, address) -> address")),
        },
        other => return Err(cx.fail(format!("goose.reserve did not return: {other:?}"))),
    };
    cx.write(destination, bytes)
}

// std's byte-set searches: a set's members are its first 256 bytes.

fn find_any(cx: &mut Cx<'_, '_>, values: &[Value]) -> Result<Out, Error> {
    let mut arguments = Arguments::new(values, 2);
    let (pointer, length) = arguments.slice(cx)?;
    let members = address(arguments.integer(cx)?);
    let text = cx.read(pointer, length)?;
    let members = cx.read(members, 256)?;
    let found = text.iter().position(|&byte| members[usize::from(byte)] != 0);
    Ok(Out::Integer(found.map_or(-1, |index| index as i64)))
}

fn find_pair(cx: &mut Cx<'_, '_>, values: &[Value]) -> Result<Out, Error> {
    let mut arguments = Arguments::new(values, 4);
    let (pointer, length) = arguments.slice(cx)?;
    let first = address(arguments.integer(cx)?);
    let distance = arguments.integer(cx)?;
    let second = address(arguments.integer(cx)?);
    let text = cx.read(pointer, length)?;
    let first = cx.read(first, 256)?;
    let second = cx.read(second, 256)?;
    let Ok(distance) = usize::try_from(distance) else {
        return Err(cx.fail("negative gs_find_pair distance is not supported by this host"));
    };
    let found = (0..text.len().saturating_sub(distance))
        .find(|&index| first[usize::from(text[index])] != 0 && second[usize::from(text[index + distance])] != 0);
    Ok(Out::Integer(found.map_or(-1, |index| index as i64)))
}

// The WASI subset of the C oracle's standalone profile.

fn iovecs(cx: &mut Cx<'_, '_>, pointer: i64, count: i64) -> Result<Vec<(u64, u64)>, Error> {
    let data = cx.read(pointer.into(), i128::from(count) * 16)?;
    let word = |bytes: &[u8]| u64::from_le_bytes(bytes.try_into().expect("eight bytes"));
    Ok(data.chunks_exact(16).map(|pair| (word(&pair[..8]), word(&pair[8..]))).collect())
}

fn fd_write(cx: &mut Cx<'_, '_>, values: &[Value]) -> Result<Out, Error> {
    let (fd, pointer, count, written) =
        (integer(cx, values, 0)?, integer(cx, values, 1)?, integer(cx, values, 2)?, integer(cx, values, 3)?);
    if fd != 1 && fd != 2 {
        return Ok(Out::Integer(8));
    }
    let mut total: i128 = 0;
    for (start, size) in iovecs(cx, pointer, count)? {
        let text = cx.read(start.into(), size.into())?;
        let mut state = cx.state();
        if fd == 1 { &mut state.out } else { &mut state.err }.extend_from_slice(&text);
        total += i128::from(size);
    }
    cx.write64(written, total)?;
    Ok(Out::Integer(0))
}

fn fd_read(cx: &mut Cx<'_, '_>, values: &[Value]) -> Result<Out, Error> {
    let (fd, pointer, count, read) =
        (integer(cx, values, 0)?, integer(cx, values, 1)?, integer(cx, values, 2)?, integer(cx, values, 3)?);
    if fd != 0 {
        return Ok(Out::Integer(8));
    }
    let mut total = 0;
    for (start, size) in iovecs(cx, pointer, count)? {
        let chunk = {
            let mut state = cx.state();
            let rest = &state.input[state.input_position..];
            let taken = rest.len().min(usize::try_from(size).unwrap_or(usize::MAX));
            let chunk = rest[..taken].to_vec();
            state.input_position += taken;
            chunk
        };
        cx.write(start.into(), &chunk)?;
        total += chunk.len();
        if (chunk.len() as u64) < size {
            break;
        }
    }
    cx.write64(read, total as i128)?;
    Ok(Out::Integer(0))
}

// Clocks.

fn monotonic_ns(cx: &Cx<'_, '_>) -> i64 {
    // Monotonic from an arbitrary origin, as CLOCK_MONOTONIC is.
    cx.state().clock_origin.elapsed().as_nanos() as i64 + 1_000_000_000
}

fn wall_ns() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_nanos() as i64)
}
