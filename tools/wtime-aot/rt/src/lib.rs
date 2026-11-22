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
    /// The minimum size overflows 64 bits.
    MemoryAllocationOverflow,
    /// The minimum size exceeds `MEMORY_LIMIT_PAGES`.
    MemoryLimit,
    AllocationFailed,
    NoStart,
    /// `_start` is an imported function that takes parameters.
    StartImportArguments,
    /// The module exports `_start`, but not as a function.
    StartNotFunction,
    /// `_start` takes parameters.
    StartArguments,
    /// A host import's result does not fit its declared type.
    ResultMismatch,
    /// A host import declared without results got one.
    ExtraResults,
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
            Trap::MemoryAllocationOverflow => "ResourceLimit: memory allocation overflow",
            Trap::MemoryLimit => "ResourceLimit: memory limit",
            Trap::AllocationFailed => "ResourceLimit: memory allocation failed",
            Trap::NoStart => "module does not export _start",
            Trap::StartNotFunction => "'_start' is not a function",
            Trap::StartArguments => "Host: argument type mismatch",
            Trap::StartImportArguments => "Host: function value count mismatch",
            Trap::ResultMismatch => "host import result does not fit its declared type",
            Trap::ExtraResults => "callback produced results when it shouldn't",
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

/// `expr?` for generated code: the value of an `Ok`, or a return of the
/// `Err`. Written as a match, it gives rustc one branch to build and check,
/// where `?` adds `Try` and `FromResidual` calls for its MIR inliner; on a
/// crate the size of the Goose compiler that is gigabytes of compile-time
/// memory.
#[macro_export]
macro_rules! t {
    ($e:expr) => {
        match $e {
            Ok(value) => value,
            Err(error) => return Err(error),
        }
    };
}

/// `Err(trap)`, inlined where it is returned. Marking it `#[cold]` and
/// out of line made every bounds check's failure path a call LLVM could not
/// see into: it returned an `R<T>` of unknown variant into the fast path,
/// and LLVM had to assume memory changed on the way back, so each access
/// reloaded the memory's length and pointer and repeated its checks (2.4
/// times the instructions of the insertion sort in examples/sorting.goose).
#[inline(always)]
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

/// The linear memory of a module (64-bit memories use it). A base, one
/// lazily zeroed allocation as large as the OS grants up to the memory's
/// maximum (at most `RESERVE`), holds the low addresses; beyond it, chunks
/// of `CHUNK` bytes, each lazily zeroed and allocated when `memory.grow`
/// reaches it. Growing never moves or touches existing bytes, so a memory
/// can grow to many GiB while the program writes little of it, and no
/// allocation exceeds what the OS grants. An access that ends within the
/// base and the length (`fast`) checks one bound, as in `Memory32`; others
/// take a cold path through the chunks.
pub struct Memory {
    /// Zero beyond `len`.
    base: Vec<u8>,
    outer: Outer,
}

/// A 64-bit memory's state beside its base: the chunks, the length, and
/// what accesses need to find their part. Generated functions keep the base
/// and `Outer` in separate `noalias` arguments. `#[repr(C)]` keeps `fast`
/// and `len` at offsets 0 and 8 so accesses use 1-byte displacements.
#[repr(C)]
pub struct Outer {
    /// `min(len, base_len)`: accesses ending at or below it are direct.
    fast: usize,
    len: usize,
    base_len: usize,
    /// The most pages it may grow to.
    max_pages: u64,
    /// Bytes `base_len..`, `CHUNK` to a chunk; zero beyond `len`.
    chunks: Vec<Vec<u8>>,
}

pub const PAGE: u64 = 65536;
/// The base is at most this large.
const RESERVE: u64 = 64 << 30;
/// A memory never exceeds this many pages (64 GiB), as under wtime-goose,
/// or the operator's lower limit (`set_memory_limit_pages`).
pub const MEMORY_LIMIT_PAGES: u64 = 1 << 20;
static PAGE_LIMIT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(MEMORY_LIMIT_PAGES);

/// Lowers the most pages any memory may have (raising past
/// `MEMORY_LIMIT_PAGES` has no effect). Set before instantiation.
pub fn set_memory_limit_pages(pages: u64) {
    PAGE_LIMIT.store(pages.min(MEMORY_LIMIT_PAGES), std::sync::atomic::Ordering::Relaxed);
}

fn page_limit() -> u64 {
    PAGE_LIMIT.load(std::sync::atomic::Ordering::Relaxed)
}
/// Blocks of bulk operations at least this long that would only write
/// zeros over zeros are skipped, so they leave untouched pages untouched.
const BLOCK: usize = 1 << 16;

/// The machine's memory (Linux's MemTotal), if known.
fn ram_bytes() -> Option<usize> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = info.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kib: usize = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

/// All bytes are zero.
fn all_zero(bytes: &[u8]) -> bool {
    bytes.iter().fold(0, |any, &byte| any | byte) == 0
}
const CHUNK_BITS: u32 = 32;
const CHUNK: usize = 1 << CHUNK_BITS;

impl Memory {
    /// A memory of `min` pages that may grow to `max` (the index type's
    /// limit when absent).
    pub fn new(min: u64, max: Option<u64>, memory64: bool) -> R<Memory> {
        // wtime-goose's limits, in its order and words.
        if min.checked_mul(PAGE).is_none() {
            return trap(Trap::MemoryAllocationOverflow);
        }
        if min > page_limit() {
            return trap(Trap::MemoryLimit);
        }
        let limit = if memory64 { 1 << 48 } else { 1 << 16 };
        let max_pages = max.unwrap_or(limit).min(limit).min(page_limit());
        if min > max_pages {
            return trap(Trap::MemoryTooLarge);
        }
        // The largest base the OS grants, up to the maximum, RESERVE and the
        // machine's memory (larger requests are refused anyway).
        let mut size = usize::try_from(max_pages.saturating_mul(PAGE).min(RESERVE)).unwrap_or(usize::MAX);
        if let Some(ram) = ram_bytes() {
            size = size.min(ram & !(PAGE as usize - 1)).max(min as usize * PAGE as usize);
        }
        let base = loop {
            match zeroed(size) {
                Ok(base) => break base,
                // A sixteenth less each time: the largest grant is usually
                // a little below the RAM size.
                Err(_) if size > PAGE as usize => size = (size - size / 16 - 1) & !(PAGE as usize - 1),
                Err(error) => return Err(error),
            }
        };
        Memory::with_base(min, max_pages, base)
    }

    /// A memory whose base holds its whole maximum, so that no access ever
    /// needs a chunk: generated code then uses `m64`'s flat accesses, which
    /// trap where the others take the slow path. Fails (AllocationFailed)
    /// where the maximum cannot be allocated.
    pub fn new_flat(min: u64, max: Option<u64>, memory64: bool) -> R<Memory> {
        if min.checked_mul(PAGE).is_none() {
            return trap(Trap::MemoryAllocationOverflow);
        }
        if min > page_limit() {
            return trap(Trap::MemoryLimit);
        }
        let limit = if memory64 { 1 << 48 } else { 1 << 16 };
        let max_pages = max.unwrap_or(limit).min(limit).min(page_limit());
        if min > max_pages {
            return trap(Trap::MemoryTooLarge);
        }
        let Some(size) = max_pages.checked_mul(PAGE).and_then(|n| usize::try_from(n).ok()) else {
            return trap(Trap::AllocationFailed);
        };
        Memory::with_base(min, max_pages, zeroed(size)?)
    }

    fn with_base(min: u64, max_pages: u64, base: Vec<u8>) -> R<Memory> {
        let base_len = base.len();
        let mut memory = Memory { base, outer: Outer { chunks: Vec::new(), len: 0, fast: 0, base_len, max_pages } };
        if memory.grow(min) < 0 {
            return trap(Trap::AllocationFailed);
        }
        Ok(memory)
    }

    /// The effective address of an access of `n` bytes, or the trap.
    #[inline(always)]
    fn at(&self, address: u64, offset: u64, n: usize) -> R<usize> {
        match address.checked_add(offset).and_then(|a| usize::try_from(a).ok()) {
            Some(at) if at <= self.outer.len && n <= self.outer.len - at => Ok(at),
            _ => trap(Trap::OutOfBounds),
        }
    }



    /// The `N` bytes of an access that ends within the base and the
    /// length, or None.
    #[inline(always)]
    fn direct<const N: usize>(&self, address: u64, offset: u64) -> Option<&[u8; N]> {
        // At most the base's length, which lets the slices below go
        // unchecked.
        let limit = self.outer.fast.min(self.base.len()).min(isize::MAX as usize);
        // `offset` is a constant where generated code calls this, so the
        // first test folds away. `address` is below `limit`, a slice's
        // length, so the sum cannot overflow.
        if offset < (1 << 62) && address < limit as u64 {
            let end = address + (offset + N as u64);
            if end <= limit as u64 {
                return self.base[..limit][end as usize - N..].first_chunk::<N>();
            }
        }
        None
    }

    #[inline(always)]
    fn direct_mut<const N: usize>(&mut self, address: u64, offset: u64) -> Option<&mut [u8; N]> {
        let limit = self.outer.fast.min(self.base.len()).min(isize::MAX as usize);
        if offset < (1 << 62) && address < limit as u64 {
            let end = address + (offset + N as u64);
            if end <= limit as u64 {
                return self.base[..limit][end as usize - N..].first_chunk_mut::<N>();
            }
        }
        None
    }

    /// The `n` (at most 8) bytes of an access outside the direct range, as
    /// a little-endian integer: an integer, not an array, crosses the
    /// Result in registers.
    #[cold]
    #[inline(never)]
    fn word_slow(&self, address: u64, offset: u64, n: usize) -> R<u64> {
        let mut out = [0; 8];
        let at = self.at(address, offset, n)?;
        self.read_into(at, &mut out[..n]);
        Ok(u64::from_le_bytes(out))
    }

