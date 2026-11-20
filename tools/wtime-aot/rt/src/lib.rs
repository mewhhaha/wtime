//! Runtime support for Wasm modules compiled ahead of time to Rust by
//! `wtime-aot`: linear memory, the numeric operators whose Wasm semantics
//! differ from Rust's, traps, and the Goose host and driver.
//!
//! Everything is safe Rust. Generated code calls these helpers; LLVM inlines
//! them, so a load is a bounds check and a read.
#![forbid(unsafe_code)]

pub mod float;
pub mod goose;
pub mod pytext;

use std::fmt;

/// A trap, or a host's refusal. One byte, so that results stay in
/// registers; a host keeps the text of its own failures in its state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trap {
    Unreachable,
    OutOfBounds,
    /// An active element segment does not fit its table.
    TableOutOfBounds,
    DivideByZero,
    IntegerOverflow,
    InvalidConversion,
    CallStackExhausted,
    MemoryTooLarge,
    AllocationFailed,
    NoStart,
    /// A host import's result does not fit its declared type.
    ResultMismatch,
    /// The host refused; its text is in the host's state.
    Host,
    /// The program ended itself; the code is in the host's state.
    Exit,
    /// The program read past the end of its input.
    StopIteration,
}

impl Trap {
    pub fn message(self) -> &'static str {
        match self {
            // The texts wtime-goose reports for the same failures.
            Trap::Unreachable => "Trap: unreachable",
            Trap::OutOfBounds => "Trap: out of bounds",
            Trap::TableOutOfBounds => "Trap: table out of bounds",
            Trap::DivideByZero => "Trap: integer divide by zero",
            Trap::IntegerOverflow => "Trap: integer overflow",
            Trap::InvalidConversion => "Trap: invalid integer conversion",
            Trap::CallStackExhausted => "ResourceLimit: call depth exhausted",
            Trap::MemoryTooLarge => "ResourceLimit: memory too large",
            Trap::AllocationFailed => "ResourceLimit: memory allocation failed",
            Trap::NoStart => "Host: module does not export _start",
            Trap::ResultMismatch => "Host: host import result does not fit its declared type",
            Trap::Host => "Host: host failure",
            Trap::Exit => "program exit",
            Trap::StopIteration => "Host: generator raised StopIteration",
        }
    }
}

impl fmt::Display for Trap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

pub type R<T> = Result<T, Trap>;

#[cold]
#[inline(never)]
pub fn trap<T>(trap: Trap) -> R<T> {
    Err(trap)
}

/// A value passed to or returned from a host function.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Val {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
}

/// The linear memory of a module: a vector as long as the memory, whose
/// capacity is reserved up to the memory's maximum (at most `RESERVE`) by
/// one zeroed allocation that the OS zeroes lazily. An access checks one
/// bound, the vector's length.
pub struct Memory {
    bytes: Vec<u8>,
    /// The most pages it may grow to.
    max_pages: u64,
}

pub const PAGE: u64 = 65536;
/// Bytes reserved up front; a memory that grows past them is reallocated.
const RESERVE: u64 = 4 << 30;

impl Memory {
    /// A memory of `min` pages that may grow to `max` (the index type's
    /// limit when absent).
    pub fn new(min: u64, max: Option<u64>, memory64: bool) -> R<Memory> {
        let limit = if memory64 { 1 << 48 } else { 1 << 16 };
        let max_pages = max.unwrap_or(limit).min(limit);
        let len = min.checked_mul(PAGE).and_then(|n| usize::try_from(n).ok());
        let Some(len) = len else { return trap(Trap::MemoryTooLarge) };
        let reserve = usize::try_from(max_pages.saturating_mul(PAGE).min(RESERVE)).unwrap_or(len).max(len);
        let mut bytes = zeroed(reserve)?;
        bytes.truncate(len);
        Ok(Memory { bytes, max_pages })
    }

    /// The effective address of an access of `n` bytes, or the trap.
    #[inline(always)]
    fn at(&self, address: u64, offset: u64, n: usize) -> R<usize> {
        match address.checked_add(offset).and_then(|a| usize::try_from(a).ok()) {
            Some(at) if at <= self.bytes.len() && n <= self.bytes.len() - at => Ok(at),
            _ => trap(Trap::OutOfBounds),
        }
    }

