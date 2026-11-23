//! Backing storage for an unshared linear memory.
//!
//! Memory is a table of pages of up to 64 KiB. A page is allocated the first
//! time a nonzero byte is written to it; until then it reads as zeros. Growing
//! the memory only extends the table, and filling with zero skips pages that
//! were never written. So a memory's cost follows the bytes the guest touches,
//! not its size: a 4 GiB initial memory or a 64-bit memory grown to many GiB
//! needs no large allocation and commits nothing it does not use. This is safe
//! Rust with no reliance on how the operating system backs large allocations.
//!
//! The last page holds only the bytes up to the memory's length (rounded up
//! by at most a factor of two when it grows), so the storage allocated stays
//! within twice the memory's accounted size, even for tiny memories with
//! custom page sizes. The page table itself, 16 bytes per 64 KiB, is reserved
//! fallibly but not counted in memory budgets. Every page and page-table allocation is fallible: running
//! out of host memory there is reported as `AllocationFailed`, not an abort.
//! (Small bookkeeping allocations elsewhere in the runtime remain infallible.)
//!
//! Callers check bounds against `len` before every access, as for the former
//! contiguous backing; out-of-range offsets here panic rather than trap.

/// log2 of the page size used for storage; independent of the Wasm page size.
const PAGE_BITS: u32 = 16;
const PAGE: usize = 1 << PAGE_BITS;
const MASK: usize = PAGE - 1;

/// The host could not allocate storage for the memory.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AllocationFailed;

#[derive(Default)]
pub(crate) struct LinearBytes {
    pages: Vec<Option<Box<[u8]>>>,
    len: usize,
}

fn page_count(len: usize) -> usize {
    len.div_ceil(PAGE)
}

fn zeroed_page(size: usize) -> Result<Box<[u8]>, AllocationFailed> {
    let mut page = Vec::new();
    page.try_reserve_exact(size).map_err(|_| AllocationFailed)?;
    page.resize(size, 0);
    Ok(page.into_boxed_slice())
}