    #[inline(always)]
    fn put<const N: usize>(&mut self, address: u64, offset: u64, value: [u8; N]) -> R<()> {
        match self.direct_mut::<N>(address, offset) {
            Some(bytes) => {
                *bytes = value;
                Ok(())
            }
            None => self.put_slow(address, offset, &value),
        }
    }

    #[cold]
    #[inline(never)]
    fn put_slow(&mut self, address: u64, offset: u64, value: &[u8]) -> R<()> {
        let at = self.at(address, offset, value.len())?;
        self.write_from(at, value);
        Ok(())
    }

    #[cold]
    #[inline(never)]
    fn put_word_slow(&mut self, address: u64, offset: u64, value: u64, n: usize) -> R<()> {
        self.put_slow(address, offset, &value.to_le_bytes()[..n])
    }

    /// Copies the bytes at `at` (in bounds) into `out`, across parts.
    fn read_into(&self, at: usize, out: &mut [u8]) {
        read_parts(&self.base, &self.outer, at, out)
    }

    /// Writes `data` at `at` (in bounds), across parts.
    fn write_from(&mut self, at: usize, data: &[u8]) {
        write_parts(&mut self.base, &mut self.outer, at, data)
    }

    #[inline(always)]
    pub fn load<const N: usize>(&self, address: u64, offset: u64) -> R<[u8; N]> {
        match self.direct::<N>(address, offset) {
            Some(bytes) => Ok(*bytes),
            None => {
                // Word by word, so that integers, not the array, cross the
                // Results and the array stays out of memory on the direct
                // path. A load has no effect, so the access is in bounds
                // exactly when each of its words is.
                let mut out = [0; N];
                let mut k = 0;
                while k < N {
                    let n = if N - k < 8 { N - k } else { 8 };
                    let Some(offset) = offset.checked_add(k as u64) else { return trap(Trap::OutOfBounds) };
                    match self.word_slow(address, offset, n) {
                        Ok(word) => out[k..k + n].copy_from_slice(&word.to_le_bytes()[..n]),
                        Err(error) => return Err(error),
                    }
                    k += n;
                }
                Ok(out)
            }
        }
    }

    #[inline(always)]
    pub fn store<const N: usize>(&mut self, address: u64, offset: u64, value: [u8; N]) -> R<()> {
        self.put(address, offset, value)
    }

    // Typed accesses, as for `Memory32`: integers, not byte arrays, cross
    // the Result.
    #[inline(always)]
    pub fn l8(&self, address: u64, offset: u64) -> R<u8> {
        match self.direct::<1>(address, offset) {
            Some(bytes) => Ok(u8::from_le_bytes(*bytes)),
            None => match self.word_slow(address, offset, 1) {
                Ok(word) => Ok(word as u8),
                Err(trap) => Err(trap),
            },
        }
    }
    #[inline(always)]
    pub fn l16(&self, address: u64, offset: u64) -> R<u16> {
        match self.direct::<2>(address, offset) {
            Some(bytes) => Ok(u16::from_le_bytes(*bytes)),
            None => match self.word_slow(address, offset, 2) {
                Ok(word) => Ok(word as u16),
                Err(trap) => Err(trap),
            },
        }
    }
    #[inline(always)]
    pub fn l32(&self, address: u64, offset: u64) -> R<u32> {
        match self.direct::<4>(address, offset) {
            Some(bytes) => Ok(u32::from_le_bytes(*bytes)),
            None => match self.word_slow(address, offset, 4) {
                Ok(word) => Ok(word as u32),
                Err(trap) => Err(trap),
            },
        }
    }
    #[inline(always)]
    pub fn l64(&self, address: u64, offset: u64) -> R<u64> {
        match self.direct::<8>(address, offset) {
            Some(bytes) => Ok(u64::from_le_bytes(*bytes)),
            None => match self.word_slow(address, offset, 8) {
                Ok(word) => Ok(word as u64),
                Err(trap) => Err(trap),
            },
        }
    }
    #[inline(always)]
    pub fn s8(&mut self, address: u64, offset: u64, value: u8) -> R<()> {
        match self.direct_mut::<1>(address, offset) {
            Some(bytes) => {
                *bytes = value.to_le_bytes();
                Ok(())
            }
            None => self.put_word_slow(address, offset, value as u64, 1),
        }
    }
    #[inline(always)]
    pub fn s16(&mut self, address: u64, offset: u64, value: u16) -> R<()> {
        match self.direct_mut::<2>(address, offset) {
            Some(bytes) => {
                *bytes = value.to_le_bytes();
                Ok(())
            }
            None => self.put_word_slow(address, offset, value as u64, 2),
        }
    }
    #[inline(always)]
    pub fn s32(&mut self, address: u64, offset: u64, value: u32) -> R<()> {
        match self.direct_mut::<4>(address, offset) {
            Some(bytes) => {
                *bytes = value.to_le_bytes();
                Ok(())
            }
            None => self.put_word_slow(address, offset, value as u64, 4),
        }
    }
    #[inline(always)]
    pub fn s64(&mut self, address: u64, offset: u64, value: u64) -> R<()> {
        match self.direct_mut::<8>(address, offset) {
            Some(bytes) => {
                *bytes = value.to_le_bytes();
                Ok(())
            }
            None => self.put_word_slow(address, offset, value as u64, 8),
        }
    }

    /// A 16-byte load as an integer: its halves cross the Results in
    /// registers.
    #[inline(always)]
    pub fn l128(&self, address: u64, offset: u64) -> R<u128> {
        match self.direct::<16>(address, offset) {
            Some(bytes) => Ok(u128::from_le_bytes(*bytes)),
            None => {
                // A load has no effect, so the access is in bounds exactly
                // when both halves are.
                let low = match self.word_slow(address, offset, 8) {
                    Ok(word) => word,
                    Err(error) => return Err(error),
                };
                let Some(offset) = offset.checked_add(8) else { return trap(Trap::OutOfBounds) };
                match self.word_slow(address, offset, 8) {
                    Ok(high) => Ok(low as u128 | (high as u128) << 64),
                    Err(error) => Err(error),
                }
            }
        }
    }
    #[inline(always)]
    pub fn s128(&mut self, address: u64, offset: u64, value: u128) -> R<()> {
        match self.direct_mut::<16>(address, offset) {
            Some(bytes) => {
                *bytes = value.to_le_bytes();
                Ok(())
            }
            None => self.put_slow(address, offset, &value.to_le_bytes()),
        }
    }

    /// The size in pages.
    #[inline(always)]
    pub fn size(&self) -> u64 {
        self.outer.size()
    }

    /// Grows by `delta` pages; the old size, or -1 when it cannot.
    #[cold]
    pub fn grow(&mut self, delta: u64) -> i64 {
        self.outer.grow(delta)
    }

    /// Creates a 64-bit memory split into its fixed base buffer and `Outer`
    /// metadata so generated code can pass the base as a disjoint `noalias`
    /// slice.
    pub fn new_parts(min: u64, max: Option<u64>, memory64: bool, flat: bool) -> R<(Vec<u8>, Outer)> {
        let m = if flat { Memory::new_flat(min, max, memory64)? } else { Memory::new(min, max, memory64)? };
        Ok((m.base, m.outer))
    }

    /// The `n` bytes at `address`, for the host and bulk operations.
    pub fn read(&self, address: u64, n: u64) -> R<Vec<u8>> {
        let n = usize::try_from(n).or_else(|_| trap(Trap::OutOfBounds))?;
        let at = self.at(address, 0, n)?;
        let mut out = vec![0; n];
        self.read_into(at, &mut out);
        Ok(out)
    }

    /// Writes `data` at `address`, for the host and bulk operations.
    pub fn write(&mut self, address: u64, data: &[u8]) -> R<()> {
        let at = self.at(address, 0, data.len())?;
        self.write_from(at, data);
        Ok(())
    }

    pub fn fill(&mut self, destination: u64, value: u8, n: u64) -> R<()> {
        fill_parts(&mut self.base, &mut self.outer, destination, value, n)
    }

    /// `memory.copy` of a constant `N` bytes (at least 1), as
    /// `Memory32::copy_n`; ranges beyond the base take `copy`.
    #[inline(always)]
    pub fn copy_n<const N: usize>(&mut self, destination: u64, source: u64) -> R<()> {
        let limit = self.outer.fast.min(self.base.len());
        if destination < limit as u64 && source < limit as u64 && destination as usize + N <= limit && source as usize + N <= limit {
            move_bytes::<N>(&mut self.base[..limit], destination as usize, source as usize);
            return Ok(());
        }
        self.copy(destination, source, N as u64)
    }

    /// `memory.fill` of a constant `N` bytes (at least 1).
    #[inline(always)]
    pub fn fill_n<const N: usize>(&mut self, destination: u64, value: u8) -> R<()> {
        let limit = self.outer.fast.min(self.base.len());
        if destination < limit as u64 && destination as usize + N <= limit {
            let at = destination as usize;
            self.base[..limit][at..at + N].fill(value);
            return Ok(());
        }
        self.fill(destination, value, N as u64)
    }

    pub fn copy(&mut self, destination: u64, source: u64, n: u64) -> R<()> {
        copy_parts(&mut self.base, &mut self.outer, destination, source, n)
    }

    /// Copies `n` bytes of `data` from `from` to `destination` (memory.init
    /// and active data segments).
    pub fn init(&mut self, destination: u64, data: &[u8], from: u64, n: u64) -> R<()> {
        let n = usize::try_from(n).or_else(|_| trap(Trap::OutOfBounds))?;
        let from = usize::try_from(from).ok().filter(|f| f.checked_add(n).is_some_and(|end| end <= data.len()));
        let Some(from) = from else { return trap(Trap::OutOfBounds) };
        let to = self.at(destination, 0, n)?;
        self.write_from(to, &data[from..from + n]);
        Ok(())
    }
}

impl Memory {
    /// The memory split for generated code: the bytes accesses reach
    /// directly (the base up to `min(len, base length)`) and the rest.
    #[inline(always)]
    pub fn parts(&mut self) -> (&mut [u8], &mut Outer) {
        let fast = self.outer.fast;
        (&mut self.base[..fast], &mut self.outer)
    }
}