    #[inline(always)]
    pub fn load<const N: usize>(&self, address: u64, offset: u64) -> R<[u8; N]> {
        let at = self.at(address, offset, N)?;
        match self.bytes.get(at..at + N) {
            Some(bytes) => Ok(bytes.try_into().unwrap_or([0; N])),
            None => trap(Trap::OutOfBounds),
        }
    }

    #[inline(always)]
    pub fn store<const N: usize>(&mut self, address: u64, offset: u64, value: [u8; N]) -> R<()> {
        let at = self.at(address, offset, N)?;
        match self.bytes.get_mut(at..at + N) {
            Some(bytes) => {
                bytes.copy_from_slice(&value);
                Ok(())
            }
            None => trap(Trap::OutOfBounds),
        }
    }

    /// The size in pages.
    pub fn size(&self) -> u64 {
        self.bytes.len() as u64 / PAGE
    }

    /// Grows by `delta` pages; the old size, or -1 when it cannot.
    pub fn grow(&mut self, delta: u64) -> i64 {
        let old = self.size();
        let Some(new) = old.checked_add(delta).filter(|&n| n <= self.max_pages) else { return -1 };
        let Some(len) = new.checked_mul(PAGE).and_then(|n| usize::try_from(n).ok()) else { return -1 };
        // The new bytes are written as zeros: a vector cannot take on bytes
        // it has not written. Its reserved capacity (or a reallocation that
        // moves the pages) holds them.
        if self.bytes.try_reserve(len - self.bytes.len()).is_err() {
            return -1;
        }
        self.bytes.resize(len, 0);
        old as i64
    }

    /// The current bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn fill(&mut self, destination: u64, value: u8, n: u64) -> R<()> {
        let n = usize::try_from(n).or_else(|_| trap(Trap::OutOfBounds))?;
        let at = self.at(destination, 0, n)?;
        self.bytes[at..at + n].fill(value);
        Ok(())
    }

    pub fn copy(&mut self, destination: u64, source: u64, n: u64) -> R<()> {
        let n = usize::try_from(n).or_else(|_| trap(Trap::OutOfBounds))?;
        let to = self.at(destination, 0, n)?;
        let from = self.at(source, 0, n)?;
        self.bytes.copy_within(from..from + n, to);
        Ok(())
    }

    /// Copies `n` bytes of `data` from `from` to `destination` (memory.init
    /// and active data segments).
    pub fn init(&mut self, destination: u64, data: &[u8], from: u64, n: u64) -> R<()> {
        let n = usize::try_from(n).or_else(|_| trap(Trap::OutOfBounds))?;
        let from = usize::try_from(from).ok().filter(|f| f.checked_add(n).is_some_and(|end| end <= data.len()));
        let Some(from) = from else { return trap(Trap::OutOfBounds) };
        let to = self.at(destination, 0, n)?;
        self.bytes[to..to + n].copy_from_slice(&data[from..from + n]);
        Ok(())
    }

    /// The `n` bytes at `address`, for the host.
    pub fn read(&self, address: u64, n: u64) -> R<Vec<u8>> {
        let n = usize::try_from(n).or_else(|_| trap(Trap::OutOfBounds))?;
        let at = self.at(address, 0, n)?;
        Ok(self.bytes[at..at + n].to_vec())
    }

    /// Writes `data` at `address`, for the host.
    pub fn write(&mut self, address: u64, data: &[u8]) -> R<()> {
        let at = self.at(address, 0, data.len())?;
        self.bytes[at..at + data.len()].copy_from_slice(data);
        Ok(())
    }
}

/// What the host reads and writes in a module's memory.
pub trait LinearMemory {
    /// The `n` bytes at `address`.
    fn read(&self, address: u64, n: u64) -> R<Vec<u8>>;
    /// Writes `data` at `address`.
    fn write(&mut self, address: u64, data: &[u8]) -> R<()>;
}

impl LinearMemory for Memory {
    fn read(&self, address: u64, n: u64) -> R<Vec<u8>> {
        Memory::read(self, address, n)
    }
    fn write(&mut self, address: u64, data: &[u8]) -> R<()> {
        Memory::write(self, address, data)
    }
}