impl LinearBytes {
    /// A zeroed memory of `len` bytes. `None` when its page table cannot be
    /// allocated.
    pub(crate) fn zeroed(len: usize) -> Option<Self> {
        let mut memory = Self::default();
        memory.grow_to(len).then_some(memory)
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// The bytes page `index` holds at the current length. An allocated last
    /// page may hold more after growth; bytes past the length stay zero.
    fn page_size(&self, index: usize) -> usize {
        (self.len - index * PAGE).min(PAGE)
    }

    /// Grows the memory to `new_len` zero-extended bytes. Returns false without
    /// changing the memory when storage cannot be allocated.
    pub(crate) fn grow_to(&mut self, new_len: usize) -> bool {
        if new_len <= self.len {
            return true;
        }
        let pages = page_count(new_len);
        if self
            .pages
            .try_reserve(pages.saturating_sub(self.pages.len()))
            .is_err()
        {
            return false;
        }
        // A written last page that was shorter than a full page extends to
        // hold the new length (or a full page) before anything else changes.
        // It at least doubles, so repeated small grows copy it O(log) times.
        if let Some(last) = self.pages.len().checked_sub(1)
            && let Some(page) = &self.pages[last]
        {
            let size = (new_len - last * PAGE).min(PAGE);
            if page.len() < size {
                let Ok(mut extended) =
                    zeroed_page(size.max(page.len().saturating_mul(2)).min(PAGE))
                else {
                    return false;
                };
                extended[..page.len()].copy_from_slice(page);
                self.pages[last] = Some(extended);
            }
        }
        self.pages.resize(pages, None);
        self.len = new_len;
        true
    }

    /// The `N` bytes at `at` when the access is in bounds and within one
    /// page; `None` leaves the access to the general path.
    #[inline(always)]
    pub(crate) fn read_array<const N: usize>(&self, at: usize) -> Option<[u8; N]> {
        let end = at.checked_add(N)?;
        if end > self.len {
            return None;
        }
        let offset = at & MASK;
        match self.pages.get(at >> PAGE_BITS)? {
            Some(page) => page.get(offset..offset + N)?.try_into().ok(),
            None => (offset + N <= PAGE).then_some([0; N]),
        }
    }
    /// Writes `data` at `at` when the access is in bounds, within one page
    /// and the page is allocated; false leaves it to the general path.
    #[inline(always)]
    pub(crate) fn write_array<const N: usize>(&mut self, at: usize, data: [u8; N]) -> bool {
        let Some(end) = at.checked_add(N) else {
            return false;
        };
        if end > self.len {
            return false;
        }
        let offset = at & MASK;
        let Some(Some(page)) = self.pages.get_mut(at >> PAGE_BITS) else {
            return false;
        };
        let Some(slot) = page.get_mut(offset..offset + N) else {
            return false;
        };
        slot.copy_from_slice(&data);
        true
    }
    /// Copies `out.len()` bytes at `at` into `out`.
    #[inline]
    pub(crate) fn read(&self, at: usize, out: &mut [u8]) {
        // An empty access may sit at the very end of memory, past every page.
        if out.is_empty() {
            return;
        }
        let offset = at & MASK;
        if offset + out.len() <= PAGE {
            match &self.pages[at >> PAGE_BITS] {
                Some(page) => out.copy_from_slice(&page[offset..offset + out.len()]),
                None => out.fill(0),
            }
        } else {
            self.read_spanning(at, out)
        }
    }

    fn read_spanning(&self, mut at: usize, mut out: &mut [u8]) {
        while !out.is_empty() {
            let offset = at & MASK;
            let amount = out.len().min(PAGE - offset);
            let (head, rest) = out.split_at_mut(amount);
            match &self.pages[at >> PAGE_BITS] {
                Some(page) => head.copy_from_slice(&page[offset..offset + amount]),
                None => head.fill(0),
            }
            out = rest;
            at += amount;
        }
    }

    fn page_mut(&mut self, index: usize) -> Result<&mut [u8], AllocationFailed> {
        if self.pages[index].is_none() {
            self.pages[index] = Some(zeroed_page(self.page_size(index))?);
        }
        Ok(self.pages[index].as_deref_mut().expect("allocated above"))
    }

    /// Allocates every page that a write of `data` at `at` will store
    /// nonzero bytes into, so that the write itself cannot fail half-way.
    fn allocate_for(&mut self, at: usize, data: &[u8]) -> Result<(), AllocationFailed> {
        let first = at >> PAGE_BITS;
        let mut at = at;
        let mut data = data;
        while !data.is_empty() {
            let amount = data.len().min(PAGE - (at & MASK));
            let index = at >> PAGE_BITS;
            if self.pages[index].is_none()
                && data[..amount].iter().any(|&byte| byte != 0)
                && self.page_mut(index).is_err()
            {
                self.release(first, index);
                return Err(AllocationFailed);
            }
            data = &data[amount..];
            at += amount;
        }
        Ok(())
    }

    /// After a failed call allocated pages `first..end` itself (all still
    /// zero), releases the ones that are entirely zero again, so that a failed
    /// write leaves no committed storage behind. Pages written earlier hold
    /// nonzero bytes and are kept.
    fn release(&mut self, first: usize, end: usize) {
        for page in &mut self.pages[first..end] {
            if page
                .as_deref()
                .is_some_and(|bytes| bytes.iter().all(|&byte| byte == 0))
            {
                *page = None;
            }
        }
    }

    /// Copies `data` to `at`. Either every byte is written or, when a page
    /// cannot be allocated, none is. Zero bytes for a page that was never
    /// written are not stored, so the page stays unallocated.
    #[inline]
    pub(crate) fn write(&mut self, at: usize, data: &[u8]) -> Result<(), AllocationFailed> {
        if data.is_empty() {
            return Ok(());
        }
        let offset = at & MASK;
        if offset + data.len() <= PAGE {
            let index = at >> PAGE_BITS;
            if self.pages[index].is_none() && data.iter().all(|&byte| byte == 0) {
                return Ok(());
            }
            self.page_mut(index)?[offset..offset + data.len()].copy_from_slice(data);
            Ok(())
        } else {
            self.allocate_for(at, data)?;
            self.write_allocated(at, data);
            Ok(())
        }
    }

    /// Stores `data` at `at` into pages that `allocate_for` prepared: pieces
    /// for unallocated pages are all zero and are skipped.
    fn write_allocated(&mut self, mut at: usize, mut data: &[u8]) {
        while !data.is_empty() {
            let offset = at & MASK;
            let amount = data.len().min(PAGE - offset);
            if let Some(page) = self.pages[at >> PAGE_BITS].as_deref_mut() {
                page[offset..offset + amount].copy_from_slice(&data[..amount]);
            }
            data = &data[amount..];
            at += amount;
        }
    }

    /// Sets `count` bytes at `at` to `byte`, allocating every page it needs
    /// before changing any. Zeroing a page that was never written leaves it
    /// unallocated.
    pub(crate) fn fill(
        &mut self,
        at: usize,
        count: usize,
        byte: u8,
    ) -> Result<(), AllocationFailed> {
        let pieces = |mut at: usize, mut count: usize| {
            std::iter::from_fn(move || {
                (count > 0).then(|| {
                    let offset = at & MASK;
                    let amount = count.min(PAGE - offset);
                    let piece = (at >> PAGE_BITS, offset, amount);
                    count -= amount;
                    at += amount;
                    piece
                })
            })
        };
        if byte != 0 {
            for (index, _, _) in pieces(at, count) {
                if self.pages[index].is_none() && self.page_mut(index).is_err() {
                    self.release(at >> PAGE_BITS, index);
                    return Err(AllocationFailed);
                }
            }
        }
        for (index, offset, amount) in pieces(at, count) {
            if let Some(page) = self.pages[index].as_deref_mut() {
                page[offset..offset + amount].fill(byte);
            }
        }
        Ok(())
    }

    /// Copies `count` bytes from `source` to `destination`, as `memmove`, all
    /// or nothing. Zero bytes copied onto never-written pages are not stored.
    /// `scratch` is reused between calls to avoid allocating.
    pub(crate) fn copy_within(
        &mut self,
        source: usize,
        destination: usize,
        count: usize,
        scratch: &mut Vec<u8>,
    ) -> Result<(), AllocationFailed> {
        scratch
            .try_reserve(count.saturating_sub(scratch.len()))
            .map_err(|_| AllocationFailed)?;
        scratch.resize(count, 0);
        self.read(source, scratch);
        self.write(destination, scratch)
    }

    /// The written pages, as their byte offsets and the contents within the
    /// memory's length (a grown last page may hold more).
    pub(crate) fn written_pages(&self) -> impl Iterator<Item = (usize, &[u8])> {
        self.pages.iter().enumerate().filter_map(|(index, page)| {
            page.as_deref().map(|bytes| {
                (
                    index * PAGE,
                    &bytes[..bytes.len().min(self.len - index * PAGE)],
                )
            })
        })
    }

    /// Releases the storage; the memory becomes empty.
    pub(crate) fn clear(&mut self) {
        self.pages = Vec::new();
        self.len = 0;
    }

    /// Test support for compact-executor memory growth tests:
    /// zero-extends, or truncates the visible bytes and clears what was cut.
    #[cfg(all(test, feature = "compact-executor"))]
    pub(crate) fn resize(&mut self, new_len: usize, value: u8) {
        assert_eq!(value, 0, "linear memory only grows with zeros");
        if new_len < self.len {
            let mut kept = Self::zeroed(new_len).expect("test memory allocation");
            let mut bytes = vec![0; new_len];
            self.read(0, &mut bytes);
            kept.write(0, &bytes).expect("test memory allocation");
            *self = kept;
        } else {
            assert!(self.grow_to(new_len), "test memory allocation");
        }
    }

    /// Pages that hold storage; for tests of sparseness.
    #[cfg(test)]
    fn allocated_pages(&self) -> usize {
        self.pages.iter().filter(|page| page.is_some()).count()
    }

    /// Bytes of page storage; for tests of accounting.
    #[cfg(test)]
    fn allocated_bytes(&self) -> usize {
        self.pages.iter().flatten().map(|page| page.len()).sum()
    }
}

#[cfg(test)]
impl std::ops::Index<usize> for LinearBytes {
    type Output = u8;
    fn index(&self, at: usize) -> &u8 {
        assert!(at < self.len, "linear memory index out of range");
        match &self.pages[at >> PAGE_BITS] {
            Some(page) => &page[at & MASK],
            None => &0,
        }
    }
}

#[cfg(test)]
impl std::ops::IndexMut<usize> for LinearBytes {
    fn index_mut(&mut self, at: usize) -> &mut u8 {
        assert!(at < self.len, "linear memory index out of range");
        &mut self
            .page_mut(at >> PAGE_BITS)
            .expect("test memory allocation")[at & MASK]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(memory: &LinearBytes, at: usize, count: usize) -> Vec<u8> {
        let mut out = vec![0xaa; count];
        memory.read(at, &mut out);
        out
    }

    #[test]
    fn unwritten_memory_reads_zero_and_allocates_nothing() {
        let memory = LinearBytes::zeroed(4 << 30).unwrap();
        assert_eq!(memory.len(), 4 << 30);
        assert_eq!(bytes(&memory, (4 << 30) - 8, 8), [0; 8]);
        assert_eq!(memory.allocated_pages(), 0);
    }

    #[test]
    fn writes_and_reads_within_and_across_pages() {
        let mut memory = LinearBytes::zeroed(3 * PAGE).unwrap();
        memory.write(5, &[1, 2, 3]).unwrap();
        assert_eq!(bytes(&memory, 4, 5), [0, 1, 2, 3, 0]);
        memory.write(PAGE - 2, &[9, 8, 7, 6]).unwrap();
        assert_eq!(bytes(&memory, PAGE - 3, 6), [0, 9, 8, 7, 6, 0]);
        assert_eq!(memory.allocated_pages(), 2);
        let pattern: Vec<u8> = (0..2 * PAGE + 10).map(|i| (i % 251) as u8 + 1).collect();
        memory.write(PAGE / 2, &pattern).unwrap();
        assert_eq!(bytes(&memory, PAGE / 2, pattern.len()), pattern);
        assert_eq!(memory.allocated_pages(), 3);
    }

    #[test]
    fn growth_keeps_contents_and_zero_extends() {
        let mut memory = LinearBytes::zeroed(10).unwrap();
        memory.write(0, &[1; 10]).unwrap();
        assert!(memory.grow_to(PAGE + 10));
        assert_eq!(bytes(&memory, 0, 12), [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0]);
        assert_eq!(bytes(&memory, PAGE, 10), [0; 10]);
        assert!(memory.grow_to(5));
        assert_eq!(memory.len(), PAGE + 10);
    }

    #[test]
    fn zero_fill_skips_unwritten_pages_and_clears_written_ones() {
        let mut memory = LinearBytes::zeroed(4 * PAGE).unwrap();
        memory.fill(0, 4 * PAGE, 0).unwrap();
        assert_eq!(memory.allocated_pages(), 0);
        memory.write(PAGE + 1, &[5; 4]).unwrap();
        memory.fill(PAGE, 2 * PAGE, 0).unwrap();
        assert_eq!(bytes(&memory, PAGE, 8), [0; 8]);
        memory.fill(3, PAGE, 7).unwrap();
        assert_eq!(bytes(&memory, 0, 5), [0, 0, 0, 7, 7]);
        assert_eq!(bytes(&memory, PAGE + 1, 3), [7, 7, 0]);
    }

    #[test]
    fn copy_within_handles_overlap_in_both_directions() {
        let mut memory = LinearBytes::zeroed(2 * PAGE).unwrap();
        let mut scratch = Vec::new();
        let pattern: Vec<u8> = (0..300).map(|i| i as u8).collect();
        memory.write(PAGE - 100, &pattern).unwrap();
        memory
            .copy_within(PAGE - 100, PAGE - 99, 300, &mut scratch)
            .unwrap();
        assert_eq!(bytes(&memory, PAGE - 99, 300), pattern);
        memory.write(PAGE - 100, &pattern).unwrap();
        memory
            .copy_within(PAGE - 99, PAGE - 100, 299, &mut scratch)
            .unwrap();
        assert_eq!(bytes(&memory, PAGE - 100, 299), pattern[1..]);
    }

    #[test]
    fn empty_accesses_at_the_end_touch_no_page() {
        let mut empty = LinearBytes::zeroed(0).unwrap();
        empty.read(0, &mut []);
        empty.write(0, &[]).unwrap();
        empty.fill(0, 0, 9).unwrap();
        empty.copy_within(0, 0, 0, &mut Vec::new()).unwrap();
        let mut memory = LinearBytes::zeroed(PAGE).unwrap();
        memory.read(PAGE, &mut []);
        memory.write(PAGE, &[]).unwrap();
        memory.fill(PAGE, 0, 9).unwrap();
        memory.copy_within(PAGE, 0, 0, &mut Vec::new()).unwrap();
        assert_eq!(memory.allocated_pages(), 0);
    }

    #[test]
    fn storage_never_exceeds_the_memory_length() {
        // A one-byte memory, as with a custom page size of one byte.
        let mut memory = LinearBytes::zeroed(1).unwrap();
        memory.write(0, &[7]).unwrap();
        assert_eq!(memory.allocated_bytes(), 1);
        // Growing extends the written partial page and keeps its contents.
        assert!(memory.grow_to(10));
        assert_eq!(memory.allocated_bytes(), 10);
        assert_eq!(bytes(&memory, 0, 10), [7, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        memory.write(9, &[8]).unwrap();
        assert!(memory.grow_to(PAGE + 3));
        assert_eq!(memory.allocated_bytes(), PAGE);
        memory.write(PAGE + 2, &[9]).unwrap();
        assert_eq!(memory.allocated_bytes(), PAGE + 3);
        assert_eq!(bytes(&memory, 8, 3), [0, 8, 0]);
        assert_eq!(bytes(&memory, PAGE, 3), [0, 0, 9]);
        let pages: Vec<(usize, usize)> = memory
            .written_pages()
            .map(|(at, page)| (at, page.len()))
            .collect();
        assert_eq!(pages, [(0, PAGE), (PAGE, 3)]);
    }

    #[test]
    fn growing_by_one_byte_reallocates_the_last_page_logarithmically() {
        let mut memory = LinearBytes::zeroed(1).unwrap();
        memory.write(0, &[1]).unwrap();
        let mut reallocations = 0;
        let mut previous = memory.allocated_bytes();
        for len in 2..=PAGE {
            assert!(memory.grow_to(len));
            memory.write(len - 1, &[(len % 251) as u8]).unwrap();
            let allocated = memory.allocated_bytes();
            assert!(
                allocated >= len && allocated <= 2 * len,
                "len={len} allocated={allocated}"
            );
            if allocated != previous {
                reallocations += 1;
                previous = allocated;
            }
        }
        assert!(reallocations <= 17, "{reallocations} reallocations");
        assert_eq!(bytes(&memory, 0, 1), [1]);
        assert_eq!(bytes(&memory, PAGE - 1, 1), [(PAGE % 251) as u8]);
        // The bytes beyond the length in a doubled page read as zero once visible.
        let mut memory = LinearBytes::zeroed(3).unwrap();
        memory.write(0, &[5, 5, 5]).unwrap();
        assert!(memory.grow_to(4));
        assert!(memory.grow_to(6));
        assert_eq!(bytes(&memory, 0, 6), [5, 5, 5, 0, 0, 0]);
    }

    #[test]
    fn written_pages_stop_at_the_length_after_growth() {
        let mut memory = LinearBytes::zeroed(4).unwrap();
        memory.write(0, &[1, 2, 3, 4]).unwrap();
        assert!(memory.grow_to(5));
        let pages: Vec<(usize, Vec<u8>)> = memory
            .written_pages()
            .map(|(at, page)| (at, page.to_vec()))
            .collect();
        assert_eq!(pages, [(0, vec![1, 2, 3, 4, 0])]);
    }

    #[test]
    fn zero_writes_and_copies_keep_unwritten_pages_unallocated() {
        let mut memory = LinearBytes::zeroed(4 * PAGE).unwrap();
        let mut scratch = Vec::new();
        memory.write(PAGE / 2, &vec![0; 2 * PAGE]).unwrap();
        memory
            .copy_within(0, 2 * PAGE, 2 * PAGE, &mut scratch)
            .unwrap();
        assert_eq!(memory.allocated_pages(), 0);
        memory.write(10, &[3]).unwrap();
        memory.copy_within(0, 2 * PAGE, PAGE, &mut scratch).unwrap();
        assert_eq!(memory.allocated_pages(), 2);
        assert_eq!(bytes(&memory, 2 * PAGE + 10, 1), [3]);
        // Zeros over a written page are stored.
        memory.write(2 * PAGE + 10, &[0]).unwrap();
        assert_eq!(bytes(&memory, 2 * PAGE + 10, 1), [0]);
    }

    #[test]
    fn impossible_growth_fails_without_change() {
        let mut memory = LinearBytes::zeroed(4).unwrap();
        assert!(!memory.grow_to(usize::MAX));
        assert_eq!(memory.len(), 4);
        assert_eq!(bytes(&memory, 0, 4), [0; 4]);
    }
}