impl Outer {
    /// Empty placeholder for moving `Outer` into a temporary host view.
    #[inline(always)]
    pub fn empty() -> Outer {
        Outer { fast: 0, len: 0, base_len: 0, max_pages: 0, chunks: Vec::new() }
    }

    /// Metadata for a flat 64-bit memory backed by a fixed-capacity buffer of
    /// `cap_bytes` bytes (`alloc64::<CAP64>`). Checks limits before allocation.
    pub fn new_flat_meta(min: u64, max: Option<u64>, cap_bytes: usize) -> R<Outer> {
        if min.checked_mul(PAGE).is_none() {
            return trap(Trap::MemoryAllocationOverflow);
        }
        if min > page_limit() {
            return trap(Trap::MemoryLimit);
        }
        let max_pages = max.unwrap_or(1 << 48).min(1 << 48).min(page_limit());
        if min > max_pages {
            return trap(Trap::MemoryTooLarge);
        }
        let Some(len) = min.checked_mul(PAGE).and_then(|n| usize::try_from(n).ok()) else {
            return trap(Trap::AllocationFailed);
        };
        if max_pages.checked_mul(PAGE).is_none_or(|b| b > cap_bytes as u64) {
            return trap(Trap::AllocationFailed);
        }
        Ok(Outer { fast: len, len, base_len: cap_bytes, max_pages, chunks: Vec::new() })
    }

    /// Bytes reachable directly in the base buffer (`min(len, base_len)`).
    #[inline(always)]
    pub fn fast(&self) -> usize {
        self.fast
    }

    /// Current byte length of the memory.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the memory currently has zero bytes.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// True while every byte of the memory lies inside the base buffer
    /// (`len == fast`, no chunks allocated yet).
    #[inline(always)]
    pub fn is_flat(&self) -> bool {
        self.len == self.fast
    }

    /// Current size in 64 KiB pages.
    #[inline(always)]
    pub fn size(&self) -> u64 {
        self.len as u64 / PAGE
    }

    /// Grows by `delta` pages; the old size, or -1 when it cannot.
    /// Never moves or touches the base buffer.
    #[cold]
    pub fn grow(&mut self, delta: u64) -> i64 {
        let old = self.size();
        let Some(new) = old.checked_add(delta).filter(|&n| n <= self.max_pages) else { return -1 };
        let Some(len) = new.checked_mul(PAGE).and_then(|n| usize::try_from(n).ok()) else { return -1 };
        let max = usize::try_from(self.max_pages.saturating_mul(PAGE)).unwrap_or(usize::MAX);
        while self.base_len + self.chunks.len() * CHUNK < len {
            let start = self.base_len + self.chunks.len() * CHUNK;
            let Ok(chunk) = zeroed(CHUNK.min(max - start)) else { return -1 };
            self.chunks.push(chunk);
        }
        self.len = len;
        self.fast = len.min(self.base_len);
        old as i64
    }

    /// The effective address of an access of `n` bytes, or the trap.
    #[inline(always)]
    fn at(&self, address: u64, offset: u64, n: usize) -> R<usize> {
        match address.checked_add(offset).and_then(|a| usize::try_from(a).ok()) {
            Some(at) if at <= self.len && n <= self.len - at => Ok(at),
            _ => trap(Trap::OutOfBounds),
        }
    }
}

/// Initializes a 64-bit memory's parts from a data segment.
pub fn init_parts(base: &mut [u8], outer: &mut Outer, destination: u64, data: &[u8], from: u64, n: u64) -> R<()> {
    let n = usize::try_from(n).or_else(|_| trap(Trap::OutOfBounds))?;
    let from = usize::try_from(from).ok().filter(|f| f.checked_add(n).is_some_and(|end| end <= data.len()));
    let Some(from) = from else { return trap(Trap::OutOfBounds) };
    let to = outer.at(destination, 0, n)?;
    write_parts(base, outer, to, &data[from..from + n]);
    Ok(())
}

/// Borrows `n` bytes at `address` from a 64-bit memory without copying when
/// the range lies within the base buffer.
pub fn slice_parts<'a>(base: &'a [u8], outer: &'a Outer, address: u64, n: u64) -> R<std::borrow::Cow<'a, [u8]>> {
    let n = usize::try_from(n).or_else(|_| trap(Trap::OutOfBounds))?;
    let at = outer.at(address, 0, n)?;
    if at + n <= outer.fast && at + n <= base.len() {
        return Ok(std::borrow::Cow::Borrowed(&base[at..at + n]));
    }
    let mut out = vec![0; n];
    read_parts(base, outer, at, &mut out);
    Ok(std::borrow::Cow::Owned(out))
}

/// Writes `data` at `address` into a 64-bit memory's parts.
pub fn write_all_parts(base: &mut [u8], outer: &mut Outer, address: u64, data: &[u8]) -> R<()> {
    let at = outer.at(address, 0, data.len())?;
    write_parts(base, outer, at, data);
    Ok(())
}

// The parts of a 64-bit memory: `base` is its base, or (from generated
// code) the base's direct bytes, which hold every in-bounds byte below the
// base's length.

/// Where byte `at` (in bounds) lives: the base or a chunk, and the offset
/// there.
fn part<'a>(base: &'a [u8], outer: &'a Outer, at: usize) -> (&'a [u8], usize) {
    if at < outer.base_len {
        (base, at)
    } else {
        let at = at - outer.base_len;
        (&outer.chunks[at >> CHUNK_BITS], at & (CHUNK - 1))
    }
}

fn part_mut<'a>(base: &'a mut [u8], outer: &'a mut Outer, at: usize) -> (&'a mut [u8], usize) {
    if at < outer.base_len {
        (base, at)
    } else {
        let at = at - outer.base_len;
        (&mut outer.chunks[at >> CHUNK_BITS], at & (CHUNK - 1))
    }
}

/// Copies the bytes at `at` (in bounds) into `out`, across parts.
fn read_parts(base: &[u8], outer: &Outer, mut at: usize, mut out: &mut [u8]) {
    while !out.is_empty() {
        let (part, o) = part(base, outer, at);
        let n = out.len().min(part.len() - o);
        let (now, rest) = out.split_at_mut(n);
        now.copy_from_slice(&part[o..o + n]);
        out = rest;
        at += n;
    }
}

/// Writes `data` at `at` (in bounds), across parts.
fn write_parts(base: &mut [u8], outer: &mut Outer, mut at: usize, mut data: &[u8]) {
    while !data.is_empty() {
        let (part, o) = part_mut(base, outer, at);
        let n = data.len().min(part.len() - o);
        part[o..o + n].copy_from_slice(&data[..n]);
        data = &data[n..];
        at += n;
    }
}

#[cold]
pub fn fill_parts(base: &mut [u8], outer: &mut Outer, destination: u64, value: u8, n: u64) -> R<()> {
    let n = usize::try_from(n).or_else(|_| trap(Trap::OutOfBounds))?;
    let mut at = outer.at(destination, 0, n)?;
    let end = at + n;
    while at < end {
        let (part, o) = part_mut(base, outer, at);
        let k = (end - at).min(part.len() - o).min(BLOCK);
        let bytes = &mut part[o..o + k];
        // Zeros over a block of zeros: left untouched.
        if !(value == 0 && k == BLOCK && all_zero(bytes)) {
            bytes.fill(value);
        }
        at += k;
    }
    Ok(())
}

#[cold]
pub fn copy_parts(base: &mut [u8], outer: &mut Outer, destination: u64, source: u64, n: u64) -> R<()> {
    let n = usize::try_from(n).or_else(|_| trap(Trap::OutOfBounds))?;
    let to = outer.at(destination, 0, n)?;
    let from = outer.at(source, 0, n)?;
    if n < BLOCK && to + n <= outer.fast && from + n <= outer.fast {
        base.copy_within(from..from + n, to);
        return Ok(());
    }
    // Block by block through a bounded buffer, in memmove's direction so
    // that overlapping ranges copy right; a block of zeros over zeros is
    // skipped.
    let mut block = vec![0; BLOCK.min(n)];
    let mut there = vec![0; BLOCK.min(n)];
    let mut done = 0;
    while done < n {
        let k = BLOCK.min(n - done);
        let at = if to <= from { done } else { n - done - k };
        read_parts(base, outer, from + at, &mut block[..k]);
        if all_zero(&block[..k]) {
            read_parts(base, outer, to + at, &mut there[..k]);
            if all_zero(&there[..k]) {
                done += k;
                continue;
            }
        }
        write_parts(base, outer, to + at, &block[..k]);
        done += k;
    }
    Ok(())
}

/// Accesses to a 64-bit memory through its disjoint `base` buffer and active
/// length `fast`: an access ending within `fast` checks one bound (`address <=
/// fast - offset - N`) with zero Rust slice panic checks; others trap (flat)
/// or take a cold register-returned slow path through the chunks.
pub mod m64 {
    use super::{Outer, R, Trap, copy_parts, fill_parts, move_bytes, read_parts, trap, write_parts};

    #[inline(always)]
    fn direct<const N: usize>(base: &[u8], fast: usize, address: u64, offset: u64) -> Option<&[u8; N]> {
        let active = base.get(..fast)?;
        let off = usize::try_from(offset).ok()?;
        let sub = active.get(off..)?;
        let limit = sub.len().checked_sub(N)?;
        if address <= limit as u64 {
            let a = address as usize;
            return sub.get(a..a + N)?.try_into().ok();
        }
        None
    }

    #[inline(always)]
    fn direct_mut<const N: usize>(base: &mut [u8], fast: usize, address: u64, offset: u64) -> Option<&mut [u8; N]> {
        let active = base.get_mut(..fast)?;
        let off = usize::try_from(offset).ok()?;
        let sub = active.get_mut(off..)?;
        let limit = sub.len().checked_sub(N)?;
        if address <= limit as u64 {
            let a = address as usize;
            return sub.get_mut(a..a + N)?.try_into().ok();
        }
        None
    }

