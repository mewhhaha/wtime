use std::fmt;

/// High-level category of a wtime error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// Binary decoding failure (malformed or truncated WebAssembly bytes).
    Decode,
    /// Static validation failure (type, index, or structural violation).
    Validation,
    /// Use of a WebAssembly proposal disabled by the active [`FeatureSet`](crate::FeatureSet).
    DisabledFeature,
    /// Host-configured [`ResourceLimits`](crate::ResourceLimits) budget, fuel, or cancellation bound exceeded.
    ResourceLimit,
    /// Module instantiation or import/export linking failure.
    Link,
    /// Runtime WebAssembly trap during execution.
    Trap,
    /// Host API misuse (such as a foreign handle, type mismatch, or callback error).
    Host,
}

/// Machine-readable guest trap reasons. Guest exception objects are a separate runtime outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrapCode {
    /// Execution reached an `unreachable` instruction.
    Unreachable,
    /// Linear memory access or initialization exceeded memory bounds.
    MemoryOutOfBounds,
    /// Table access or initialization exceeded table bounds.
    TableOutOfBounds,
    /// Integer division or remainder by zero.
    IntegerDivisionByZero,
    /// Signed integer division overflow (such as `i32::MIN / -1`).
    IntegerOverflow,
    /// Floating-point value could not be converted to the target integer type.
    InvalidConversionToInteger,
    /// Indirect or typed reference call signature did not match the callee.
    IndirectCallTypeMismatch,
    /// Dereference or call through a null reference.
    NullReference,
    /// Atomic memory access address was not naturally aligned.
    UnalignedAtomic,
    /// Trap not covered by a more specific variant.
    Other,
}

/// Optional module-level entity associated with an [`Error`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorContext {
    /// Function index in the module's function index space.
    Function(u32),
    /// Type index in the module's type index space.
    Type(u32),
    /// Memory index in the module's memory index space.
    Memory(u32),
    /// Table index in the module's table index space.
    Table(u32),
    /// Global index in the module's global index space.
    Global(u32),
}

/// Structured error returned across decoding, validation, linking, and execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// High-level failure category.
    pub kind: ErrorKind,
    /// Human-readable diagnostic message.
    pub message: String,
    /// Optional absolute byte offset in the source WebAssembly binary.
    pub offset: Option<usize>,
    /// Machine-readable trap reason when `kind == ErrorKind::Trap`.
    pub trap_code: Option<TrapCode>,
    /// Optional module entity index providing additional location context.
    pub context: Option<ErrorContext>,
}
impl Error {
    /// Creates a new error with the given category and message.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            offset: None,
            trap_code: if kind == ErrorKind::Trap {
                Some(TrapCode::Other)
            } else {
                None
            },
            context: None,
        }
    }
    /// Creates a trap error (`ErrorKind::Trap`) with a specific [`TrapCode`].
    pub fn trap(code: TrapCode, message: impl Into<String>) -> Self {
        let mut error = Self::new(ErrorKind::Trap, message);
        error.trap_code = Some(code);
        error
    }
    /// Attaches a module entity context to this error.
    pub fn in_context(mut self, context: ErrorContext) -> Self {
        self.context = Some(context);
        self
    }
    /// Attaches an absolute source byte offset to this error.
    pub fn at(mut self, offset: usize) -> Self {
        self.offset = Some(offset);
        self
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)?;
        if let Some(offset) = self.offset {
            write!(f, " at byte {offset}")?;
        }
        if let Some(context) = self.context {
            write!(f, " ({context:?})")?;
        }
        Ok(())
    }
}
impl std::error::Error for Error {}
/// Convenience `Result` alias using wtime's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn t005_case_01() {
        let e = Error::trap(TrapCode::Unreachable, "unreachable");
        assert!(e.to_string().contains("unreachable"));
        assert_eq!(e.trap_code, Some(TrapCode::Unreachable));
    }
    #[test]
    fn t005_case_02() {
        let e = Error::new(ErrorKind::Decode, "bad byte")
            .at(17)
            .in_context(ErrorContext::Function(2));
        assert_eq!(e.offset, Some(17));
        assert_eq!(e.kind, ErrorKind::Decode);
        assert!(e.to_string().contains("Function(2)"));
    }
    #[test]
    fn t005_case_03() {
        assert_eq!(Error::new(ErrorKind::Host, "host failure").trap_code, None);
        assert_ne!(ErrorKind::Host, ErrorKind::Trap);
    }
}
