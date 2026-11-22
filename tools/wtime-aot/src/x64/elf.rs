//! Pure-Rust Linux x86-64 ELF64 shared object (`ET_DYN`) writer.
//!
//! Produces a self-contained, zero-relocation, zero-`DT_NEEDED` shared library
//! (`library.so`) loadable directly by `dlopen` / `libloading` in sub-millisecond
//! time with a non-executable stack (`PT_GNU_STACK`).

const PAGE: usize = 4096;

#[inline]
fn align_up(n: usize, a: usize) -> usize {
    (n + (a - 1)) & !(a - 1)
}

fn elf_hash(name: &[u8]) -> u32 {
    let mut h: u32 = 0;
    for &b in name {
        h = (h << 4).wrapping_add(u32::from(b));
        let g = h & 0xf000_0000;
        if g != 0 {
            h ^= g >> 24;
        }
        h &= !g;
    }
    h
}

/// Metadata layout computed BEFORE finalizing `.text` relocations, because
/// `text_vaddr` depends on the exact size of `.dynsym` + `.hash` + `.dynstr` + `.rodata`.
pub struct ElfLayout {
    pub rodata_vaddr: usize,
    pub text_vaddr: usize,
    pub dyn_vaddr: usize,
    dynsym_off: usize,
    hash_off: usize,
    dynstr_off: usize,
    dynstr: Vec<u8>,
    name_offs: Vec<u32>,
    nbucket: usize,
}

impl ElfLayout {
    pub fn plan(export_names: &[&str], rodata_len: usize) -> Self {
        let nsyms = 1 + export_names.len();
        let nbucket = export_names.len().max(1);

        let mut dynstr = vec![0u8];
        let mut name_offs = Vec::with_capacity(export_names.len());
        for name in export_names {
            name_offs.push(dynstr.len() as u32);
            dynstr.extend_from_slice(name.as_bytes());
            dynstr.push(0);
        }

        // 64-byte Ehdr + 5 * 56-byte Phdrs = 344 bytes.
        let mut cur = 64 + 5 * 56;
        cur = align_up(cur, 8);
        let dynsym_off = cur;
        cur += nsyms * 24;

        cur = align_up(cur, 4);
        let hash_off = cur;
        cur += (2 + nbucket + nsyms) * 4;

        let dynstr_off = cur;
        cur += dynstr.len();

        cur = align_up(cur, 16);
        let rodata_vaddr = cur;
        let seg0_end = rodata_vaddr + rodata_len;
        let text_vaddr = align_up(seg0_end, PAGE);

        Self {
            rodata_vaddr,
            text_vaddr,
            dyn_vaddr: 0, // filled once text_len is known
            dynsym_off,
            hash_off,
            dynstr_off,
            dynstr,
            name_offs,
            nbucket,
        }
    }

