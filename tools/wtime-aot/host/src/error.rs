//! Error, Result, Trap, GcHeapOutOfMemory, and PoolConcurrencyLimitError types.

use std::any::Any;
use std::error::Error as StdError;
use std::fmt;

/// Convenience `Result<T, Error>` alias matching Wasmtime's host API.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Helper macro matching `wasmtime::bail!`.
#[macro_export]
macro_rules! bail {
    ($msg:literal $(,)?) => {
        return ::std::result::Result::Err($crate::Error::msg($msg))
    };
    ($fmt:expr, $($arg:tt)*) => {
        return ::std::result::Result::Err($crate::Error::msg(::std::format!($fmt, $($arg)*)))
    };
}

/// Concrete WebAssembly execution traps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Trap {
    StackOverflow,
    MemoryOutOfBounds,
    HeapMisaligned,
    TableOutOfBounds,
    IndirectCallToNull,
    BadSignature,
    IntegerOverflow,
    IntegerDivisionByZero,
    BadConversionToInteger,
    UnreachableCodeReached,
    Interrupt,
    AlwaysTrapAdapter,
    OutOfFuel,
    AtomicWaitNonSharedMemory,
    NullReference,
    ArrayOutOfBounds,
    AllocationTooLarge,
    CastFailure,
    CannotEnterComponent,
    NoAsyncResult,
    UnhandledTag,
    ContinuationAlreadyConsumed,
    DisabledOpcode,
}

impl fmt::Display for Trap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StackOverflow => write!(f, "call stack exhausted"),
            Self::MemoryOutOfBounds => write!(f, "out of bounds memory access"),
            Self::HeapMisaligned => write!(f, "unaligned atomic"),
            Self::TableOutOfBounds => write!(f, "out of bounds table access"),
            Self::IndirectCallToNull => write!(f, "indirect call to null"),
            Self::BadSignature => write!(f, "indirect call type mismatch"),
            Self::IntegerOverflow => write!(f, "integer overflow"),
            Self::IntegerDivisionByZero => write!(f, "integer divide by zero"),
            Self::BadConversionToInteger => write!(f, "invalid conversion to integer"),
            Self::UnreachableCodeReached => write!(f, "unreachable"),
            Self::Interrupt => write!(f, "execution interrupted"),
            Self::AlwaysTrapAdapter => write!(f, "always trap adapter"),
            Self::OutOfFuel => write!(f, "out of fuel"),
            Self::AtomicWaitNonSharedMemory => write!(f, "atomic wait on non-shared memory"),
            Self::NullReference => write!(f, "null reference"),
            Self::ArrayOutOfBounds => write!(f, "out of bounds array access"),
            Self::AllocationTooLarge => write!(f, "allocation too large"),
            Self::CastFailure => write!(f, "cast failure"),
            Self::CannotEnterComponent => write!(f, "cannot enter component"),
            Self::NoAsyncResult => write!(f, "no async result"),
            Self::UnhandledTag => write!(f, "unhandled exception"),
            Self::ContinuationAlreadyConsumed => write!(f, "continuation already consumed"),
            Self::DisabledOpcode => write!(f, "disabled opcode"),
        }
    }
}

impl StdError for Trap {}

impl Trap {
    /// Maps a `wtime-aot` x64 `R11d` trap code into a structured `Trap`.
    pub fn from_x64_code(code: u32) -> Self {
        use wtime_aot::x64::*;
        if code == crate::abi::TRAP_INTERRUPT_CODE {
            return Trap::Interrupt;
        }
        match code as u8 {
            TRAP_UNREACHABLE => Trap::UnreachableCodeReached,
            TRAP_OOB => Trap::MemoryOutOfBounds,
            TRAP_TABLE_OOB => Trap::TableOutOfBounds,
            TRAP_INDIRECT_CALL => Trap::BadSignature,
            TRAP_DIV_ZERO => Trap::IntegerDivisionByZero,
            TRAP_INT_OVERFLOW => Trap::IntegerOverflow,
            TRAP_INVALID_CONV => Trap::BadConversionToInteger,
            TRAP_CALL_STACK => Trap::StackOverflow,
            TRAP_MEM_TOO_LARGE | TRAP_MEM_LIMIT | TRAP_ALLOC_FAILED => Trap::AllocationTooLarge,
            TRAP_UNALIGNED_ATOMIC => Trap::HeapMisaligned,
            _ => Trap::UnreachableCodeReached,
        }
    }
}

/// Recoverable GC heap OOM error returned when an allocation cannot fit within the Store's GC heap cap
/// even after a full collection.
#[derive(Debug)]
pub struct GcHeapOutOfMemory<T = ()> {
    host_data: T,
    bytes_needed: u64,
}