    #[inline(always)]
    fn direct_byte(base: &[u8], fast: usize, address: u64, offset: u64) -> Option<&u8> {
        let active = base.get(..fast)?;
        let off = usize::try_from(offset).ok()?;
        let sub = active.get(off..)?;
        sub.get(usize::try_from(address).ok()?)
    }

    #[inline(always)]
    fn direct_byte_mut(base: &mut [u8], fast: usize, address: u64, offset: u64) -> Option<&mut u8> {
        let active = base.get_mut(..fast)?;
        let off = usize::try_from(offset).ok()?;
        let sub = active.get_mut(off..)?;
        sub.get_mut(usize::try_from(address).ok()?)
    }

    /// The `n` (at most 8) bytes of an access outside the direct bytes,
    /// returned in registers (`Option<u64>`, zero `sret` stack slot and 5
    /// register arguments).
    #[cold]
    #[inline(never)]
    fn get_word_slow(base: &[u8], outer: &Outer, at: u64, n: usize) -> Option<u64> {
        let mut out = [0; 8];
        let at = outer.at(at, 0, n).ok()?;
        read_parts(base, outer, at, &mut out[..n]);
        Some(u64::from_le_bytes(out))
    }

    #[cold]
    #[inline(never)]
    fn get_128_slow(base: &[u8], outer: &Outer, at: u64) -> R<u128> {
        let mut out = [0; 16];
        let at = outer.at(at, 0, 16)?;
        read_parts(base, outer, at, &mut out);
        Ok(u128::from_le_bytes(out))
    }

    /// Writes `n` (at most 8) bytes outside the direct range; returns `true`
    /// on success and `false` on `OutOfBounds` (1-byte register return, 6
    /// register arguments, zero stack spills).
    #[cold]
    #[inline(never)]
    fn put_word_slow(base: &mut [u8], outer: &mut Outer, at: u64, value: u64, n: usize) -> bool {
        let Ok(at) = outer.at(at, 0, n) else { return false };
        write_parts(base, outer, at, &value.to_le_bytes()[..n]);
        true
    }

    #[cold]
    #[inline(never)]
    fn put_128_slow(base: &mut [u8], outer: &mut Outer, at: u64, value: u128) -> bool {
        let Ok(at) = outer.at(at, 0, 16) else { return false };
        write_parts(base, outer, at, &value.to_le_bytes());
        true
    }

    macro_rules! access_word {
        ($load:ident, $store:ident, $t:ty, $n:expr) => {
            #[inline(always)]
            pub fn $load(base: &[u8], fast: usize, outer: &Outer, address: u64, offset: u64) -> R<$t> {
                if let Some(bytes) = direct::<$n>(base, fast, address, offset) {
                    return Ok(<$t>::from_le_bytes(*bytes));
                }
                let Some(at) = address.checked_add(offset) else { return trap(Trap::OutOfBounds) };
                match get_word_slow(base, outer, at, $n) {
                    Some(value) => Ok(value as $t),
                    None => trap(Trap::OutOfBounds),
                }
            }
            #[inline(always)]
            pub fn $store(base: &mut [u8], fast: usize, outer: &mut Outer, address: u64, offset: u64, value: $t) -> R<()> {
                if let Some(bytes) = direct_mut::<$n>(base, fast, address, offset) {
                    *bytes = value.to_le_bytes();
                    return Ok(());
                }
                let Some(at) = address.checked_add(offset) else { return trap(Trap::OutOfBounds) };
                if put_word_slow(base, outer, at, value as u64, $n) { Ok(()) } else { trap(Trap::OutOfBounds) }
            }
        };
    }

    #[inline(always)]
    pub fn l128(base: &[u8], fast: usize, outer: &Outer, address: u64, offset: u64) -> R<u128> {
        if let Some(bytes) = direct::<16>(base, fast, address, offset) {
            return Ok(u128::from_le_bytes(*bytes));
        }
        let Some(at) = address.checked_add(offset) else { return trap(Trap::OutOfBounds) };
        get_128_slow(base, outer, at)
    }
    #[inline(always)]
    pub fn s128(base: &mut [u8], fast: usize, outer: &mut Outer, address: u64, offset: u64, value: u128) -> R<()> {
        if let Some(bytes) = direct_mut::<16>(base, fast, address, offset) {
            *bytes = value.to_le_bytes();
            return Ok(());
        }
        let Some(at) = address.checked_add(offset) else { return trap(Trap::OutOfBounds) };
        if put_128_slow(base, outer, at, value) { Ok(()) } else { trap(Trap::OutOfBounds) }
    }

    // Flat accesses, for a memory made by `Memory::new_flat`: its direct
    // bytes are all its bytes, so a miss traps with one compare and branch.
    macro_rules! flat {
        ($load:ident, $store:ident, $t:ty, $n:expr) => {
            #[inline(always)]
            pub fn $load(base: &[u8], len: usize, address: u64, offset: u64) -> R<$t> {
                match direct::<$n>(base, len, address, offset) {
                    Some(bytes) => Ok(<$t>::from_le_bytes(*bytes)),
                    None => trap(Trap::OutOfBounds),
                }
            }
            #[inline(always)]
            pub fn $store(base: &mut [u8], len: usize, address: u64, offset: u64, value: $t) -> R<()> {
                match direct_mut::<$n>(base, len, address, offset) {
                    Some(bytes) => {
                        *bytes = value.to_le_bytes();
                        Ok(())
                    }
                    None => trap(Trap::OutOfBounds),
                }
            }
        };
    }
    #[inline(always)]
    pub fn fl8(base: &[u8], len: usize, address: u64, offset: u64) -> R<u8> {
        match direct_byte(base, len, address, offset) {
            Some(&b) => Ok(b),
            None => trap(Trap::OutOfBounds),
        }
    }
    #[inline(always)]
    pub fn fs8(base: &mut [u8], len: usize, address: u64, offset: u64, value: u8) -> R<()> {
        match direct_byte_mut(base, len, address, offset) {
            Some(slot) => {
                *slot = value;
                Ok(())
            }
            None => trap(Trap::OutOfBounds),
        }
    }
    flat!(fl16, fs16, u16, 2);
    flat!(fl32, fs32, u32, 4);
    flat!(fl64, fs64, u64, 8);
    flat!(fl128, fs128, u128, 16);

    #[inline(always)]
    pub fn l8(base: &[u8], fast: usize, outer: &Outer, address: u64, offset: u64) -> R<u8> {
        if let Some(&b) = direct_byte(base, fast, address, offset) {
            return Ok(b);
        }
        let Some(at) = address.checked_add(offset) else { return trap(Trap::OutOfBounds) };
        match get_word_slow(base, outer, at, 1) {
            Some(value) => Ok(value as u8),
            None => trap(Trap::OutOfBounds),
        }
    }
    #[inline(always)]
    pub fn s8(base: &mut [u8], fast: usize, outer: &mut Outer, address: u64, offset: u64, value: u8) -> R<()> {
        if let Some(slot) = direct_byte_mut(base, fast, address, offset) {
            *slot = value;
            return Ok(());
        }
        let Some(at) = address.checked_add(offset) else { return trap(Trap::OutOfBounds) };
        if put_word_slow(base, outer, at, value as u64, 1) { Ok(()) } else { trap(Trap::OutOfBounds) }
    }
    access_word!(l16, s16, u16, 2);
    access_word!(l32, s32, u32, 4);
    access_word!(l64, s64, u64, 8);

    /// `memory.copy` of a constant `N` bytes (at least 1) on a flat 64-bit memory.
    #[inline(always)]
    pub fn fcopy_n<const N: usize>(base: &mut [u8], len: usize, destination: u64, source: u64) -> R<()> {
        let Some(active) = base.get_mut(..len) else { return trap(Trap::OutOfBounds) };
        let Some(limit) = active.len().checked_sub(N) else { return trap(Trap::OutOfBounds) };
        if destination <= limit as u64 && source <= limit as u64 {
            move_bytes::<N>(active, destination as usize, source as usize);
            return Ok(());
        }
        trap(Trap::OutOfBounds)
    }

    /// `memory.fill` of a constant `N` bytes (at least 1) on a flat 64-bit memory.
    #[inline(always)]
    pub fn ffill_n<const N: usize>(base: &mut [u8], len: usize, destination: u64, value: u8) -> R<()> {
        let Some(active) = base.get_mut(..len) else { return trap(Trap::OutOfBounds) };
        let Some(limit) = active.len().checked_sub(N) else { return trap(Trap::OutOfBounds) };
        if destination <= limit as u64
            && let Some(dst) = active.get_mut(destination as usize..).and_then(|s| s.first_chunk_mut::<N>())
        {
            dst.fill(value);
            return Ok(());
        }
        trap(Trap::OutOfBounds)
    }

    /// `memory.copy` of a constant `N` bytes (at least 1) on a chunked 64-bit memory.
    #[inline(always)]
    pub fn copy_n<const N: usize>(base: &mut [u8], fast: usize, outer: &mut Outer, destination: u64, source: u64) -> R<()> {
        if let Some(active) = base.get_mut(..fast)
            && let Some(limit) = active.len().checked_sub(N)
            && destination <= limit as u64
            && source <= limit as u64
        {
            move_bytes::<N>(active, destination as usize, source as usize);
            return Ok(());
        }
        copy_parts(base, outer, destination, source, N as u64)
    }

    /// `memory.fill` of a constant `N` bytes (at least 1) on a chunked 64-bit memory.
    #[inline(always)]
    pub fn fill_n<const N: usize>(base: &mut [u8], fast: usize, outer: &mut Outer, destination: u64, value: u8) -> R<()> {
        if let Some(active) = base.get_mut(..fast)
            && let Some(limit) = active.len().checked_sub(N)
            && destination <= limit as u64
            && let Some(dst) = active.get_mut(destination as usize..).and_then(|s| s.first_chunk_mut::<N>())
        {
            dst.fill(value);
            return Ok(());
        }
        fill_parts(base, outer, destination, value, N as u64)
    }
}