    /// Emits the complete ELF64 `.so` image.
    /// `exports` must match `export_names` order from `plan`, giving `(text_offset, size)`.
    pub fn emit(mut self, rodata: &[u8], text: &[u8], exports: &[(&str, u64, u64)]) -> Vec<u8> {
        let seg0_end = self.rodata_vaddr + rodata.len();
        debug_assert_eq!(self.text_vaddr, align_up(seg0_end, PAGE));
        let seg1_end = self.text_vaddr + text.len();
        let dyn_vaddr = align_up(seg1_end, PAGE);
        self.dyn_vaddr = dyn_vaddr;

        // 6 dynamic entries * 16 bytes = 96 bytes.
        let dyn_size: usize = 6 * 16;
        let total_size = dyn_vaddr + dyn_size;
        let mut img = vec![0u8; total_size];

        // ---- ELF64 Header (64 bytes) ----
        img[0..4].copy_from_slice(b"\x7fELF");
        img[4] = 2; // ELFCLASS64
        img[5] = 1; // ELFDATA2LSB
        img[6] = 1; // EV_CURRENT
        img[7] = 0; // ELFOSABI_NONE
        put_u16(&mut img, 16, 3); // ET_DYN
        put_u16(&mut img, 18, 62); // EM_X86_64
        put_u32(&mut img, 20, 1); // EV_CURRENT
        put_u64(&mut img, 24, 0); // e_entry
        put_u64(&mut img, 32, 64); // e_phoff
        put_u64(&mut img, 40, 0); // e_shoff
        put_u32(&mut img, 48, 0); // e_flags
        put_u16(&mut img, 52, 64); // e_ehsize
        put_u16(&mut img, 54, 56); // e_phentsize
        put_u16(&mut img, 56, 5); // e_phnum
        put_u16(&mut img, 58, 0); // e_shentsize
        put_u16(&mut img, 60, 0); // e_shnum
        put_u16(&mut img, 62, 0); // e_shstrndx

        // ---- Program Headers (5 * 56 bytes at offset 64) ----
        let ph = |img: &mut [u8], idx: usize, p_type: u32, p_flags: u32, off: usize, vaddr: usize, sz: usize, align: usize| {
            let base = 64 + idx * 56;
            put_u32(img, base, p_type);
            put_u32(img, base + 4, p_flags);
            put_u64(img, base + 8, off as u64);
            put_u64(img, base + 16, vaddr as u64);
            put_u64(img, base + 24, vaddr as u64);
            put_u64(img, base + 32, sz as u64);
            put_u64(img, base + 40, sz as u64);
            put_u64(img, base + 48, align as u64);
        };
        const PT_LOAD: u32 = 1;
        const PT_DYNAMIC: u32 = 2;
        const PT_GNU_STACK: u32 = 0x6474_e551;
        const PF_X: u32 = 1;
        const PF_W: u32 = 2;
        const PF_R: u32 = 4;

        ph(&mut img, 0, PT_LOAD, PF_R, 0, 0, seg0_end, PAGE);
        ph(&mut img, 1, PT_LOAD, PF_R | PF_X, self.text_vaddr, self.text_vaddr, text.len(), PAGE);
        ph(&mut img, 2, PT_LOAD, PF_R | PF_W, dyn_vaddr, dyn_vaddr, dyn_size, PAGE);
        ph(&mut img, 3, PT_DYNAMIC, PF_R | PF_W, dyn_vaddr, dyn_vaddr, dyn_size, 8);
        ph(&mut img, 4, PT_GNU_STACK, PF_R | PF_W, 0, 0, 0, 16);

        // ---- .dynsym ----
        let nsyms = 1 + exports.len();
        for (i, &(_, text_off, sym_size)) in exports.iter().enumerate() {
            let sym_idx = i + 1;
            let base = self.dynsym_off + sym_idx * 24;
            put_u32(&mut img, base, self.name_offs[i]); // st_name
            img[base + 4] = (1 << 4) | 2; // STB_GLOBAL | STT_FUNC
            img[base + 5] = 0; // STV_DEFAULT
            put_u16(&mut img, base + 6, 1); // non-zero shndx (defined)
            put_u64(&mut img, base + 8, self.text_vaddr as u64 + text_off);
            put_u64(&mut img, base + 16, sym_size);
        }

        // ---- .hash (SysV ELF hash table) ----
        let mut buckets = vec![0u32; self.nbucket];
        let mut chains = vec![0u32; nsyms];
        for (i, &(name, _, _)) in exports.iter().enumerate() {
            let sym_idx = (i + 1) as u32;
            let b = (elf_hash(name.as_bytes()) as usize) % self.nbucket;
            chains[sym_idx as usize] = buckets[b];
            buckets[b] = sym_idx;
        }
        put_u32(&mut img, self.hash_off, self.nbucket as u32);
        put_u32(&mut img, self.hash_off + 4, nsyms as u32);
        let mut hpos = self.hash_off + 8;
        for &b in &buckets {
            put_u32(&mut img, hpos, b);
            hpos += 4;
        }
        for &c in &chains {
            put_u32(&mut img, hpos, c);
            hpos += 4;
        }

        // ---- .dynstr & .rodata ----
        img[self.dynstr_off..self.dynstr_off + self.dynstr.len()].copy_from_slice(&self.dynstr);
        img[self.rodata_vaddr..self.rodata_vaddr + rodata.len()].copy_from_slice(rodata);

        // ---- .text ----
        img[self.text_vaddr..self.text_vaddr + text.len()].copy_from_slice(text);

        // ---- .dynamic ----
        let mut dpos = dyn_vaddr;
        let mut put_dyn = |tag: u64, val: u64| {
            put_u64(&mut img, dpos, tag);
            put_u64(&mut img, dpos + 8, val);
            dpos += 16;
        };
        put_dyn(4, self.hash_off as u64); // DT_HASH
        put_dyn(5, self.dynstr_off as u64); // DT_STRTAB
        put_dyn(6, self.dynsym_off as u64); // DT_SYMTAB
        put_dyn(10, self.dynstr.len() as u64); // DT_STRSZ
        put_dyn(11, 24); // DT_SYMENT
        put_dyn(0, 0); // DT_NULL

        img
    }
}

#[inline]
fn put_u16(buf: &mut [u8], off: usize, v: u16) {
    buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
}

#[inline]
fn put_u32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

#[inline]
fn put_u64(buf: &mut [u8], off: usize, v: u64) {
    buf[off..off + 8].copy_from_slice(&v.to_le_bytes());
}
