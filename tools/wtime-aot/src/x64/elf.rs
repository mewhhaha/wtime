//! Pure-Rust Linux x86-64 ELF64 shared object (`ET_DYN`) writer.
//!
//! Produces a self-contained, zero-relocation, zero-`DT_NEEDED` shared library
//! (`library.so`) loadable directly by `dlopen` / `libloading`, with a conforming
//! section-header table (`.dynsym`, `.hash`, `.dynstr`, `.rodata`, `.text`,
//! `.dynamic`, `.shstrtab`) and a non-executable stack (`PT_GNU_STACK`).

use std::collections::BTreeSet;

const PAGE: usize = 4096;

#[inline]
fn checked_align_up(n: usize, a: usize) -> Result<usize, String> {
    let mask = a - 1;
    n.checked_add(mask)
        .map(|v| v & !mask)
        .ok_or_else(|| "ELF layout size overflow".to_string())
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

/// Keyed export symbol placement within `.text`, verified by name against `ElfLayout::plan`.
#[derive(Clone, Copy, Debug)]
pub struct ExportDef<'a> {
    pub name: &'a str,
    pub text_off: u64,
    pub size: u64,
}

/// Metadata layout computed BEFORE finalizing `.text` relocations, because
/// `text_vaddr` depends on the exact size of `.dynsym` + `.hash` + `.dynstr` + `.rodata`.
///
/// Export names and `planned_rodata_len` are captured once in `plan` and validated
/// strictly by `emit`, preventing positional drift or sub-page size changes.
pub struct ElfLayout {
    pub rodata_vaddr: usize,
    pub text_vaddr: usize,
    pub dyn_vaddr: usize,
    planned_rodata_len: usize,
    export_names: Vec<String>,
    dynsym_off: usize,
    dynsym_len: usize,
    hash_off: usize,
    hash_len: usize,
    dynstr_off: usize,
    dynstr: Vec<u8>,
    name_offs: Vec<u32>,
    nbucket: usize,
}

impl ElfLayout {
    pub fn plan(export_names: &[&str], rodata_len: usize) -> Result<Self, String> {
        let mut seen = BTreeSet::new();
        for &name in export_names {
            if name.is_empty() || name.as_bytes().contains(&0) {
                return Err(format!("invalid ELF export name {name:?}"));
            }
            if !seen.insert(name) {
                return Err(format!("duplicate ELF export name {name:?}"));
            }
        }
        let nsyms = 1 + export_names.len();
        let nbucket = export_names.len().max(1);

        let mut dynstr = vec![0u8];
        let mut name_offs = Vec::with_capacity(export_names.len());
        let mut owned_names = Vec::with_capacity(export_names.len());
        for &name in export_names {
            let off = u32::try_from(dynstr.len()).map_err(|_| ".dynstr exceeds u32")?;
            name_offs.push(off);
            dynstr.extend_from_slice(name.as_bytes());
            dynstr.push(0);
            owned_names.push(name.to_owned());
        }

        // 64-byte Ehdr + 5 * 56-byte Phdrs = 344 bytes.
        let mut cur = 64 + 5 * 56;
        cur = checked_align_up(cur, 8)?;
        let dynsym_off = cur;
        let dynsym_len = nsyms.checked_mul(24).ok_or(".dynsym size overflow")?;
        cur = cur.checked_add(dynsym_len).ok_or("ELF header overflow")?;

        cur = checked_align_up(cur, 4)?;
        let hash_off = cur;
        let hash_len = nbucket
            .checked_add(nsyms)
            .and_then(|v| v.checked_add(2))
            .and_then(|v| v.checked_mul(4))
            .ok_or(".hash size overflow")?;
        cur = cur.checked_add(hash_len).ok_or("ELF header overflow")?;

        let dynstr_off = cur;
        cur = cur.checked_add(dynstr.len()).ok_or("ELF header overflow")?;

        cur = checked_align_up(cur, 16)?;
        let rodata_vaddr = cur;
        let seg0_end = rodata_vaddr
            .checked_add(rodata_len)
            .ok_or("rodata size overflow")?;
        let text_vaddr = checked_align_up(seg0_end, PAGE)?;
        if text_vaddr >= i32::MAX as usize {
            return Err(format!(
                "ELF rodata + headers ({text_vaddr} B) exceed rel32 addressable 2 GiB range"
            ));
        }

        Ok(Self {
            rodata_vaddr,
            text_vaddr,
            dyn_vaddr: 0,
            planned_rodata_len: rodata_len,
            export_names: owned_names,
            dynsym_off,
            dynsym_len,
            hash_off,
            hash_len,
            dynstr_off,
            dynstr,
            name_offs,
            nbucket,
        })
    }