impl<T> GcHeapOutOfMemory<T> {
    pub fn new(host_data: T, bytes_needed: u64) -> Self {
        Self {
            host_data,
            bytes_needed,
        }
    }

    pub fn bytes_needed(&self) -> u64 {
        self.bytes_needed
    }

    pub fn into_inner(self) -> T {
        self.host_data
    }

    pub fn take_host_data(self) -> T {
        self.host_data
    }
}

impl<T: fmt::Debug> fmt::Display for GcHeapOutOfMemory<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GC heap out of memory (requested {} bytes)",
            self.bytes_needed
        )
    }
}

impl<T: fmt::Debug> StdError for GcHeapOutOfMemory<T> {}

/// Error returned when a pooled Engine reaches its configured concurrent Store limit.
#[derive(Clone, Debug)]
pub struct PoolConcurrencyLimitError {
    limit: usize,
    name: String,
}

impl PoolConcurrencyLimitError {
    pub fn new(limit: usize, name: impl Into<String>) -> Self {
        Self {
            limit,
            name: name.into(),
        }
    }

    pub fn limit(&self) -> usize {
        self.limit
    }
}

impl fmt::Display for PoolConcurrencyLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "maximum concurrent {} limit of {} reached",
            self.name, self.limit
        )
    }
}

impl StdError for PoolConcurrencyLimitError {}

trait AnyStdError: StdError + Any + Send + Sync + 'static {
    fn as_any(&self) -> &dyn Any;
}

impl<E: StdError + Any + Send + Sync + 'static> AnyStdError for E {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Dynamic error wrapper supporting `downcast_ref::<T>()` and `is::<T>()` (compatible with `wasmtime::Error` / `anyhow::Error`).
pub struct Error {
    inner: Box<dyn AnyStdError>,
    context: Vec<String>,
}

#[derive(Debug)]
struct MsgError(String);

impl fmt::Display for MsgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl StdError for MsgError {}

impl Error {
    pub fn new<E>(err: E) -> Self
    where
        E: StdError + Send + Sync + 'static,
    {
        Self {
            inner: Box::new(err),
            context: Vec::new(),
        }
    }

    pub fn msg<M: fmt::Display>(msg: M) -> Self {
        Self {
            inner: Box::new(MsgError(msg.to_string())),
            context: Vec::new(),
        }
    }

    pub fn context<C: fmt::Display>(mut self, ctx: C) -> Self {
        self.context.push(ctx.to_string());
        self
    }

    pub fn is<T: Any + Send + Sync + 'static>(&self) -> bool {
        self.downcast_ref::<T>().is_some()
    }

    pub fn downcast_ref<T: Any + Send + Sync + 'static>(&self) -> Option<&T> {
        self.inner.as_any().downcast_ref::<T>()
    }

    pub fn root_cause(&self) -> &(dyn StdError + 'static) {
        self.inner.as_ref()
    }

    pub fn chain(&self) -> ErrorChain<'_> {
        ErrorChain {
            next: Some(self.inner.as_ref()),
        }
    }
}

/// Iterator over causal errors inside [`Error`].
pub struct ErrorChain<'a> {
    next: Option<&'a (dyn StdError + 'static)>,
}

impl<'a> Iterator for ErrorChain<'a> {
    type Item = &'a (dyn StdError + 'static);

    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.next?;
        self.next = cur.source();
        Some(cur)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in self.context.iter().rev() {
            writeln!(f, "{c}:")?;
        }
        write!(f, "{}", self.inner)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(top) = self.context.last() {
            write!(f, "{top}: {}", self.inner)
        } else {
            write!(f, "{}", self.inner)
        }
    }
}

impl From<Trap> for Error {
    fn from(t: Trap) -> Self {
        Self::new(t)
    }
}

impl<T: fmt::Debug + Send + Sync + 'static> From<GcHeapOutOfMemory<T>> for Error {
    fn from(e: GcHeapOutOfMemory<T>) -> Self {
        Self::new(e)
    }
}

impl From<PoolConcurrencyLimitError> for Error {
    fn from(e: PoolConcurrencyLimitError) -> Self {
        Self::new(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::new(e)
    }
}

impl From<std::collections::TryReserveError> for Error {
    fn from(e: std::collections::TryReserveError) -> Self {
        Self::new(e)
    }
}

impl From<std::num::TryFromIntError> for Error {
    fn from(e: std::num::TryFromIntError) -> Self {
        Self::new(e)
    }
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Self::msg(s)
    }
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Self::msg(s)
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(self.inner.as_ref())
    }
}