/// Accesses to a flat 64-bit memory backed by a fixed-capacity array
/// (`&mut [u8; CAP]`): `CAP` is a compile-time constant so LLVM propagates
/// `MIN_BYTES <= len <= CAP` even across `memory.grow` PHI nodes and emits
/// a single `cmpq (len - offset - N), address; ja trap` per access while
/// passing the memory pointer in one register instead of a fat slice.
pub mod m64f {
    use super::{R, Trap, move_bytes, trap};

    #[inline(always)]
    fn direct<const CAP: usize, const N: usize>(bytes: &[u8; CAP], len: usize, address: u64, offset: u64) -> Option<&[u8; N]> {
        if len > CAP {
            return None;
        }
        let k = usize::try_from(offset.checked_add(N as u64)?).ok()?;
        let limit = len.checked_sub(k)?;
        if address <= limit as u64 {
            let a = (address as usize).wrapping_add(offset as usize);
            return bytes.get(a..)?.first_chunk::<N>();
        }
        None
    }

    #[inline(always)]
    fn direct_mut<const CAP: usize, const N: usize>(bytes: &mut [u8; CAP], len: usize, address: u64, offset: u64) -> Option<&mut [u8; N]> {
        if len > CAP {
            return None;
        }
        let k = usize::try_from(offset.checked_add(N as u64)?).ok()?;
        let limit = len.checked_sub(k)?;
        if address <= limit as u64 {
            let a = (address as usize).wrapping_add(offset as usize);
            return bytes.get_mut(a..)?.first_chunk_mut::<N>();
        }
        None
    }

    macro_rules! access {
        ($load:ident, $store:ident, $t:ty, $n:expr) => {
            #[inline(always)]
            pub fn $load<const CAP: usize>(bytes: &[u8; CAP], len: usize, address: u64, offset: u64) -> R<$t> {
                match direct::<CAP, $n>(bytes, len, address, offset) {
                    Some(b) => Ok(<$t>::from_le_bytes(*b)),
                    None => trap(Trap::OutOfBounds),
                }
            }
            #[inline(always)]
            pub fn $store<const CAP: usize>(bytes: &mut [u8; CAP], len: usize, address: u64, offset: u64, value: $t) -> R<()> {
                match direct_mut::<CAP, $n>(bytes, len, address, offset) {
                    Some(b) => {
                        *b = value.to_le_bytes();
                        Ok(())
                    }
                    None => trap(Trap::OutOfBounds),
                }
            }
        };
    }

    access!(l8, s8, u8, 1);
    access!(l16, s16, u16, 2);
    access!(l32, s32, u32, 4);
    access!(l64, s64, u64, 8);
    access!(l128, s128, u128, 16);

    /// `memory.copy` of a constant `N` bytes (at least 1) on a fixed-capacity flat 64-bit memory.
    #[inline(always)]
    pub fn copy_n<const CAP: usize, const N: usize>(bytes: &mut [u8; CAP], len: usize, destination: u64, source: u64) -> R<()> {
        if len <= CAP
            && let Some(limit) = len.checked_sub(N)
            && destination <= limit as u64
            && source <= limit as u64
        {
            move_bytes::<N>(bytes, destination as usize, source as usize);
            return Ok(());
        }
        trap(Trap::OutOfBounds)
    }

    /// `memory.fill` of a constant `N` bytes (at least 1) on a fixed-capacity flat 64-bit memory.
    #[inline(always)]
    pub fn fill_n<const CAP: usize, const N: usize>(bytes: &mut [u8; CAP], len: usize, destination: u64, value: u8) -> R<()> {
        if len <= CAP
            && let Some(limit) = len.checked_sub(N)
            && destination <= limit as u64
            && let Some(dst) = bytes.get_mut(destination as usize..).and_then(|s| s.first_chunk_mut::<N>())
        {
            dst.fill(value);
            return Ok(());
        }
        trap(Trap::OutOfBounds)
    }
}

/// Allocates a lazily-zeroed fixed-capacity byte array for a flat 64-bit memory.
pub fn alloc64<const CAP: usize>() -> R<Box<[u8; CAP]>> {
    match zeroed(CAP)?.into_boxed_slice().try_into() {
        Ok(bytes) => Ok(bytes),
        Err(_) => trap(Trap::AllocationFailed),
    }
}

/// What the host reads and writes in a module's memory.
pub trait LinearMemory {
    /// The `n` bytes at `address`.
    fn read(&self, address: u64, n: u64) -> R<Vec<u8>>;
    /// Borrows `n` bytes at `address` without copying when contiguous.
    fn slice(&self, address: u64, n: u64) -> R<std::borrow::Cow<'_, [u8]>> {
        self.read(address, n).map(std::borrow::Cow::Owned)
    }
    /// Writes `data` at `address`.
    fn write(&mut self, address: u64, data: &[u8]) -> R<()>;
}

impl LinearMemory for Memory {
    fn read(&self, address: u64, n: u64) -> R<Vec<u8>> {
        Memory::read(self, address, n)
    }
    fn slice(&self, address: u64, n: u64) -> R<std::borrow::Cow<'_, [u8]>> {
        slice_parts(&self.base, &self.outer, address, n)
    }
    fn write(&mut self, address: u64, data: &[u8]) -> R<()> {
        Memory::write(self, address, data)
    }
}

/// A borrowed host view of a split 64-bit memory (`base` + owned `Outer`).
pub struct Mem64Host<'a> {
    pub base: &'a mut [u8],
    pub outer: Outer,
}

impl LinearMemory for Mem64Host<'_> {
    fn read(&self, address: u64, n: u64) -> R<Vec<u8>> {
        Ok(slice_parts(self.base, &self.outer, address, n)?.into_owned())
    }
    fn slice(&self, address: u64, n: u64) -> R<std::borrow::Cow<'_, [u8]>> {
        slice_parts(self.base, &self.outer, address, n)
    }
    fn write(&mut self, address: u64, data: &[u8]) -> R<()> {
        write_all_parts(self.base, &mut self.outer, address, data)
    }
}

/// A borrowed host view of a fixed-capacity flat 64-bit memory (`&mut [u8; CAP]` + `Outer`).
pub struct Mem64FixedHost<'a, const CAP: usize> {
    pub base: &'a mut [u8; CAP],
    pub outer: Outer,
}

impl<const CAP: usize> LinearMemory for Mem64FixedHost<'_, CAP> {
    fn read(&self, address: u64, n: u64) -> R<Vec<u8>> {
        Ok(slice_parts(self.base, &self.outer, address, n)?.into_owned())
    }
    fn slice(&self, address: u64, n: u64) -> R<std::borrow::Cow<'_, [u8]>> {
        slice_parts(self.base, &self.outer, address, n)
    }
    fn write(&mut self, address: u64, data: &[u8]) -> R<()> {
        write_all_parts(self.base, &mut self.outer, address, data)
    }
}

/// Bytes a 32-bit memory spans: every address plus offset, both 32-bit,
/// and an access of up to 16 bytes.
pub const SPAN32: usize = (1 << 33) + 16;

/// Metadata of a split 32-bit linear memory, kept inside `Ctx` so the byte
/// buffer (`&mut [u8; CAP]`) can be passed as a disjoint `noalias` pointer.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Mem32Meta {
    pub len: usize,
    pub max_pages: u64,
}

impl Mem32Meta {
    #[inline(always)]
    pub const fn empty() -> Mem32Meta {
        Mem32Meta { len: 0, max_pages: 0 }
    }

    pub fn new(min: u64, max: Option<u64>) -> R<Mem32Meta> {
        if min > page_limit() {
            return trap(Trap::MemoryLimit);
        }
        let max_pages = max.unwrap_or(1 << 16).min(1 << 16).min(page_limit());
        let Some(len) = min.checked_mul(PAGE).and_then(|n| usize::try_from(n).ok()) else {
            return trap(Trap::MemoryTooLarge);
        };
        if min > max_pages {
            return trap(Trap::MemoryTooLarge);
        }
        Ok(Mem32Meta { len, max_pages })
    }

    #[inline(always)]
    pub fn size(&self) -> u64 {
        self.len as u64 / PAGE
    }

    #[cold]
    pub fn grow(&mut self, delta: u64) -> i64 {
        let old = self.size();
        let Some(new) = old.checked_add(delta).filter(|&n| n <= self.max_pages) else { return -1 };
        self.len = (new * PAGE) as usize;
        old as i64
    }

    fn range(&self, address: u64, n: u64) -> R<std::ops::Range<usize>> {
        match address.checked_add(n) {
            Some(end) if end <= self.len as u64 => Ok(address as usize..end as usize),
            _ => trap(Trap::OutOfBounds),
        }
    }

    #[cold]
    pub fn fill(&self, bytes: &mut [u8], destination: u64, value: u8, n: u64) -> R<()> {
        let range = self.range(destination, n)?;
        let Some(dst) = bytes.get_mut(range) else { return trap(Trap::OutOfBounds) };
        dst.fill(value);
        Ok(())
    }

    #[cold]
    pub fn copy(&self, bytes: &mut [u8], destination: u64, source: u64, n: u64) -> R<()> {
        let to = self.range(destination, n)?;
        let from = self.range(source, n)?;
        if to.end > bytes.len() || from.end > bytes.len() {
            return trap(Trap::OutOfBounds);
        }
        bytes.copy_within(from, to.start);
        Ok(())
    }

    pub fn init(&self, bytes: &mut [u8], destination: u64, data: &[u8], from: u64, n: u64) -> R<()> {
        let source = usize::try_from(from).ok().zip(usize::try_from(n).ok());
        let Some(source) = source.and_then(|(f, n)| data.get(f..f.checked_add(n)?)) else {
            return trap(Trap::OutOfBounds);
        };
        let to = self.range(destination, n)?;
        let Some(dst) = bytes.get_mut(to) else { return trap(Trap::OutOfBounds) };
        dst.copy_from_slice(source);
        Ok(())
    }