/// Bytes a 32-bit memory spans: every address plus offset, both 32-bit,
/// and an access of up to 16 bytes.
const SPAN32: usize = (1 << 33) + 16;

/// A 32-bit linear memory: a lazily zeroed allocation spanning every
/// address a 32-bit access can form, and the memory's current length. The
/// span is a constant, so LLVM proves the allocation's own bound and an
/// access checks only the length; growing moves the length and touches
/// nothing.
pub struct Memory32 {
    bytes: Box<[u8; SPAN32]>,
    len: usize,
    max_pages: u64,
}

impl Memory32 {
    /// A memory of `min` pages that may grow to `max` (at most 65536).
    pub fn new(min: u64, max: Option<u64>) -> R<Memory32> {
        let max_pages = max.unwrap_or(1 << 16).min(1 << 16);
        let Some(len) = min.checked_mul(PAGE).and_then(|n| usize::try_from(n).ok()) else {
            return trap(Trap::MemoryTooLarge);
        };
        if min > max_pages {
            return trap(Trap::MemoryTooLarge);
        }
        let bytes: Box<[u8; SPAN32]> = match zeroed(SPAN32)?.into_boxed_slice().try_into() {
            Ok(bytes) => bytes,
            Err(_) => return trap(Trap::AllocationFailed),
        };
        Ok(Memory32 { bytes, len, max_pages })
    }

    /// The first byte of an access of `N` bytes at `address + offset`.
    #[inline(always)]
    fn at<const N: usize>(&self, address: u32, offset: u32) -> R<usize> {
        let at = address as usize + offset as usize;
        if at + N > self.len {
            return trap(Trap::OutOfBounds);
        }
        Ok(at)
    }

    #[inline(always)]
    pub fn load32<const N: usize>(&self, address: u32, offset: u32) -> R<[u8; N]> {
        let at = self.at::<N>(address, offset)?;
        let mut out = [0; N];
        out.copy_from_slice(&self.bytes[at..at + N]);
        Ok(out)
    }
    #[inline(always)]
    pub fn store32<const N: usize>(&mut self, address: u32, offset: u32, value: [u8; N]) -> R<()> {
        let at = self.at::<N>(address, offset)?;
        self.bytes[at..at + N].copy_from_slice(&value);
        Ok(())
    }

