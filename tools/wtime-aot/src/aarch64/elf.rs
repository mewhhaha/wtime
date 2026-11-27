//! Pure-Rust Linux AArch64 (`EM_AARCH64 = 183`) ELF64 writer supporting both
//! Blot ABI 8 shared libraries (`ET_DYN`, zero `DT_NEEDED`, zero dynamic relocations)
//! and standalone static Linux AArch64 executables (`ET_EXEC`, zero `PT_INTERP`).
//!
//! Uses 64 KiB (`0x10000`) segment alignment so emitted binaries run cleanly across
//! 4 KiB, 16 KiB, and 64 KiB Linux AArch64 kernels.

use std::collections::BTreeSet;

use crate::x64::elf::ExportDef;

const SEG_ALIGN: usize = 0x10000; // 64 KiB universal Linux AArch64 page alignment
const EM_AARCH64: u16 = 183;

#[inline]
fn checked_align_up(n: usize, a: usize) -> Result<usize, String> {
    let mask = a - 1;
    n.checked_add(mask)
        .map(|v| v & !mask)
        .ok_or_else(|| "aarch64 ELF layout size overflow".to_string())
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

/// Planned Linux AArch64 ELF64 `ET_DYN` shared library layout.
pub struct A64ElfSoLayout {
    pub rodata_vaddr: usize,
    pub text_vaddr: usize,
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

impl A64ElfSoLayout {
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

        let mut cur = 64 + 5 * 56;
        cur = checked_align_up(cur, 8)?;
        let dynsym_off = cur;
        let dynsym_len = nsyms.checked_mul(24).ok_or(".dynsym overflow")?;
        cur = cur.checked_add(dynsym_len).ok_or("ELF header overflow")?;

        cur = checked_align_up(cur, 4)?;
        let hash_off = cur;
        let hash_len = nbucket
            .checked_add(nsyms)
            .and_then(|v| v.checked_add(2))
            .and_then(|v| v.checked_mul(4))
            .ok_or(".hash overflow")?;
        cur = cur.checked_add(hash_len).ok_or("ELF header overflow")?;

        let dynstr_off = cur;
        cur = cur.checked_add(dynstr.len()).ok_or("ELF header overflow")?;
        cur = checked_align_up(cur, 16)?;
        let rodata_vaddr = cur;
        let seg0_end = rodata_vaddr
            .checked_add(rodata_len)
            .ok_or("rodata overflow")?;
        let text_vaddr = checked_align_up(seg0_end, SEG_ALIGN)?;

        Ok(Self {
            rodata_vaddr,
            text_vaddr,
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

    /// Virtual address of the 64-byte writable `GuardState` tail inside `PT_LOAD 2`
    /// when emitting a `--guard-pages` `ET_DYN` image for a `.text` section of `text_len` bytes.
    pub fn guard_state_vaddr_for(&self, text_len: usize) -> Result<usize, String> {
        let seg1_end = self
            .text_vaddr
            .checked_add(text_len)
            .ok_or("aarch64 ELF: text size overflow")?;
        let dyn_vaddr = checked_align_up(seg1_end, SEG_ALIGN)?;
        // 7 dynamic entries (including DT_FLAGS_1 = DF_1_NODELETE) * 16 bytes = 112 bytes.
        dyn_vaddr
            .checked_add(7 * 16)
            .ok_or_else(|| "aarch64 ELF: guard state overflow".to_string())
    }

    #[allow(dead_code)]
    pub fn emit(
        self,
        rodata: &[u8],
        text: &[u8],
        exports: &[ExportDef<'_>],
    ) -> Result<Vec<u8>, String> {
        self.emit_with_guard_pages(rodata, text, exports, false)
    }

    pub fn emit_with_guard_pages(
        self,
        rodata: &[u8],
        text: &[u8],
        exports: &[ExportDef<'_>],
        use_guard_pages: bool,
    ) -> Result<Vec<u8>, String> {
        if rodata.len() != self.planned_rodata_len {
            return Err("aarch64 ELF: rodata length mismatch".into());
        }
        if text.is_empty() {
            return Err("aarch64 ELF: .text must be non-empty".into());
        }
        if exports.len() != self.export_names.len() {
            return Err("aarch64 ELF: export count mismatch".into());
        }
        for (expected, def) in self.export_names.iter().zip(exports) {
            if expected != def.name || def.size == 0 || def.text_off + def.size > text.len() as u64
            {
                return Err(format!("aarch64 ELF: invalid export {:?}", def.name));
            }
        }

        let seg0_end = self.rodata_vaddr + rodata.len();
        let seg1_end = self.text_vaddr + text.len();
        let dyn_vaddr = checked_align_up(seg1_end, SEG_ALIGN)?;
        let dyn_size = if use_guard_pages { 7 * 16 } else { 6 * 16 };
        let rw_size = if use_guard_pages {
            dyn_size + 64
        } else {
            dyn_size
        };

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
        let shstrtab_off = dyn_vaddr + rw_size;
        let shoff = checked_align_up(shstrtab_off + shstrtab.len(), 8)?;
        let total_size = shoff + 8 * 64;
        let mut img = vec![0u8; total_size];

        // ELF64 header
        img[0..4].copy_from_slice(b"\x7fELF");
        img[4] = 2;
        img[5] = 1;
        img[6] = 1;
        put_u16(&mut img, 16, 3); // ET_DYN
        put_u16(&mut img, 18, EM_AARCH64);
        put_u32(&mut img, 20, 1);
        put_u64(&mut img, 32, 64);
        put_u64(&mut img, 40, shoff as u64);
        put_u16(&mut img, 52, 64);
        put_u16(&mut img, 54, 56);
        put_u16(&mut img, 56, 5);
        put_u16(&mut img, 58, 64);
        put_u16(&mut img, 60, 8);
        put_u16(&mut img, 62, 7);

        let ph = |img: &mut [u8],
                  idx: usize,
                  p_type: u32,
                  flags: u32,
                  off: usize,
                  vaddr: usize,
                  sz: usize,
                  align: usize| {
            let b = 64 + idx * 56;
            put_u32(img, b, p_type);
            put_u32(img, b + 4, flags);
            put_u64(img, b + 8, off as u64);
            put_u64(img, b + 16, vaddr as u64);
            put_u64(img, b + 24, vaddr as u64);
            put_u64(img, b + 32, sz as u64);
            put_u64(img, b + 40, sz as u64);
            put_u64(img, b + 48, align as u64);
        };
        ph(&mut img, 0, 1, 4, 0, 0, seg0_end, SEG_ALIGN);
        ph(
            &mut img,
            1,
            1,
            5,
            self.text_vaddr,
            self.text_vaddr,
            text.len(),
            SEG_ALIGN,
        );
        ph(&mut img, 2, 1, 6, dyn_vaddr, dyn_vaddr, rw_size, SEG_ALIGN);
        ph(&mut img, 3, 2, 6, dyn_vaddr, dyn_vaddr, dyn_size, 8);
        ph(&mut img, 4, 0x6474_e551, 6, 0, 0, 0, 16); // PT_GNU_STACK (RW, non-exec)

        // .dynsym + .hash + .dynstr + .rodata + .text
        let nsyms = 1 + exports.len();
        let mut buckets = vec![0u32; self.nbucket];
        let mut chains = vec![0u32; nsyms];
        for (i, def) in exports.iter().enumerate() {
            let sym_idx = i + 1;
            let off = self.dynsym_off + sym_idx * 24;
            put_u32(&mut img, off, self.name_offs[i]);
            img[off + 4] = (1 << 4) | 2; // STB_GLOBAL | STT_FUNC
            put_u16(&mut img, off + 6, 5); // .text
            put_u64(&mut img, off + 8, self.text_vaddr as u64 + def.text_off);
            put_u64(&mut img, off + 16, def.size);
            let b = (elf_hash(def.name.as_bytes()) as usize) % self.nbucket;
            chains[sym_idx] = buckets[b];
            buckets[b] = sym_idx as u32;
        }
        put_u32(&mut img, self.hash_off, self.nbucket as u32);
        put_u32(&mut img, self.hash_off + 4, nsyms as u32);
        for (i, &b) in buckets.iter().enumerate() {
            put_u32(&mut img, self.hash_off + 8 + i * 4, b);
        }
        let chain_base = self.hash_off + 8 + self.nbucket * 4;
        for (i, &c) in chains.iter().enumerate() {
            put_u32(&mut img, chain_base + i * 4, c);
        }
        img[self.dynstr_off..self.dynstr_off + self.dynstr.len()].copy_from_slice(&self.dynstr);
        img[self.rodata_vaddr..self.rodata_vaddr + rodata.len()].copy_from_slice(rodata);
        img[self.text_vaddr..self.text_vaddr + text.len()].copy_from_slice(text);

        // .dynamic
        let mut dyn_entries: Vec<(u64, u64)> = vec![
            (4, self.hash_off as u64),
            (5, self.dynstr_off as u64),
            (6, self.dynsym_off as u64),
            (10, self.dynstr.len() as u64),
            (11, 24),
        ];
        if use_guard_pages {
            dyn_entries.push((0x6fff_fffb, 0x8)); // DT_FLAGS_1 = DF_1_NODELETE
        }
        dyn_entries.push((0, 0)); // DT_NULL
        for (i, &(tag, val)) in dyn_entries.iter().enumerate() {
            put_u64(&mut img, dyn_vaddr + i * 16, tag);
            put_u64(&mut img, dyn_vaddr + i * 16 + 8, val);
        }
        img[shstrtab_off..shstrtab_off + shstrtab.len()].copy_from_slice(&shstrtab);

        #[allow(clippy::too_many_arguments)]
        let sh = |img: &mut [u8],
                  idx: usize,
                  name: u32,
                  stype: u32,
                  flags: u64,
                  addr: usize,
                  off: usize,
                  sz: usize,
                  link: u32,
                  info: u32,
                  align: usize,
                  entsz: usize| {
            let b = shoff + idx * 64;
            put_u32(img, b, name);
            put_u32(img, b + 4, stype);
            put_u64(img, b + 8, flags);
            put_u64(img, b + 16, addr as u64);
            put_u64(img, b + 24, off as u64);
            put_u64(img, b + 32, sz as u64);
            put_u32(img, b + 40, link);
            put_u32(img, b + 44, info);
            put_u64(img, b + 48, align as u64);
            put_u64(img, b + 56, entsz as u64);
        };
        sh(
            &mut img,
            1,
            sh_name_offs[1],
            11,
            2,
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
            sh_name_offs[2],
            5,
            2,
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
            sh_name_offs[3],
            3,
            2,
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
            sh_name_offs[4],
            1,
            2,
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
            sh_name_offs[5],
            1,
            6,
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
            sh_name_offs[6],
            6,
            3,
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
            sh_name_offs[7],
            3,
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

/// Planned standalone Linux AArch64 static executable (`ET_EXEC`, zero `PT_INTERP`/`DT_NEEDED`).
pub struct A64ElfExecLayout {
    pub base_vaddr: usize,
    pub rodata_off: usize,
    pub rodata_vaddr: usize,
    pub text_off: usize,
    pub text_vaddr: usize,
    planned_rodata_len: usize,
}

impl A64ElfExecLayout {
    pub const BASE_VADDR: usize = 0x40_0000;

    pub fn plan(rodata_len: usize) -> Result<Self, String> {
        let rodata_off = checked_align_up(64 + 3 * 56, 16)?;
        let rodata_vaddr = Self::BASE_VADDR + rodata_off;
        let seg0_end = rodata_off + rodata_len;
        let text_off = checked_align_up(seg0_end, SEG_ALIGN)?;
        let text_vaddr = Self::BASE_VADDR + text_off;
        Ok(Self {
            base_vaddr: Self::BASE_VADDR,
            rodata_off,
            rodata_vaddr,
            text_off,
            text_vaddr,
            planned_rodata_len: rodata_len,
        })
    }

    pub fn emit(self, rodata: &[u8], text: &[u8], entry_text_off: u64) -> Result<Vec<u8>, String> {
        if rodata.len() != self.planned_rodata_len
            || text.is_empty()
            || entry_text_off >= text.len() as u64
        {
            return Err("aarch64 ET_EXEC: invalid layout parameters".into());
        }
        let seg0_end = self.rodata_off + rodata.len();
        let text_end = self.text_off + text.len();

        let sh_names = ["", ".rodata", ".text", ".shstrtab"];
        let mut shstrtab = Vec::new();
        let mut sh_name_offs = [0u32; 4];
        for (i, n) in sh_names.iter().enumerate() {
            sh_name_offs[i] = shstrtab.len() as u32;
            shstrtab.extend_from_slice(n.as_bytes());
            shstrtab.push(0);
        }
        let shstrtab_off = text_end;
        let shoff = checked_align_up(shstrtab_off + shstrtab.len(), 8)?;
        let mut img = vec![0u8; shoff + 4 * 64];

        img[0..4].copy_from_slice(b"\x7fELF");
        img[4] = 2;
        img[5] = 1;
        img[6] = 1;
        put_u16(&mut img, 16, 2); // ET_EXEC
        put_u16(&mut img, 18, EM_AARCH64);
        put_u32(&mut img, 20, 1);
        put_u64(&mut img, 24, self.text_vaddr as u64 + entry_text_off);
        put_u64(&mut img, 32, 64);
        put_u64(&mut img, 40, shoff as u64);
        put_u16(&mut img, 52, 64);
        put_u16(&mut img, 54, 56);
        put_u16(&mut img, 56, 3);
        put_u16(&mut img, 58, 64);
        put_u16(&mut img, 60, 4);
        put_u16(&mut img, 62, 3);

        let ph = |img: &mut [u8],
                  idx: usize,
                  p_type: u32,
                  flags: u32,
                  off: usize,
                  vaddr: usize,
                  sz: usize,
                  align: usize| {
            let b = 64 + idx * 56;
            put_u32(img, b, p_type);
            put_u32(img, b + 4, flags);
            put_u64(img, b + 8, off as u64);
            put_u64(img, b + 16, vaddr as u64);
            put_u64(img, b + 24, vaddr as u64);
            put_u64(img, b + 32, sz as u64);
            put_u64(img, b + 40, sz as u64);
            put_u64(img, b + 48, align as u64);
        };
        ph(&mut img, 0, 1, 4, 0, self.base_vaddr, seg0_end, SEG_ALIGN);
        ph(
            &mut img,
            1,
            1,
            5,
            self.text_off,
            self.text_vaddr,
            text.len(),
            SEG_ALIGN,
        );
        ph(&mut img, 2, 0x6474_e551, 6, 0, 0, 0, 16);

        img[self.rodata_off..self.rodata_off + rodata.len()].copy_from_slice(rodata);
        img[self.text_off..self.text_off + text.len()].copy_from_slice(text);
        img[shstrtab_off..shstrtab_off + shstrtab.len()].copy_from_slice(&shstrtab);

        let sh = |img: &mut [u8],
                  idx: usize,
                  name: u32,
                  stype: u32,
                  flags: u64,
                  addr: usize,
                  off: usize,
                  sz: usize,
                  align: usize| {
            let b = shoff + idx * 64;
            put_u32(img, b, name);
            put_u32(img, b + 4, stype);
            put_u64(img, b + 8, flags);
            put_u64(img, b + 16, addr as u64);
            put_u64(img, b + 24, off as u64);
            put_u64(img, b + 32, sz as u64);
            put_u64(img, b + 48, align as u64);
        };
        sh(
            &mut img,
            1,
            sh_name_offs[1],
            1,
            2,
            self.rodata_vaddr,
            self.rodata_off,
            rodata.len(),
            16,
        );
        sh(
            &mut img,
            2,
            sh_name_offs[2],
            1,
            6,
            self.text_vaddr,
            self.text_off,
            text.len(),
            16,
        );
        sh(
            &mut img,
            3,
            sh_name_offs[3],
            3,
            0,
            0,
            shstrtab_off,
            shstrtab.len(),
            1,
        );
        Ok(img)
    }
}
