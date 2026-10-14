use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    Decode,
    Validation,
    DisabledFeature,
    ResourceLimit,
    Link,
    Trap,
    Host,
}

/// Machine-readable guest trap reasons. Guest exception objects are a separate runtime outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrapCode {
    Unreachable,
    MemoryOutOfBounds,
    TableOutOfBounds,
    IntegerDivisionByZero,
    IntegerOverflow,
    InvalidConversionToInteger,
    IndirectCallTypeMismatch,
    NullReference,
    UnalignedAtomic,
    Other,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorContext {
    Function(u32),
    Type(u32),
    Memory(u32),
    Table(u32),
    Global(u32),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    pub offset: Option<usize>,
    pub trap_code: Option<TrapCode>,
    pub context: Option<ErrorContext>,
}
impl Error {
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
    pub fn trap(code: TrapCode, message: impl Into<String>) -> Self {
        let mut error = Self::new(ErrorKind::Trap, message);
        error.trap_code = Some(code);
        error
    }
    pub fn in_context(mut self, context: ErrorContext) -> Self {
        self.context = Some(context);
        self
    }
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
