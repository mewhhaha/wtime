//! Backing storage for an unshared linear memory.
//!
//! Memory is a table of fixed-size pages. A page is allocated the first time a
//! nonzero byte is written to it; until then it reads as zeros. Growing the
//! memory only extends the table, and filling with zero skips pages that were
//! never written. So a memory's cost follows the bytes the guest touches, not
//! its size: a 4 GiB initial memory or a 64-bit memory grown to many GiB needs
//! no large allocation and commits nothing it does not use. This is safe Rust
//! with no reliance on how the operating system backs large allocations.
//!
//! Callers check bounds against `len` before every access, as for the former
//! contiguous backing; out-of-range offsets here panic rather than trap.

/// log2 of the page size used for storage; independent of the Wasm page size.
const PAGE_BITS: u32 = 16;
const PAGE: usize = 1 << PAGE_BITS;
const MASK: usize = PAGE - 1;

#[derive(Default)]
pub(crate) struct LinearBytes {
    pages: Vec<Option<Box<[u8]>>>,
    len: usize,
}

fn page_count(len: usize) -> usize {
    len.div_ceil(PAGE)
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

    /// Grows the memory to `new_len` zero-extended bytes. Returns false without
    /// changing the memory when the page table cannot be allocated.
    pub(crate) fn grow_to(&mut self, new_len: usize) -> bool {
        if new_len <= self.len {
            return true;
        }
        let pages = page_count(new_len);
        if self.pages.try_reserve(pages.saturating_sub(self.pages.len())).is_err() {
            return false;
        }
        self.pages.resize(pages, None);
        self.len = new_len;
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

    fn page_mut(&mut self, index: usize) -> &mut [u8] {
        self.pages[index].get_or_insert_with(|| vec![0u8; PAGE].into_boxed_slice())
    }

    /// Copies `data` to `at`.
    #[inline]
    pub(crate) fn write(&mut self, at: usize, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let offset = at & MASK;
        if offset + data.len() <= PAGE {
            self.page_mut(at >> PAGE_BITS)[offset..offset + data.len()].copy_from_slice(data);
        } else {
            self.write_spanning(at, data)
        }
    }

    fn write_spanning(&mut self, mut at: usize, mut data: &[u8]) {
        while !data.is_empty() {
            let offset = at & MASK;
            let amount = data.len().min(PAGE - offset);
            self.page_mut(at >> PAGE_BITS)[offset..offset + amount].copy_from_slice(&data[..amount]);
            data = &data[amount..];
            at += amount;
        }
    }

    /// Sets `count` bytes at `at` to `byte`. Zeroing a page that was never
    /// written leaves it unallocated.
    pub(crate) fn fill(&mut self, mut at: usize, mut count: usize, byte: u8) {
        while count > 0 {
            let offset = at & MASK;
            let amount = count.min(PAGE - offset);
            let index = at >> PAGE_BITS;
            if byte != 0 || self.pages[index].is_some() {
                self.page_mut(index)[offset..offset + amount].fill(byte);
            }
            count -= amount;
            at += amount;
        }
    }

    /// Copies `count` bytes from `source` to `destination`, as `memmove`.
    /// `scratch` is reused between calls to avoid allocating.
    pub(crate) fn copy_within(&mut self, source: usize, destination: usize, count: usize, scratch: &mut Vec<u8>) {
        scratch.resize(count, 0);
        self.read(source, scratch);
        self.write(destination, scratch);
    }

    /// The whole memory as one contiguous vector.
    pub(crate) fn to_vec(&self) -> Vec<u8> {
        let mut bytes = vec![0u8; self.len];
        self.read(0, &mut bytes);
        bytes
    }

    /// Releases the storage; the memory becomes empty.
    pub(crate) fn clear(&mut self) {
        self.pages = Vec::new();
        self.len = 0;
    }

    /// Test support for code written against the former `Vec<u8>` backing
    /// (used by tests of some feature configurations only):
    /// zero-extends, or truncates the visible bytes and clears what was cut.
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn resize(&mut self, new_len: usize, value: u8) {
        assert_eq!(value, 0, "linear memory only grows with zeros");
        if new_len < self.len {
            self.fill(new_len, self.len - new_len, 0);
            self.len = new_len;
            self.pages.truncate(page_count(new_len));
        } else {
            assert!(self.grow_to(new_len), "test memory allocation");
        }
    }

    /// Pages that hold storage; for tests of sparseness.
    #[cfg(test)]
    fn allocated_pages(&self) -> usize {
        self.pages.iter().filter(|page| page.is_some()).count()
    }
}

/// Index-style access for tests written against the contiguous backing.
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
        &mut self.page_mut(at >> PAGE_BITS)[at & MASK]
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
        memory.write(5, &[1, 2, 3]);
        assert_eq!(bytes(&memory, 4, 5), [0, 1, 2, 3, 0]);
        memory.write(PAGE - 2, &[9, 8, 7, 6]);
        assert_eq!(bytes(&memory, PAGE - 3, 6), [0, 9, 8, 7, 6, 0]);
        assert_eq!(memory.allocated_pages(), 2);
        let pattern: Vec<u8> = (0..2 * PAGE + 10).map(|i| (i % 251) as u8 + 1).collect();
        memory.write(PAGE / 2, &pattern);
        assert_eq!(bytes(&memory, PAGE / 2, pattern.len()), pattern);
        assert_eq!(memory.allocated_pages(), 3);
    }

    #[test]
    fn growth_keeps_contents_and_zero_extends() {
        let mut memory = LinearBytes::zeroed(10).unwrap();
        memory.write(0, &[1; 10]);
        assert!(memory.grow_to(PAGE + 10));
        assert_eq!(bytes(&memory, 0, 12), [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0]);
        assert_eq!(bytes(&memory, PAGE, 10), [0; 10]);
        assert!(memory.grow_to(5));
        assert_eq!(memory.len(), PAGE + 10);
    }

    #[test]
    fn zero_fill_skips_unwritten_pages_and_clears_written_ones() {
        let mut memory = LinearBytes::zeroed(4 * PAGE).unwrap();
        memory.fill(0, 4 * PAGE, 0);
        assert_eq!(memory.allocated_pages(), 0);
        memory.write(PAGE + 1, &[5; 4]);
        memory.fill(PAGE, 2 * PAGE, 0);
        assert_eq!(bytes(&memory, PAGE, 8), [0; 8]);
        memory.fill(3, PAGE, 7);
        assert_eq!(bytes(&memory, 0, 5), [0, 0, 0, 7, 7]);
        assert_eq!(bytes(&memory, PAGE + 1, 3), [7, 7, 0]);
    }

    #[test]
    fn copy_within_handles_overlap_in_both_directions() {
        let mut memory = LinearBytes::zeroed(2 * PAGE).unwrap();
        let mut scratch = Vec::new();
        let pattern: Vec<u8> = (0..300).map(|i| i as u8).collect();
        memory.write(PAGE - 100, &pattern);
        memory.copy_within(PAGE - 100, PAGE - 99, 300, &mut scratch);
        assert_eq!(bytes(&memory, PAGE - 99, 300), pattern);
        memory.write(PAGE - 100, &pattern);
        memory.copy_within(PAGE - 99, PAGE - 100, 299, &mut scratch);
        assert_eq!(bytes(&memory, PAGE - 100, 299), pattern[1..]);
    }

    #[test]
    fn empty_accesses_at_the_end_touch_no_page() {
        let mut empty = LinearBytes::zeroed(0).unwrap();
        empty.read(0, &mut []);
        empty.write(0, &[]);
        empty.fill(0, 0, 9);
        empty.copy_within(0, 0, 0, &mut Vec::new());
        let mut memory = LinearBytes::zeroed(PAGE).unwrap();
        memory.read(PAGE, &mut []);
        memory.write(PAGE, &[]);
        memory.fill(PAGE, 0, 9);
        memory.copy_within(PAGE, 0, 0, &mut Vec::new());
        assert_eq!(memory.allocated_pages(), 0);
    }

    #[test]
    fn impossible_growth_fails_without_change() {
        let mut memory = LinearBytes::zeroed(4).unwrap();
        assert!(!memory.grow_to(usize::MAX));
        assert_eq!(memory.len(), 4);
        assert_eq!(memory.to_vec(), [0; 4]);
    }
}