    pub fn slice<'a>(&self, bytes: &'a [u8], address: u64, n: u64) -> R<std::borrow::Cow<'a, [u8]>> {
        let range = self.range(address, n)?;
        let Some(src) = bytes.get(range) else { return trap(Trap::OutOfBounds) };
        Ok(std::borrow::Cow::Borrowed(src))
    }

    pub fn write(&self, bytes: &mut [u8], address: u64, data: &[u8]) -> R<()> {
        let range = self.range(address, data.len() as u64)?;
        let Some(dst) = bytes.get_mut(range) else { return trap(Trap::OutOfBounds) };
        dst.copy_from_slice(data);
        Ok(())
    }
}

/// Allocates a lazily-zeroed fixed-capacity byte array for a 32-bit memory.
pub fn alloc32<const CAP: usize>() -> R<Box<[u8; CAP]>> {
    match zeroed(CAP)?.into_boxed_slice().try_into() {
        Ok(bytes) => Ok(bytes),
        Err(_) => trap(Trap::AllocationFailed),
    }
}

/// A borrowed host view of a split 32-bit memory (`bytes` + `Mem32Meta`).
pub struct Mem32Host<'a, const CAP: usize> {
    pub bytes: &'a mut [u8; CAP],
    pub meta: Mem32Meta,
}

impl<const CAP: usize> LinearMemory for Mem32Host<'_, CAP> {
    fn read(&self, address: u64, n: u64) -> R<Vec<u8>> {
        Ok(self.meta.slice(self.bytes, address, n)?.into_owned())
    }
    fn slice(&self, address: u64, n: u64) -> R<std::borrow::Cow<'_, [u8]>> {
        self.meta.slice(self.bytes, address, n)
    }
    fn write(&mut self, address: u64, data: &[u8]) -> R<()> {
        self.meta.write(self.bytes, address, data)
    }
}

/// Accesses to a 32-bit memory through its disjoint byte array (`&mut [u8; CAP]`)
/// and active length `len`. Using `active.get(offset..)` + `checked_sub(N)` +
/// `&sub[a..a + N]` gives a single hoisted-limit compare per access, direct
/// typed loads/stores (no split-byte reassembly), and enables LLVM
/// auto-vectorization.
pub mod m32 {
    use super::{R, Trap, move_bytes, trap};

    #[inline(always)]
    fn direct<const CAP: usize, const N: usize>(bytes: &[u8; CAP], len: usize, address: u32, offset: u32) -> Option<&[u8; N]> {
        let active = bytes.get(..len)?;
        if offset == 0 && N >= 4 {
            let limit = active.len().checked_sub(N)?;
            let a = address as usize;
            if a <= limit {
                return active.get(a..)?.first_chunk::<N>();
            }
            None
        } else {
            let end = (address as usize).checked_add(offset as usize)?.checked_add(N)?;
            active.get(..end)?.last_chunk::<N>()
        }
    }

    #[inline(always)]
    fn direct_mut<const CAP: usize, const N: usize>(bytes: &mut [u8; CAP], len: usize, address: u32, offset: u32) -> Option<&mut [u8; N]> {
        let active = bytes.get_mut(..len)?;
        if offset == 0 && N >= 4 {
            let limit = active.len().checked_sub(N)?;
            let a = address as usize;
            if a <= limit {
                return active.get_mut(a..)?.first_chunk_mut::<N>();
            }
            None
        } else {
            let end = (address as usize).checked_add(offset as usize)?.checked_add(N)?;
            active.get_mut(..end)?.last_chunk_mut::<N>()
        }
    }

    #[inline(always)]
    pub fn l8<const CAP: usize>(bytes: &[u8; CAP], len: usize, address: u32, offset: u32) -> R<u8> {
        let Some(active) = bytes.get(..len) else { return trap(Trap::OutOfBounds) };
        let Some(at) = (address as usize).checked_add(offset as usize) else { return trap(Trap::OutOfBounds) };
        match active.get(at) {
            Some(&b) => Ok(b),
            None => trap(Trap::OutOfBounds),
        }
    }

    #[inline(always)]
    pub fn s8<const CAP: usize>(bytes: &mut [u8; CAP], len: usize, address: u32, offset: u32, value: u8) -> R<()> {
        let Some(active) = bytes.get_mut(..len) else { return trap(Trap::OutOfBounds) };
        let Some(at) = (address as usize).checked_add(offset as usize) else { return trap(Trap::OutOfBounds) };
        match active.get_mut(at) {
            Some(slot) => {
                *slot = value;
                Ok(())
            }
            None => trap(Trap::OutOfBounds),
        }
    }

    macro_rules! access {
        ($load:ident, $store:ident, $t:ty, $n:expr) => {
            #[inline(always)]
            pub fn $load<const CAP: usize>(bytes: &[u8; CAP], len: usize, address: u32, offset: u32) -> R<$t> {
                match direct::<CAP, $n>(bytes, len, address, offset) {
                    Some(b) => Ok(<$t>::from_le_bytes(*b)),
                    None => trap(Trap::OutOfBounds),
                }
            }
            #[inline(always)]
            pub fn $store<const CAP: usize>(bytes: &mut [u8; CAP], len: usize, address: u32, offset: u32, value: $t) -> R<()> {
                match direct_mut::<CAP, $n>(bytes, len, address, offset) {
                    Some(b) => {
                        *b = value.to_le_bytes();
                        Ok(())
                    }
                    None => trap(Trap::OutOfBounds),
                }
            }
        };
    }

    access!(l16, s16, u16, 2);
    access!(l32, s32, u32, 4);
    access!(l64, s64, u64, 8);
    access!(l128, s128, u128, 16);

    /// `memory.copy` of a constant `N` bytes (at least 1).
    #[inline(always)]
    pub fn copy_n<const CAP: usize, const N: usize>(bytes: &mut [u8; CAP], len: usize, destination: u32, source: u32) -> R<()> {
        let Some(active) = bytes.get_mut(..len) else { return trap(Trap::OutOfBounds) };
        let Some(limit) = active.len().checked_sub(N) else { return trap(Trap::OutOfBounds) };
        if destination as usize <= limit && source as usize <= limit {
            move_bytes::<N>(active, destination as usize, source as usize);
            return Ok(());
        }
        trap(Trap::OutOfBounds)
    }

    /// `memory.fill` of a constant `N` bytes (at least 1).
    #[inline(always)]
    pub fn fill_n<const CAP: usize, const N: usize>(bytes: &mut [u8; CAP], len: usize, destination: u32, value: u8) -> R<()> {
        let Some(active) = bytes.get_mut(..len) else { return trap(Trap::OutOfBounds) };
        let Some(limit) = active.len().checked_sub(N) else { return trap(Trap::OutOfBounds) };
        let at = destination as usize;
        if at <= limit
            && let Some(dst) = active.get_mut(at..).and_then(|s| s.first_chunk_mut::<N>())
        {
            dst.fill(value);
            return Ok(());
        }
        trap(Trap::OutOfBounds)
    }
}