    /// Emits the complete ELF64 `.so` image with conforming section headers.
    /// Every `ExportDef` must match `plan`'s export names and have a non-empty extent within `.text`.
    pub fn emit(
        mut self,
        rodata: &[u8],
        text: &[u8],
        exports: &[ExportDef<'_>],
    ) -> Result<Vec<u8>, String> {
        if rodata.len() != self.planned_rodata_len {
            return Err(format!(
                "ELF rodata length changed after plan: planned {} vs emitted {}",
                self.planned_rodata_len,
                rodata.len()
            ));
        }
        if text.is_empty() {
            return Err("ELF .text section must be non-empty".into());
        }
        if exports.len() != self.export_names.len() {
            return Err(format!(
                "ELF export count mismatch: planned {} vs emitted {}",
                self.export_names.len(),
                exports.len()
            ));
        }
        for (expected, def) in self.export_names.iter().zip(exports) {
            if expected != def.name {
                return Err(format!(
                    "ELF export name/order mismatch: expected {expected:?}, got {:?}",
                    def.name
                ));
            }
            if def.size == 0 {
                return Err(format!("ELF export {:?} has empty extent", def.name));
            }
            let end = def
                .text_off
                .checked_add(def.size)
                .ok_or_else(|| format!("export {:?} overflow", def.name))?;
            if end > text.len() as u64 {
                return Err(format!(
                    "export {:?} extent ({end}) exceeds .text ({})",
                    def.name,
                    text.len()
                ));
            }
        }

        let seg0_end = self
            .rodata_vaddr
            .checked_add(rodata.len())
            .ok_or("rodata size overflow")?;
        let seg1_end = self
            .text_vaddr
            .checked_add(text.len())
            .ok_or("text size overflow")?;
        if seg1_end > i32::MAX as usize {
            return Err(format!(
                "ELF image ({seg1_end} B) exceeds rel32 addressable 2 GiB range"
            ));
        }
        let dyn_vaddr = checked_align_up(seg1_end, PAGE)?;
        self.dyn_vaddr = dyn_vaddr;

        // 6 dynamic entries * 16 bytes = 96 bytes.
        let dyn_size: usize = 6 * 16;

        // Non-allocated `.shstrtab` and 8-entry Section Header Table appended after `.dynamic`.
        // Section indices:
        //   0: SHN_UNDEF
        //   1: .dynsym
        //   2: .hash
        //   3: .dynstr
        //   4: .rodata
        //   5: .text      <-- st_shndx for all exported functions!
        //   6: .dynamic
        //   7: .shstrtab
        const SHNDX_TEXT: u16 = 5;
        let sh_names = [
            "",
            ".dynsym",
            ".hash",
            ".dynstr",
            ".rodata",
            ".text",
            ".dynamic",
            ".shstrtab",
        ];
        let mut shstrtab = Vec::new();
        let mut sh_name_offs = [0u32; 8];
        for (i, name) in sh_names.iter().enumerate() {
            sh_name_offs[i] = shstrtab.len() as u32;
            shstrtab.extend_from_slice(name.as_bytes());
            shstrtab.push(0);
        }
        let shstrtab_off = dyn_vaddr
            .checked_add(dyn_size)
            .ok_or("ELF section table overflow")?;
        let shoff = checked_align_up(
            shstrtab_off
                .checked_add(shstrtab.len())
                .ok_or("ELF section table overflow")?,
            8,
        )?;
        let total_size = shoff
            .checked_add(8 * 64)
            .ok_or("ELF total image overflow")?;
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
        put_u64(&mut img, 40, shoff as u64); // e_shoff
        put_u32(&mut img, 48, 0); // e_flags
        put_u16(&mut img, 52, 64); // e_ehsize
        put_u16(&mut img, 54, 56); // e_phentsize
        put_u16(&mut img, 56, 5); // e_phnum
        put_u16(&mut img, 58, 64); // e_shentsize
        put_u16(&mut img, 60, 8); // e_shnum
        put_u16(&mut img, 62, 7); // e_shstrndx (.shstrtab)

        // ---- Program Headers (5 * 56 bytes at offset 64) ----
        let ph = |img: &mut [u8],
                  idx: usize,
                  p_type: u32,
                  p_flags: u32,
                  off: usize,
                  vaddr: usize,
                  sz: usize,
                  align: usize| {
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
        ph(
            &mut img,
            1,
            PT_LOAD,
            PF_R | PF_X,
            self.text_vaddr,
            self.text_vaddr,
            text.len(),
            PAGE,
        );
        ph(
            &mut img,
            2,
            PT_LOAD,
            PF_R | PF_W,
            dyn_vaddr,
            dyn_vaddr,
            dyn_size,
            PAGE,
        );
        ph(
            &mut img,
            3,
            PT_DYNAMIC,
            PF_R | PF_W,
            dyn_vaddr,
            dyn_vaddr,
            dyn_size,
            8,
        );
        ph(&mut img, 4, PT_GNU_STACK, PF_R | PF_W, 0, 0, 0, 16);

        // ---- .dynsym ----
        let nsyms = 1 + exports.len();
        for (i, def) in exports.iter().enumerate() {
            let sym_idx = i + 1;
            let base = self.dynsym_off + sym_idx * 24;
            put_u32(&mut img, base, self.name_offs[i]); // st_name
            img[base + 4] = (1 << 4) | 2; // STB_GLOBAL | STT_FUNC
            img[base + 5] = 0; // STV_DEFAULT
            put_u16(&mut img, base + 6, SHNDX_TEXT); // real .text section index
            put_u64(&mut img, base + 8, self.text_vaddr as u64 + def.text_off);
            put_u64(&mut img, base + 16, def.size);
        }

        // ---- .hash (SysV ELF hash table) ----
        let mut buckets = vec![0u32; self.nbucket];
        let mut chains = vec![0u32; nsyms];
        for (i, name) in self.export_names.iter().enumerate() {
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

        // ---- .shstrtab ----
        img[shstrtab_off..shstrtab_off + shstrtab.len()].copy_from_slice(&shstrtab);

        // ---- Section Header Table (8 * 64 bytes) ----
        #[allow(clippy::too_many_arguments)]
        let sh = |img: &mut [u8],
                  idx: usize,
                  sh_type: u32,
                  sh_flags: u64,
                  sh_addr: usize,
                  sh_off: usize,
                  sh_size: usize,
                  sh_link: u32,
                  sh_info: u32,
                  sh_align: u64,
                  sh_entsize: u64| {
            let base = shoff + idx * 64;
            put_u32(img, base, sh_name_offs[idx]);
            put_u32(img, base + 4, sh_type);
            put_u64(img, base + 8, sh_flags);
            put_u64(img, base + 16, sh_addr as u64);
            put_u64(img, base + 24, sh_off as u64);
            put_u64(img, base + 32, sh_size as u64);
            put_u32(img, base + 40, sh_link);
            put_u32(img, base + 44, sh_info);
            put_u64(img, base + 48, sh_align);
            put_u64(img, base + 56, sh_entsize);
        };
        const SHT_PROGBITS: u32 = 1;
        const SHT_STRTAB: u32 = 3;
        const SHT_HASH: u32 = 5;
        const SHT_DYNAMIC: u32 = 6;
        const SHT_DYNSYM: u32 = 11;
        const SHF_WRITE: u64 = 1;
        const SHF_ALLOC: u64 = 2;
        const SHF_EXECINSTR: u64 = 4;

        sh(
            &mut img,
            1,
            SHT_DYNSYM,
            SHF_ALLOC,
            self.dynsym_off,
            self.dynsym_off,
            self.dynsym_len,
            3,
            1,
            8,
            24,
        );
        sh(
            &mut img,
            2,
            SHT_HASH,
            SHF_ALLOC,
            self.hash_off,
            self.hash_off,
            self.hash_len,
            1,
            0,
            4,
            4,
        );
        sh(
            &mut img,
            3,
            SHT_STRTAB,
            SHF_ALLOC,
            self.dynstr_off,
            self.dynstr_off,
            self.dynstr.len(),
            0,
            0,
            1,
            0,
        );
        sh(
            &mut img,
            4,
            SHT_PROGBITS,
            SHF_ALLOC,
            self.rodata_vaddr,
            self.rodata_vaddr,
            rodata.len(),
            0,
            0,
            16,
            0,
        );
        sh(
            &mut img,
            5,
            SHT_PROGBITS,
            SHF_ALLOC | SHF_EXECINSTR,
            self.text_vaddr,
            self.text_vaddr,
            text.len(),
            0,
            0,
            16,
            0,
        );
        sh(
            &mut img,
            6,
            SHT_DYNAMIC,
            SHF_ALLOC | SHF_WRITE,
            dyn_vaddr,
            dyn_vaddr,
            dyn_size,
            3,
            0,
            8,
            16,
        );
        sh(
            &mut img,
            7,
            SHT_STRTAB,
            0,
            0,
            shstrtab_off,
            shstrtab.len(),
            0,
            0,
            1,
            0,
        );

        Ok(img)
    }
}

/// Fixed virtual load base for static Linux x86-64 `ET_EXEC` executables (`4 MiB`).
pub const EXEC_BASE_VADDR: usize = 0x0040_0000;

/// Metadata layout for a pure-Rust static Linux x86-64 ELF64 executable (`ET_EXEC`).
///
/// Produces a zero-`DT_NEEDED`, zero-`PT_INTERP`, zero-relocation static Linux binary
/// with `PT_LOAD` R (headers + `.rodata`), `PT_LOAD` RX (`.text`), `PT_GNU_STACK` RW,
/// and conforming section headers (`.rodata`, `.text`, `.shstrtab`).
pub struct ElfExecLayout {
    pub rodata_off: usize,
    pub rodata_vaddr: usize,
    pub text_off: usize,
    pub text_vaddr: usize,
    planned_rodata_len: usize,
}

impl ElfExecLayout {
    pub fn plan(rodata_len: usize) -> Result<Self, String> {
        // 64-byte Ehdr + 3 * 56-byte Phdrs = 232 bytes.
        let rodata_off = checked_align_up(64 + 3 * 56, 16)?;
        let seg0_end = rodata_off
            .checked_add(rodata_len)
            .ok_or("ELF exec rodata size overflow")?;
        let text_off = checked_align_up(seg0_end, PAGE)?;
        let rodata_vaddr = EXEC_BASE_VADDR
            .checked_add(rodata_off)
            .ok_or("ELF exec rodata vaddr overflow")?;
        let text_vaddr = EXEC_BASE_VADDR
            .checked_add(text_off)
            .ok_or("ELF exec text vaddr overflow")?;
        if text_off >= i32::MAX as usize {
            return Err(format!(
                "ELF exec rodata + headers ({text_off} B) exceed rel32 addressable 2 GiB range"
            ));
        }
        Ok(Self {
            rodata_off,
            rodata_vaddr,
            text_off,
            text_vaddr,
            planned_rodata_len: rodata_len,
        })
    }

    /// Emits the complete static Linux x86-64 `ET_EXEC` image with entry point `text_vaddr + entry_off`.
    pub fn emit(self, rodata: &[u8], text: &[u8], entry_off: u64) -> Result<Vec<u8>, String> {
        if rodata.len() != self.planned_rodata_len {
            return Err(format!(
                "ELF exec rodata length changed after plan: planned {} vs emitted {}",
                self.planned_rodata_len,
                rodata.len()
            ));
        }
        if text.is_empty() {
            return Err("ELF exec .text section must be non-empty".into());
        }
        if entry_off >= text.len() as u64 {
            return Err(format!(
                "ELF exec entry offset ({entry_off}) exceeds .text ({})",
                text.len()
            ));
        }
        let seg0_end = self
            .rodata_off
            .checked_add(rodata.len())
            .ok_or("ELF exec rodata overflow")?;
        let seg1_end = self
            .text_off
            .checked_add(text.len())
            .ok_or("ELF exec text overflow")?;
        if seg1_end > i32::MAX as usize {
            return Err(format!(
                "ELF exec image ({seg1_end} B) exceeds rel32 addressable 2 GiB range"
            ));
        }
        let sh_names = ["", ".rodata", ".text", ".shstrtab"];
        let mut shstrtab = Vec::new();
        let mut sh_name_offs = [0u32; 4];
        for (i, name) in sh_names.iter().enumerate() {
            sh_name_offs[i] = shstrtab.len() as u32;
            shstrtab.extend_from_slice(name.as_bytes());
            shstrtab.push(0);
        }
        let shstrtab_off = seg1_end;
        let shoff = checked_align_up(
            shstrtab_off
                .checked_add(shstrtab.len())
                .ok_or("ELF exec shstrtab overflow")?,
            8,
        )?;
        let total_size = shoff
            .checked_add(4 * 64)
            .ok_or("ELF exec total image overflow")?;
        let mut img = vec![0u8; total_size];

        // ---- ELF64 Header ----
        img[0..4].copy_from_slice(b"\x7fELF");
        img[4] = 2; // ELFCLASS64
        img[5] = 1; // ELFDATA2LSB
        img[6] = 1; // EV_CURRENT
        img[7] = 0; // ELFOSABI_NONE
        put_u16(&mut img, 16, 2); // ET_EXEC
        put_u16(&mut img, 18, 62); // EM_X86_64
        put_u32(&mut img, 20, 1); // EV_CURRENT
        put_u64(&mut img, 24, self.text_vaddr as u64 + entry_off); // e_entry
        put_u64(&mut img, 32, 64); // e_phoff
        put_u64(&mut img, 40, shoff as u64); // e_shoff
        put_u32(&mut img, 48, 0); // e_flags
        put_u16(&mut img, 52, 64); // e_ehsize
        put_u16(&mut img, 54, 56); // e_phentsize
        put_u16(&mut img, 56, 3); // e_phnum
        put_u16(&mut img, 58, 64); // e_shentsize
        put_u16(&mut img, 60, 4); // e_shnum
        put_u16(&mut img, 62, 3); // e_shstrndx (.shstrtab)

        let ph = |img: &mut [u8],
                  idx: usize,
                  p_type: u32,
                  p_flags: u32,
                  off: usize,
                  vaddr: usize,
                  sz: usize,
                  align: usize| {
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
        const PT_GNU_STACK: u32 = 0x6474_e551;
        const PF_X: u32 = 1;
        const PF_W: u32 = 2;
        const PF_R: u32 = 4;

        ph(
            &mut img,
            0,
            PT_LOAD,
            PF_R,
            0,
            EXEC_BASE_VADDR,
            seg0_end,
            PAGE,
        );
        ph(
            &mut img,
            1,
            PT_LOAD,
            PF_R | PF_X,
            self.text_off,
            self.text_vaddr,
            text.len(),
            PAGE,
        );
        ph(&mut img, 2, PT_GNU_STACK, PF_R | PF_W, 0, 0, 0, 16);

        img[self.rodata_off..self.rodata_off + rodata.len()].copy_from_slice(rodata);
        img[self.text_off..self.text_off + text.len()].copy_from_slice(text);
        img[shstrtab_off..shstrtab_off + shstrtab.len()].copy_from_slice(&shstrtab);

        #[allow(clippy::too_many_arguments)]
        let sh = |img: &mut [u8],
                  idx: usize,
                  sh_type: u32,
                  sh_flags: u64,
                  sh_addr: usize,
                  sh_off: usize,
                  sh_size: usize,
                  sh_align: u64| {
            let base = shoff + idx * 64;
            put_u32(img, base, sh_name_offs[idx]);
            put_u32(img, base + 4, sh_type);
            put_u64(img, base + 8, sh_flags);
            put_u64(img, base + 16, sh_addr as u64);
            put_u64(img, base + 24, sh_off as u64);
            put_u64(img, base + 32, sh_size as u64);
            put_u32(img, base + 40, 0);
            put_u32(img, base + 44, 0);
            put_u64(img, base + 48, sh_align);
            put_u64(img, base + 56, 0);
        };
        const SHT_PROGBITS: u32 = 1;
        const SHT_STRTAB: u32 = 3;
        const SHF_ALLOC: u64 = 2;
        const SHF_EXECINSTR: u64 = 4;

        sh(
            &mut img,
            1,
            SHT_PROGBITS,
            SHF_ALLOC,
            self.rodata_vaddr,
            self.rodata_off,
            rodata.len(),
            16,
        );
        sh(
            &mut img,
            2,
            SHT_PROGBITS,
            SHF_ALLOC | SHF_EXECINSTR,
            self.text_vaddr,
            self.text_off,
            text.len(),
            16,
        );
        sh(
            &mut img,
            3,
            SHT_STRTAB,
            0,
            0,
            shstrtab_off,
            shstrtab.len(),
            1,
        );

        Ok(img)
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