    // Typed accesses: integers, not byte arrays, cross the Result, which
    // LLVM keeps in registers.
    #[inline(always)]
    pub fn l8(&self, address: u32, offset: u32) -> R<u8> {
        let at = self.at::<1>(address, offset)?;
        Ok(self.bytes[at])
    }
    #[inline(always)]
    pub fn l16(&self, address: u32, offset: u32) -> R<u16> {
        let at = self.at::<2>(address, offset)?;
        let b = &self.bytes[at..at + 2];
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    #[inline(always)]
    pub fn l32(&self, address: u32, offset: u32) -> R<u32> {
        let at = self.at::<4>(address, offset)?;
        let b = &self.bytes[at..at + 4];
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    #[inline(always)]
    pub fn l64(&self, address: u32, offset: u32) -> R<u64> {
        let at = self.at::<8>(address, offset)?;
        let b = &self.bytes[at..at + 8];
        Ok(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }
    #[inline(always)]
    pub fn s8(&mut self, address: u32, offset: u32, value: u8) -> R<()> {
        let at = self.at::<1>(address, offset)?;
        self.bytes[at] = value;
        Ok(())
    }
    #[inline(always)]
    pub fn s16(&mut self, address: u32, offset: u32, value: u16) -> R<()> {
        self.store32::<2>(address, offset, value.to_le_bytes())
    }
    #[inline(always)]
    pub fn s32(&mut self, address: u32, offset: u32, value: u32) -> R<()> {
        self.store32::<4>(address, offset, value.to_le_bytes())
    }
    #[inline(always)]
    pub fn s64(&mut self, address: u32, offset: u32, value: u64) -> R<()> {
        self.store32::<8>(address, offset, value.to_le_bytes())
    }

    /// The size in pages.
    pub fn size(&self) -> u64 {
        self.len as u64 / PAGE
    }

    /// Grows by `delta` pages; the old size, or -1 when it cannot.
    pub fn grow(&mut self, delta: u64) -> i64 {
        let old = self.size();
        let Some(new) = old.checked_add(delta).filter(|&n| n <= self.max_pages) else { return -1 };
        self.len = (new * PAGE) as usize;
        old as i64
    }

    /// The range of `n` bytes at `address`, or the trap.
    fn range(&self, address: u64, n: u64) -> R<std::ops::Range<usize>> {
        match address.checked_add(n) {
            Some(end) if end <= self.len as u64 => Ok(address as usize..end as usize),
            _ => trap(Trap::OutOfBounds),
        }
    }

    pub fn fill(&mut self, destination: u64, value: u8, n: u64) -> R<()> {
        let range = self.range(destination, n)?;
        self.bytes[range].fill(value);
        Ok(())
    }

    pub fn copy(&mut self, destination: u64, source: u64, n: u64) -> R<()> {
        let to = self.range(destination, n)?;
        let from = self.range(source, n)?;
        self.bytes.copy_within(from, to.start);
        Ok(())
    }

    pub fn init(&mut self, destination: u64, data: &[u8], from: u64, n: u64) -> R<()> {
        let source = usize::try_from(from).ok().zip(usize::try_from(n).ok());
        let Some(source) = source.and_then(|(f, n)| data.get(f..f.checked_add(n)?)) else {
            return trap(Trap::OutOfBounds);
        };
        let to = self.range(destination, n)?;
        self.bytes[to].copy_from_slice(source);
        Ok(())
    }
}

impl LinearMemory for Memory32 {
    fn read(&self, address: u64, n: u64) -> R<Vec<u8>> {
        let range = self.range(address, n)?;
        Ok(self.bytes[range].to_vec())
    }
    fn write(&mut self, address: u64, data: &[u8]) -> R<()> {
        let range = self.range(address, data.len() as u64)?;
        self.bytes[range].copy_from_slice(data);
        Ok(())
    }
}

// Integer division and remainder, which trap.

macro_rules! division {
    ($div_s:ident, $div_u:ident, $rem_s:ident, $rem_u:ident, $s:ty, $u:ty) => {
        #[inline(always)]
        pub fn $div_s(a: $s, b: $s) -> R<$s> {
            if b == 0 {
                return trap(Trap::DivideByZero);
            }
            if a == <$s>::MIN && b == -1 {
                return trap(Trap::IntegerOverflow);
            }
            Ok(a / b)
        }
        #[inline(always)]
        pub fn $div_u(a: $s, b: $s) -> R<$s> {
            if b == 0 {
                return trap(Trap::DivideByZero);
            }
            Ok(((a as $u) / (b as $u)) as $s)
        }
        #[inline(always)]
        pub fn $rem_s(a: $s, b: $s) -> R<$s> {
            if b == 0 {
                return trap(Trap::DivideByZero);
            }
            Ok(a.wrapping_rem(b))
        }
        #[inline(always)]
        pub fn $rem_u(a: $s, b: $s) -> R<$s> {
            if b == 0 {
                return trap(Trap::DivideByZero);
            }
            Ok(((a as $u) % (b as $u)) as $s)
        }
    };
}
division!(i32_div_s, i32_div_u, i32_rem_s, i32_rem_u, i32, u32);
division!(i64_div_s, i64_div_u, i64_rem_s, i64_rem_u, i64, u64);

// min and max: NaN if either is, and -0 below +0.

macro_rules! min_max {
    ($min:ident, $max:ident, $f:ty) => {
        #[inline(always)]
        pub fn $min(a: $f, b: $f) -> $f {
            if a.is_nan() || b.is_nan() {
                <$f>::NAN
            } else if a == b {
                if a.is_sign_negative() { a } else { b }
            } else if a < b {
                a
            } else {
                b
            }
        }
        #[inline(always)]
        pub fn $max(a: $f, b: $f) -> $f {
            if a.is_nan() || b.is_nan() {
                <$f>::NAN
            } else if a == b {
                if a.is_sign_positive() { a } else { b }
            } else if a > b {
                a
            } else {
                b
            }
        }
    };
}
min_max!(f32_min, f32_max, f32);
min_max!(f64_min, f64_max, f64);

// Truncation to integers, which traps on NaN and out of range: the value
// truncated toward zero must lie strictly between the bounds given (each
// exactly representable in the float type).

macro_rules! truncate {
    ($name:ident, $f:ty, $i:ty, $low:expr, $high:expr) => {
        #[inline(always)]
        pub fn $name(a: $f) -> R<$i> {
            if a.is_nan() {
                return trap(Trap::InvalidConversion);
            }
            let t = a.trunc();
            if t <= $low || t >= $high {
                return trap(Trap::IntegerOverflow);
            }
            Ok(t as $i)
        }
    };
}
truncate!(i32_trunc_f32_s, f32, i32, -2147483904.0f32, 2147483648.0f32);
truncate!(i32_trunc_f32_u, f32, u32, -1.0f32, 4294967296.0f32);
truncate!(i32_trunc_f64_s, f64, i32, -2147483649.0f64, 2147483648.0f64);
truncate!(i32_trunc_f64_u, f64, u32, -1.0f64, 4294967296.0f64);
truncate!(i64_trunc_f32_s, f32, i64, -9223373136366403584.0f32, 9223372036854775808.0f32);
truncate!(i64_trunc_f32_u, f32, u64, -1.0f32, 18446744073709551616.0f32);
truncate!(i64_trunc_f64_s, f64, i64, -9223372036854777856.0f64, 9223372036854775808.0f64);
truncate!(i64_trunc_f64_u, f64, u64, -1.0f64, 18446744073709551616.0f64);

/// `n` zero bytes from a zeroed allocation (pages the OS zeroes when first
/// touched), or the trap when it fails.
fn zeroed(n: usize) -> R<Vec<u8>> {
    let mut probe: Vec<u8> = Vec::new();
    if probe.try_reserve_exact(n).is_err() {
        return trap(Trap::AllocationFailed);
    }
    drop(probe);
    Ok(vec![0u8; n])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_bounds() {
        assert_eq!(i32_trunc_f32_s(-2147483648.0), Ok(i32::MIN));
        assert!(i32_trunc_f32_s(2147483648.0).is_err());
        assert_eq!(i32_trunc_f64_s(-2147483648.9), Ok(i32::MIN));
        assert!(i32_trunc_f64_s(-2147483649.0).is_err());
        assert_eq!(i32_trunc_f64_u(-0.9), Ok(0));
        assert!(i32_trunc_f64_u(-1.0).is_err());
        assert_eq!(i64_trunc_f64_s(-9223372036854775808.0), Ok(i64::MIN));
        assert!(i64_trunc_f64_u(18446744073709551616.0).is_err());
        assert_eq!(i32_trunc_f32_s(f32::NAN), Err(Trap::InvalidConversion));
    }

    #[test]
    fn min_max_signed_zero_and_nan() {
        assert!(f64_min(0.0, -0.0).is_sign_negative());
        assert!(f64_max(-0.0, 0.0).is_sign_positive());
        assert!(f32_min(1.0, f32::NAN).is_nan());
    }

    #[test]
    fn memory32_bounds() {
        let mut m = Memory32::new(1, Some(2)).unwrap();
        assert_eq!(m.l32(65532, 0), Ok(0));
        assert!(m.l32(65533, 0).is_err());
        assert!(m.l32(u32::MAX, u32::MAX).is_err());
        assert!(m.s64(65530, 0, 1).is_err());
        assert_eq!(m.grow(1), 1);
        assert_eq!(m.grow(1), -1);
        assert!(m.s64(131064, 0, 7).is_ok());
        assert_eq!(m.l64(131060, 4), Ok(7));
        assert!(m.copy(0, 131064, 8).is_ok());
        assert_eq!(m.l64(0, 0), Ok(7));
    }

    #[test]
    fn memory_bounds() {
        let mut m = Memory::new(1, Some(2), false).unwrap();
        assert!(m.load::<4>(65532, 0).is_ok());
        assert!(m.load::<4>(65533, 0).is_err());
        assert!(m.load::<4>(u64::MAX, 8).is_err());
        assert_eq!(m.grow(1), 1);
        assert_eq!(m.grow(1), -1);
        assert!(m.store::<8>(131064, 0, [1; 8]).is_ok());
    }
}