/// Moves `N` bytes (`from..from + N` to `to`, both in bounds) as memmove
/// does, without a call where it can: through a value up to 64 bytes, as
/// 32-byte blocks through registers when the ranges are disjoint (the
/// usual case: struct copies), else by `copy_within`.
#[inline(always)]
fn move_bytes<const N: usize>(bytes: &mut [u8], to: usize, from: usize) {
    if N <= 64 {
        if let Some(value) = bytes.get(from..).and_then(|s| s.first_chunk::<N>()).copied()
            && let Some(target) = bytes.get_mut(to..).and_then(|s| s.first_chunk_mut::<N>())
        {
            *target = value;
        }
    } else if to + N <= from || from + N <= to {
        let mut k = 0;
        loop {
            let at = if k + 32 > N { N - 32 } else { k };
            if let Some(block) = bytes.get(from + at..).and_then(|s| s.first_chunk::<32>()).copied()
                && let Some(target) = bytes.get_mut(to + at..).and_then(|s| s.first_chunk_mut::<32>())
            {
                *target = block;
            }
            if at + 32 >= N {
                break;
            }
            k += 32;
        }
    } else {
        bytes.copy_within(from..from + N, to);
    }
}

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
        if min > page_limit() {
            return trap(Trap::MemoryLimit);
        }
        let max_pages = max.unwrap_or(1 << 16).min(1 << 16).min(page_limit());
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

    /// The current length in bytes.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    /// The memory has no bytes.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The bytes, for `m32`'s accesses.
    #[inline(always)]
    pub fn bytes_mut(&mut self) -> &mut [u8; SPAN32] {
        &mut self.bytes
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

    /// `memory.copy` of a constant `N` bytes (at least 1): both ranges are
    /// checked before anything is written. Up to 64 bytes pass through a
    /// value, which LLVM keeps in registers; longer copies are one memmove
    /// (through a value they would be two memcpy calls). Either way
    /// overlapping ranges copy as memmove does.
    #[inline(always)]
    pub fn copy_n<const N: usize>(&mut self, destination: u32, source: u32) -> R<()> {
        let to = self.at::<N>(destination, 0)?;
        let from = self.at::<N>(source, 0)?;
        move_bytes::<N>(&mut self.bytes[..], to, from);
        Ok(())
    }

    /// `memory.fill` of a constant `N` bytes (at least 1).
    #[inline(always)]
    pub fn fill_n<const N: usize>(&mut self, destination: u32, value: u8) -> R<()> {
        let at = self.at::<N>(destination, 0)?;
        self.bytes[at..at + N].fill(value);
        Ok(())
    }

    /// The size in pages.
    #[inline(always)]
    pub fn size(&self) -> u64 {
        self.len as u64 / PAGE
    }

    /// Grows by `delta` pages; the old size, or -1 when it cannot.
    #[cold]
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
    fn slice(&self, address: u64, n: u64) -> R<std::borrow::Cow<'_, [u8]>> {
        let range = self.range(address, n)?;
        Ok(std::borrow::Cow::Borrowed(&self.bytes[range]))
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
    fn constant_copies_overlap_and_check_both_ranges() {
        let mut m = Memory32::new(1, Some(1)).unwrap();
        for k in 0..8u8 {
            m.s8(k as u32, 0, k + 1).unwrap();
        }
        // Forward and backward overlap, as memmove.
        m.copy_n::<6>(2, 0).unwrap();
        assert_eq!(m.l64(0, 0), Ok(u64::from_le_bytes([1, 2, 1, 2, 3, 4, 5, 6])));
        m.copy_n::<6>(0, 2).unwrap();
        assert_eq!(m.l64(0, 0), Ok(u64::from_le_bytes([1, 2, 3, 4, 5, 6, 5, 6])));
        // Either range out of bounds traps and writes nothing.
        assert_eq!(m.copy_n::<4>(65533, 0), Err(Trap::OutOfBounds));
        assert_eq!(m.copy_n::<4>(0, 65533), Err(Trap::OutOfBounds));
        assert_eq!(m.copy_n::<4>(u32::MAX, 0), Err(Trap::OutOfBounds));
        assert_eq!(m.l64(0, 0), Ok(u64::from_le_bytes([1, 2, 3, 4, 5, 6, 5, 6])));
        // Long copies take the memmove path.
        for k in 0..100u8 {
            m.s8(1000 + k as u32, 0, k).unwrap();
        }
        // Disjoint long copies take the block path; a tail shorter than a
        // block included.
        for n in [65usize, 96, 100, 160, 255] {
            let mut want = vec![0u8; n];
            for (k, byte) in want.iter_mut().enumerate() {
                *byte = (k * 7 + n) as u8;
            }
            m.write(3000, &want).unwrap();
            match n {
                65 => m.copy_n::<65>(5000, 3000).unwrap(),
                96 => m.copy_n::<96>(5000, 3000).unwrap(),
                100 => m.copy_n::<100>(5000, 3000).unwrap(),
                160 => m.copy_n::<160>(5000, 3000).unwrap(),
                _ => m.copy_n::<255>(5000, 3000).unwrap(),
            }
            assert_eq!(m.read(5000, n as u64).unwrap(), want, "{n}");
            assert_eq!(m.l8(5000 + n as u32, 0), Ok(0), "{n}: nothing past the end");
        }
        m.fill_n::<7>(6000, 9).unwrap();
        assert_eq!(m.read(5999, 9).unwrap(), vec![0, 9, 9, 9, 9, 9, 9, 9, 0]);
        assert_eq!(m.fill_n::<8>(65530, 1), Err(Trap::OutOfBounds));
        m.copy_n::<90>(1010, 1000).unwrap();
        assert_eq!(m.l8(1010 + 89, 0), Ok(89));
        assert_eq!(m.l8(1010, 0), Ok(0));
        assert_eq!(m.copy_n::<90>(65500, 0), Err(Trap::OutOfBounds));
        assert!(m.copy_n::<4>(65532, 0).is_ok());
        assert_eq!(m.l32(65532, 0), Ok(u32::from_le_bytes([1, 2, 3, 4])));
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

    #[test]
    fn flat_memory64_traps_on_every_miss() {
        let mut m = Memory::new_flat(1, Some(3), true).unwrap();
        assert_eq!(m.grow(2), 1);
        assert_eq!(m.grow(1), -1);
        let end = 3 * PAGE;
        let len = m.outer.fast();
        assert_eq!(len as u64, end);
        let base = &mut m.base[..];
        assert_eq!(m64::fs64(base, len, end - 8, 0, 42), Ok(()));
        assert_eq!(m64::fl64(base, len, end - 8, 0), Ok(42));
        assert_eq!(m64::fl64(base, len, end - 7, 0), Err(Trap::OutOfBounds));
        assert_eq!(m64::fl8(base, len, u64::MAX, 1), Err(Trap::OutOfBounds));
        assert_eq!(m64::fs128(base, len, 0, end - 15, 1), Err(Trap::OutOfBounds));
        assert_eq!(m64::fl32(base, len, 1 << 62, 0), Err(Trap::OutOfBounds));
        assert_eq!(m64::fcopy_n::<8>(base, len, 0, end - 8), Ok(()));
        assert_eq!(m64::fl64(base, len, 0, 0), Ok(42));
        assert_eq!(m64::fcopy_n::<8>(base, len, end - 7, 0), Err(Trap::OutOfBounds));
        assert_eq!(m64::ffill_n::<4>(base, len, 16, 7), Ok(()));
        assert_eq!(m64::fl32(base, len, 16, 0), Ok(0x0707_0707));
        assert_eq!(m64::ffill_n::<4>(base, len, end - 3, 7), Err(Trap::OutOfBounds));
        // 64 GiB at once: granted or refused cleanly, depending on the machine.
        assert!(matches!(Memory::new_flat(1, None, true).map(|_| ()), Ok(()) | Err(Trap::AllocationFailed)));
    }

    #[test]
    fn m64_accesses_agree_with_the_memory() {
        let page = PAGE as usize;
        let edge = 2 * PAGE;
        let mut m = Memory::with_base(4, 1 << 20, vec![0; 2 * page]).unwrap();
        let mut seed = 12345u64;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            seed >> 33
        };
        for _ in 0..20000 {
            let address = match next() % 4 {
                0 => edge - 20 + next() % 40,
                1 => 4 * PAGE - 20 + next() % 40,
                2 => next() % (4 * PAGE),
                _ => u64::MAX - next() % 40,
            };
            let offset = next() % 24;
            let value = next() as u128 * 0x1_0000_0001 + 7;
            let expected = m.store::<8>(address, offset, (value as u64).to_le_bytes());
            let fast = m.outer.fast();
            assert!(!m.outer.is_flat());
            let got = m64::s64(&mut m.base, fast, &mut m.outer, address, offset, value as u64);
            assert_eq!(got, expected);
            let expected = m.load::<16>(address, offset).map(u128::from_le_bytes);
            assert_eq!(m64::l128(&m.base, fast, &m.outer, address, offset), expected, "{address} {offset}");
            let expected = m.load::<4>(address, offset).map(u32::from_le_bytes);
            assert_eq!(m64::l32(&m.base, fast, &m.outer, address, offset), expected);
            if m64::s16(&mut m.base, fast, &mut m.outer, address, offset, value as u16).is_ok() {
                assert_eq!(m.load::<2>(address, offset).map(u16::from_le_bytes), Ok(value as u16));
            }
        }
        let fast = m.outer.fast();
        m64::copy_n::<100>(&mut m.base, fast, &mut m.outer, edge - 50, 10).unwrap();
        m64::fill_n::<9>(&mut m.base, fast, &mut m.outer, edge - 4, 3).unwrap();
        assert_eq!(m.read(edge - 4, 9).unwrap(), vec![3; 9]);

        // Also test is_flat() transitioning from true to false on grow past base_len.
        let mut g = Memory::with_base(1, 1 << 20, vec![0; 2 * page]).unwrap();
        assert!(g.outer.is_flat());
        assert_eq!(m64::fs64(&mut g.base, g.outer.fast(), PAGE - 8, 0, 99), Ok(()));
        assert_eq!(m64::fl64(&g.base, g.outer.fast(), PAGE - 7, 0), Err(Trap::OutOfBounds));
        assert_eq!(g.grow(2), 1);
        assert!(!g.outer.is_flat());
        assert_eq!(m64::s64(&mut g.base, g.outer.fast(), &mut g.outer, 2 * PAGE - 4, 0, 0x1122_3344_5566_7788), Ok(()));
        assert_eq!(m64::l64(&g.base, g.outer.fast(), &g.outer, 2 * PAGE - 4, 0), Ok(0x1122_3344_5566_7788));
    }

    #[test]
    fn memory64_limits_match_wtime_goose() {
        assert_eq!(Memory::new(1 << 48, None, true).err(), Some(Trap::MemoryAllocationOverflow));
        assert_eq!(Memory::new((1 << 20) + 1, None, true).err(), Some(Trap::MemoryLimit));
        let mut m = Memory::new(1, None, true).unwrap();
        assert_eq!(m.grow(1 << 20), -1);
        assert_eq!(m.grow((1 << 20) - 1), 1);
        assert_eq!(m.size(), 1 << 20);
    }

    #[test]
    fn long_copies_overlap_both_ways_across_parts() {
        let page = PAGE as usize;
        for (to, from) in [(1000usize, 0usize), (0, 1000), (page - 7, 3 * page / 2), (3 * page / 2, page - 7)] {
            let mut m = Memory::with_base(8, 1 << 20, vec![0; 2 * page]).unwrap();
            let mut model = vec![0u8; 8 * page];
            for (k, byte) in model.iter_mut().enumerate().take(5 * page) {
                *byte = (k % 251) as u8;
            }
            m.write(0, &model).unwrap();
            let n = 3 * page + 123;
            model.copy_within(from..from + n, to);
            m.copy(to as u64, from as u64, n as u64).unwrap();
            assert_eq!(m.read(0, model.len() as u64).unwrap(), model, "copy {from} -> {to}");
        }
    }

    #[test]
    fn memory64_accesses_straddle_the_base_and_chunks() {
        // A base of 2 pages, then chunks: the edges at 2 pages and at
        // 2 pages + 4 GiB.
        for edge in [2 * PAGE, 2 * PAGE + (1 << 32)] {
            straddles(Memory::with_base(edge / PAGE + 1, 1 << 48, vec![0; 2 * PAGE as usize]).unwrap(), edge);
        }
        let edge = 1u64 << 32;
        straddles(Memory::new(edge / PAGE + 1, None, true).unwrap(), edge);
    }

    fn straddles(mut m: Memory, edge: u64) {
        m.s64(edge - 4, 0, 0x0102_0304_0506_0708).unwrap();
        assert_eq!(m.l64(edge - 4, 0), Ok(0x0102_0304_0506_0708));
        assert_eq!(m.l32(edge, 0), Ok(0x0102_0304));
        assert_eq!(m.load::<16>(edge - 8, 0).map(|b| b[4..12].to_vec()), Ok(vec![8, 7, 6, 5, 4, 3, 2, 1]));
        m.fill(edge - 2, 9, 4).unwrap();
        assert_eq!(m.l32(edge - 2, 0), Ok(0x0909_0909));
        // Overlapping copies across the boundary, both ways.
        m.copy(edge - 3, edge - 4, 8).unwrap();
        assert_eq!(m.read(edge - 4, 9).unwrap(), m.read(edge - 4, 1).unwrap().into_iter().chain(m.read(edge - 3, 8).unwrap()).collect::<Vec<_>>());
        m.write(edge - 1, &[1, 2, 3]).unwrap();
        assert_eq!(m.read(edge - 1, 3).unwrap(), vec![1, 2, 3]);
        m.copy(edge - 10, edge - 1, 3).unwrap();
        assert_eq!(m.read(edge - 10, 3).unwrap(), vec![1, 2, 3]);
        m.copy_n::<3>(edge - 20, edge - 1).unwrap();
        assert_eq!(m.read(edge - 20, 3).unwrap(), vec![1, 2, 3]);
        m.copy_n::<2>(edge - 1, edge - 20).unwrap();
        assert_eq!(m.read(edge - 1, 3).unwrap(), vec![1, 2, 3]);
        m.copy_n::<2>(edge - 19, edge - 20).unwrap();
        assert_eq!(m.read(edge - 20, 3).unwrap(), vec![1, 1, 2]);
        assert_eq!(m.copy_n::<2>(edge + PAGE - 1, 0), Err(Trap::OutOfBounds));
        assert_eq!(m.copy_n::<2>(0, u64::MAX), Err(Trap::OutOfBounds));
        assert_eq!(m.l8(edge + PAGE - 1, 0), Ok(0));
        assert_eq!(m.l8(edge + PAGE, 0), Err(Trap::OutOfBounds));
    }

    #[test]
    fn memory64_grows_lazily_and_keeps_its_bytes() {
        // A memory with no maximum allocates up front to the reserve and
        // grows within it by moving the length; the bytes past the length
        // stay out of reach until then.
        let mut m = Memory::new(1, None, true).unwrap();
        m.s64(8, 0, 42).unwrap();
        assert_eq!(m.l64(65536, 0), Err(Trap::OutOfBounds));
        assert_eq!(m.grow(1 << 14), 1);
        assert_eq!(m.size(), (1 << 14) + 1);
        assert_eq!(m.l64(8, 0), Ok(42));
        assert_eq!(m.l64(((1 << 14) + 1) * 65536 - 8, 0), Ok(0));
        assert_eq!(m.l64(((1 << 14) + 1) * 65536 - 7, 0), Err(Trap::OutOfBounds));
        assert_eq!(m.grow(1 << 48), -1);
    }

    #[test]
    fn m64f_fixed_capacity_accesses_and_limits() {
        const CAP: usize = 3 * PAGE as usize;
        assert_eq!(Outer::new_flat_meta(1 << 48, None, CAP).err(), Some(Trap::MemoryAllocationOverflow));
        assert_eq!(Outer::new_flat_meta((1 << 20) + 1, None, CAP).err(), Some(Trap::MemoryLimit));
        assert_eq!(Outer::new_flat_meta(4, Some(3), CAP).err(), Some(Trap::MemoryTooLarge));
        assert_eq!(Outer::new_flat_meta(1, Some(4), CAP).err(), Some(Trap::AllocationFailed));

        let mut outer = Outer::new_flat_meta(1, Some(3), CAP).unwrap();
        let mut bytes = alloc64::<CAP>().unwrap();
        assert_eq!(outer.fast(), PAGE as usize);
        assert_eq!(outer.grow(2), 1);
        assert_eq!(outer.grow(1), -1);
        let len = outer.fast();
        assert_eq!(len, CAP);

        assert_eq!(m64f::s8::<CAP>(&mut bytes, len, (len - 1) as u64, 0, 0xAB), Ok(()));
        assert_eq!(m64f::l8::<CAP>(&bytes, len, (len - 1) as u64, 0), Ok(0xAB));
        assert_eq!(m64f::l8::<CAP>(&bytes, len, len as u64, 0), Err(Trap::OutOfBounds));
        assert_eq!(m64f::l8::<CAP>(&bytes, len, u64::MAX, 1), Err(Trap::OutOfBounds));

        assert_eq!(m64f::s64::<CAP>(&mut bytes, len, (len - 16) as u64, 8, 0x1122_3344_5566_7788), Ok(()));
        assert_eq!(m64f::l64::<CAP>(&bytes, len, (len - 16) as u64, 8), Ok(0x1122_3344_5566_7788));
        assert_eq!(m64f::l64::<CAP>(&bytes, len, (len - 15) as u64, 8), Err(Trap::OutOfBounds));
        assert_eq!(m64f::s128::<CAP>(&mut bytes, len, 0, 16, 0xDEAD_BEEF_CAFE_BABE_0123_4567_89AB_CDEF), Ok(()));
        assert_eq!(m64f::l128::<CAP>(&bytes, len, 0, 16), Ok(0xDEAD_BEEF_CAFE_BABE_0123_4567_89AB_CDEF));
        assert_eq!(m64f::l128::<CAP>(&bytes, len, (len - 15) as u64, 0), Err(Trap::OutOfBounds));
        assert_eq!(m64f::l32::<CAP>(&bytes, len + 1, 0, 0), Err(Trap::OutOfBounds));

        // Overlapping copy_n both ways and fill_n.
        m64f::fill_n::<CAP, 6>(&mut bytes, len, 64, 0x5A).unwrap();
        assert_eq!(&bytes[63..71], &[0, 0x5A, 0x5A, 0x5A, 0x5A, 0x5A, 0x5A, 0]);
        for i in 0..8u8 {
            bytes[100 + i as usize] = i + 1;
        }
        m64f::copy_n::<CAP, 6>(&mut bytes, len, 102, 100).unwrap();
        assert_eq!(&bytes[100..108], &[1, 2, 1, 2, 3, 4, 5, 6]);
        m64f::copy_n::<CAP, 6>(&mut bytes, len, 100, 102).unwrap();
        assert_eq!(&bytes[100..108], &[1, 2, 3, 4, 5, 6, 5, 6]);
        assert_eq!(m64f::copy_n::<CAP, 8>(&mut bytes, len, (len - 7) as u64, 0), Err(Trap::OutOfBounds));
        assert_eq!(m64f::fill_n::<CAP, 8>(&mut bytes, len, (len - 7) as u64, 1), Err(Trap::OutOfBounds));

        let mut host = Mem64FixedHost { base: &mut bytes, outer };
        assert!(matches!(host.slice(100, 6).unwrap(), std::borrow::Cow::Borrowed(b) if b == [1, 2, 3, 4, 5, 6]));
        host.write(200, b"hello").unwrap();
        assert_eq!(host.read(200, 5).unwrap(), b"hello");
    }

    #[test]
    fn m32_hybrid_accesses_and_mem32_host() {
        const CAP: usize = 2 * PAGE as usize;
        let mut meta = Mem32Meta::new(1, Some(2)).unwrap();
        let mut bytes = alloc32::<CAP>().unwrap();
        let len = meta.len;

        // Zero-offset (>= 4 bytes uses first_chunk) and non-zero offset / <= 2 bytes (uses last_chunk).
        assert_eq!(m32::s32::<CAP>(&mut bytes, len, (len - 4) as u32, 0, 0x1234_5678), Ok(()));
        assert_eq!(m32::l32::<CAP>(&bytes, len, (len - 4) as u32, 0), Ok(0x1234_5678));
        assert_eq!(m32::l32::<CAP>(&bytes, len, (len - 3) as u32, 0), Err(Trap::OutOfBounds));
        assert_eq!(m32::s16::<CAP>(&mut bytes, len, (len - 2) as u32, 0, 0xABCD), Ok(()));
        assert_eq!(m32::l16::<CAP>(&bytes, len, (len - 2) as u32, 0), Ok(0xABCD));
        assert_eq!(m32::l16::<CAP>(&bytes, len, (len - 1) as u32, 0), Err(Trap::OutOfBounds));
        assert_eq!(m32::s64::<CAP>(&mut bytes, len, (len - 16) as u32, 8, 0x0102_0304_0506_0708), Ok(()));
        assert_eq!(m32::l64::<CAP>(&bytes, len, (len - 16) as u32, 8), Ok(0x0102_0304_0506_0708));
        assert_eq!(m32::l64::<CAP>(&bytes, len, u32::MAX, 16), Err(Trap::OutOfBounds));
        assert_eq!(m32::s8::<CAP>(&mut bytes, len, (len - 1) as u32, 0, 0xFE), Ok(()));
        assert_eq!(m32::l8::<CAP>(&bytes, len, (len - 1) as u32, 0), Ok(0xFE));
        assert_eq!(m32::l8::<CAP>(&bytes, len, len as u32, 0), Err(Trap::OutOfBounds));

        assert_eq!(meta.grow(1), 1);
        assert_eq!(meta.grow(1), -1);
        let len2 = meta.len;
        m32::fill_n::<CAP, 5>(&mut bytes, len2, (len2 - 5) as u32, 0x77).unwrap();
        assert_eq!(m32::fill_n::<CAP, 5>(&mut bytes, len2, (len2 - 4) as u32, 0x77), Err(Trap::OutOfBounds));
        m32::copy_n::<CAP, 5>(&mut bytes, len2, 32, (len2 - 5) as u32).unwrap();
        let mut host = Mem32Host { bytes: &mut bytes, meta };
        assert!(matches!(host.slice(32, 5).unwrap(), std::borrow::Cow::Borrowed(b) if b == [0x77; 5]));
        host.write(40, b"wtime").unwrap();
        assert_eq!(host.read(40, 5).unwrap(), b"wtime");
    }
}
