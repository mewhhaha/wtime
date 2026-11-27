//! SHA-256 (FIPS 180-4), for the build cache's content keys.

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// An incremental SHA-256 hasher (zero heap allocation, direct 64-byte chunk compression).
#[derive(Clone)]
pub struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    block_len: usize,
    length: u64,
}

impl Sha256 {
    pub fn new() -> Sha256 {
        Sha256 {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            block: [0u8; 64],
            block_len: 0,
            length: 0,
        }
    }

    pub fn update(&mut self, mut bytes: &[u8]) {
        self.length = self.length.wrapping_add(bytes.len() as u64);
        if self.block_len > 0 {
            let take = (64 - self.block_len).min(bytes.len());
            self.block[self.block_len..self.block_len + take].copy_from_slice(&bytes[..take]);
            self.block_len += take;
            bytes = &bytes[take..];
            if self.block_len == 64 {
                let b = self.block;
                self.compress(&b);
                self.block_len = 0;
            }
        }
        let (chunks, rem) = bytes.as_chunks::<64>();
        for chunk in chunks {
            self.compress(chunk);
        }
        if !rem.is_empty() {
            self.block[..rem.len()].copy_from_slice(rem);
            self.block_len = rem.len();
        }
    }

    /// Finalizes and returns the raw 32-byte digest.
    pub fn finish(mut self) -> [u8; 32] {
        let bits = self.length.wrapping_mul(8);
        self.block[self.block_len] = 0x80;
        self.block_len += 1;
        if self.block_len > 56 {
            self.block[self.block_len..64].fill(0);
            let b = self.block;
            self.compress(&b);
            self.block_len = 0;
        }
        self.block[self.block_len..56].fill(0);
        self.block[56..64].copy_from_slice(&bits.to_be_bytes());
        let b = self.block;
        self.compress(&b);
        let mut out = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    /// The digest as 64 lowercase hex digits.
    pub fn hex(self) -> String {
        to_hex(&self.finish())
    }

    #[inline(always)]
    fn compress(&mut self, block: &[u8; 64]) {
        let words = block.as_chunks::<4>().0;
        let mut w = [
            u32::from_be_bytes(words[0]),
            u32::from_be_bytes(words[1]),
            u32::from_be_bytes(words[2]),
            u32::from_be_bytes(words[3]),
            u32::from_be_bytes(words[4]),
            u32::from_be_bytes(words[5]),
            u32::from_be_bytes(words[6]),
            u32::from_be_bytes(words[7]),
            u32::from_be_bytes(words[8]),
            u32::from_be_bytes(words[9]),
            u32::from_be_bytes(words[10]),
            u32::from_be_bytes(words[11]),
            u32::from_be_bytes(words[12]),
            u32::from_be_bytes(words[13]),
            u32::from_be_bytes(words[14]),
            u32::from_be_bytes(words[15]),
        ];
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        macro_rules! r0 {
            ($a:ident, $b:ident, $c:ident, $d:ident, $e:ident, $f:ident, $g:ident, $h:ident, $idx:expr) => {{
                let s1 = $e.rotate_right(6) ^ $e.rotate_right(11) ^ $e.rotate_right(25);
                let ch = $g ^ ($e & ($f ^ $g));
                let t1 = $h
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[$idx])
                    .wrapping_add(w[$idx]);
                let s0 = $a.rotate_right(2) ^ $a.rotate_right(13) ^ $a.rotate_right(22);
                let maj = ($a & $b) | ($c & ($a | $b));
                $d = $d.wrapping_add(t1);
                $h = t1.wrapping_add(s0.wrapping_add(maj));
            }};
        }
        macro_rules! rx_step {
            ($a:ident, $b:ident, $c:ident, $d:ident, $e:ident, $f:ident, $g:ident, $h:ident, $idx:expr, $store:tt) => {{
                const I0: usize = $idx & 15;
                const I1: usize = ($idx + 1) & 15;
                const I9: usize = ($idx + 9) & 15;
                const I14: usize = ($idx + 14) & 15;
                let wm15 = w[I1];
                let wm2 = w[I14];
                let ws0 = wm15.rotate_right(7) ^ wm15.rotate_right(18) ^ (wm15 >> 3);
                let ws1 = wm2.rotate_right(17) ^ wm2.rotate_right(19) ^ (wm2 >> 10);
                let wt = w[I0]
                    .wrapping_add(ws0)
                    .wrapping_add(w[I9])
                    .wrapping_add(ws1);
                rx_store!(w[I0] = wt, $store);
                let s1 = $e.rotate_right(6) ^ $e.rotate_right(11) ^ $e.rotate_right(25);
                let ch = $g ^ ($e & ($f ^ $g));
                let t1 = $h
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[$idx])
                    .wrapping_add(wt);
                let s0 = $a.rotate_right(2) ^ $a.rotate_right(13) ^ $a.rotate_right(22);
                let maj = ($a & $b) | ($c & ($a | $b));
                $d = $d.wrapping_add(t1);
                $h = t1.wrapping_add(s0.wrapping_add(maj));
            }};
        }
        macro_rules! rx_store {
            ($dst:expr, yes) => {
                $dst;
            };
            ($dst:expr, no) => {};
        }
        macro_rules! rx {
            ($a:ident, $b:ident, $c:ident, $d:ident, $e:ident, $f:ident, $g:ident, $h:ident, $idx:expr) => {
                rx_step!($a, $b, $c, $d, $e, $f, $g, $h, $idx, yes)
            };
        }
        macro_rules! rx_end {
            ($a:ident, $b:ident, $c:ident, $d:ident, $e:ident, $f:ident, $g:ident, $h:ident, $idx:expr) => {
                rx_step!($a, $b, $c, $d, $e, $f, $g, $h, $idx, no)
            };
        }
        r0!(a, b, c, d, e, f, g, h, 0);
        r0!(h, a, b, c, d, e, f, g, 1);
        r0!(g, h, a, b, c, d, e, f, 2);
        r0!(f, g, h, a, b, c, d, e, 3);
        r0!(e, f, g, h, a, b, c, d, 4);
        r0!(d, e, f, g, h, a, b, c, 5);
        r0!(c, d, e, f, g, h, a, b, 6);
        r0!(b, c, d, e, f, g, h, a, 7);
        r0!(a, b, c, d, e, f, g, h, 8);
        r0!(h, a, b, c, d, e, f, g, 9);
        r0!(g, h, a, b, c, d, e, f, 10);
        r0!(f, g, h, a, b, c, d, e, 11);
        r0!(e, f, g, h, a, b, c, d, 12);
        r0!(d, e, f, g, h, a, b, c, 13);
        r0!(c, d, e, f, g, h, a, b, 14);
        r0!(b, c, d, e, f, g, h, a, 15);
        rx!(a, b, c, d, e, f, g, h, 16);
        rx!(h, a, b, c, d, e, f, g, 17);
        rx!(g, h, a, b, c, d, e, f, 18);
        rx!(f, g, h, a, b, c, d, e, 19);
        rx!(e, f, g, h, a, b, c, d, 20);
        rx!(d, e, f, g, h, a, b, c, 21);
        rx!(c, d, e, f, g, h, a, b, 22);
        rx!(b, c, d, e, f, g, h, a, 23);
        rx!(a, b, c, d, e, f, g, h, 24);
        rx!(h, a, b, c, d, e, f, g, 25);
        rx!(g, h, a, b, c, d, e, f, 26);
        rx!(f, g, h, a, b, c, d, e, 27);
        rx!(e, f, g, h, a, b, c, d, 28);
        rx!(d, e, f, g, h, a, b, c, 29);
        rx!(c, d, e, f, g, h, a, b, 30);
        rx!(b, c, d, e, f, g, h, a, 31);
        rx!(a, b, c, d, e, f, g, h, 32);
        rx!(h, a, b, c, d, e, f, g, 33);
        rx!(g, h, a, b, c, d, e, f, 34);
        rx!(f, g, h, a, b, c, d, e, 35);
        rx!(e, f, g, h, a, b, c, d, 36);
        rx!(d, e, f, g, h, a, b, c, 37);
        rx!(c, d, e, f, g, h, a, b, 38);
        rx!(b, c, d, e, f, g, h, a, 39);
        rx!(a, b, c, d, e, f, g, h, 40);
        rx!(h, a, b, c, d, e, f, g, 41);
        rx!(g, h, a, b, c, d, e, f, 42);
        rx!(f, g, h, a, b, c, d, e, 43);
        rx!(e, f, g, h, a, b, c, d, 44);
        rx!(d, e, f, g, h, a, b, c, 45);
        rx!(c, d, e, f, g, h, a, b, 46);
        rx!(b, c, d, e, f, g, h, a, 47);
        rx!(a, b, c, d, e, f, g, h, 48);
        rx!(h, a, b, c, d, e, f, g, 49);
        rx!(g, h, a, b, c, d, e, f, 50);
        rx!(f, g, h, a, b, c, d, e, 51);
        rx!(e, f, g, h, a, b, c, d, 52);
        rx!(d, e, f, g, h, a, b, c, 53);
        rx!(c, d, e, f, g, h, a, b, 54);
        rx!(b, c, d, e, f, g, h, a, 55);
        rx!(a, b, c, d, e, f, g, h, 56);
        rx!(h, a, b, c, d, e, f, g, 57);
        rx!(g, h, a, b, c, d, e, f, 58);
        rx!(f, g, h, a, b, c, d, e, 59);
        rx!(e, f, g, h, a, b, c, d, 60);
        rx!(d, e, f, g, h, a, b, c, 61);
        rx_end!(c, d, e, f, g, h, a, b, 62);
        rx_end!(b, c, d, e, f, g, h, a, 63);
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

pub fn to_hex(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &b in digest {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

pub fn digest(bytes: &[u8]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(bytes);
    hash.finish()
}

pub fn hex(bytes: &[u8]) -> String {
    to_hex(&digest(bytes))
}

/// Chunk size for parallel 256 KiB Merkle SHA-256 artifact sidecars (`merkle256k:<64-hex>`).
pub const MERKLE_CHUNK_BYTES: usize = 256 << 10;
const MERKLE_PREFIX: &str = "merkle256k:";

/// Computes a deterministic SHA-256 Merkle root over `chunk_bytes`-sized chunks with domain tag
/// `domain` in parallel across up to 16 threads using dynamic work-stealing. Uses fallible
/// allocation and `spawn_scoped` with inline fallback.
pub fn parallel_merkle_digest_with_chunk(
    domain: &[u8],
    bytes: &[u8],
    chunk_bytes: usize,
) -> Option<[u8; 32]> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    if bytes.len() <= chunk_bytes {
        let mut h = Sha256::new();
        h.update(domain);
        h.update(&(bytes.len() as u64).to_le_bytes());
        h.update(&digest(bytes));
        return Some(h.finish());
    }
    let n_chunks = bytes.len().div_ceil(chunk_bytes);
    let mut leaf_hashes: Vec<[u8; 32]> = Vec::new();
    leaf_hashes.try_reserve_exact(n_chunks).ok()?;
    leaf_hashes.resize(n_chunks, [0u8; 32]);
    let n_threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(16)
        .min(n_chunks);
    let next_chunk = AtomicUsize::new(0);
    let mut local_slots: Vec<Vec<(usize, [u8; 32])>> = Vec::new();
    local_slots.try_reserve_exact(n_threads).ok()?;
    for _ in 0..n_threads {
        local_slots.push(Vec::new());
    }
    std::thread::scope(|s| {
        let next_ref = &next_chunk;
        let run_worker = |out: &mut Vec<(usize, [u8; 32])>| {
            loop {
                let idx = next_ref.fetch_add(1, Ordering::Relaxed);
                if idx >= n_chunks {
                    break;
                }
                let off = idx * chunk_bytes;
                let end = (off + chunk_bytes).min(bytes.len());
                out.push((idx, digest(&bytes[off..end])));
            }
        };
        let mut rem = local_slots.as_mut_slice();
        while let Some((first, rest)) = rem.split_first_mut() {
            rem = rest;
            if rem.is_empty() {
                run_worker(first);
            } else {
                let _ = std::thread::Builder::new().spawn_scoped(s, move || run_worker(first));
            }
        }
    });
    for slot_vec in &local_slots {
        for &(idx, d) in slot_vec {
            leaf_hashes[idx] = d;
        }
    }
    let mut root = Sha256::new();
    root.update(domain);
    root.update(&(bytes.len() as u64).to_le_bytes());
    for leaf in &leaf_hashes {
        root.update(leaf);
    }
    Some(root.finish())
}

/// Computes a deterministic SHA-256 Merkle root over 256 KiB chunks with domain tag `domain`
/// in parallel across up to 16 threads. Uses fallible allocation and `spawn_scoped` with inline fallback.
pub fn parallel_merkle_digest_with_domain(domain: &[u8], bytes: &[u8]) -> Option<[u8; 32]> {
    parallel_merkle_digest_with_chunk(domain, bytes, MERKLE_CHUNK_BYTES)
}

/// Computes a deterministic SHA-256 Merkle root over 256 KiB chunks in parallel across up to 16
/// threads. Uses fallible allocation and `spawn_scoped` with inline fallback.
pub fn parallel_merkle_digest(bytes: &[u8]) -> Option<[u8; 32]> {
    parallel_merkle_digest_with_domain(b"WTIME-MERKLE256K-1", bytes)
}

/// Computes the integrity sidecar digest for a cached build artifact.
///
/// Small artifacts (`<= 256 KiB`) emit plain 64-hex standard FIPS 180-4 SHA-256 (matching
/// `sha256sum`); large artifacts (`> 256 KiB`) emit a self-describing `merkle256k:<64-hex>`
/// parallel SHA-256 Merkle root so multi-MiB `.so` publication and cache-hit verification scale
/// across cores.
pub fn artifact_sidecar_digest(bytes: &[u8]) -> String {
    if bytes.len() > MERKLE_CHUNK_BYTES
        && let Some(root) = parallel_merkle_digest(bytes)
    {
        return format!("{MERKLE_PREFIX}{}", to_hex(&root));
    }
    hex(bytes)
}

/// Verifies `bytes` against an integrity sidecar line (plain 64-hex SHA-256 or `merkle256k:<64-hex>`).
pub fn verify_artifact_sidecar(bytes: &[u8], expected_trimmed: &str) -> bool {
    if let Some(expected_hex) = expected_trimmed.strip_prefix(MERKLE_PREFIX) {
        expected_hex.len() == 64
            && parallel_merkle_digest(bytes).is_some_and(|root| to_hex(&root) == expected_hex)
    } else {
        expected_trimmed.len() == 64 && hex(bytes) == expected_trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digests() {
        assert_eq!(
            hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let mut million = Sha256::new();
        for _ in 0..1000 {
            million.update(&[b'a'; 1000]);
        }
        assert_eq!(
            million.hex(),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
        // Boundary lengths around 55/56/63/64/65/128 bytes
        for len in [55usize, 56, 57, 63, 64, 65, 119, 120, 128, 129] {
            let data: Vec<u8> = (0..len).map(|i| (i * 31) as u8).collect();
            let one_shot = hex(&data);
            let mut inc = Sha256::new();
            for ch in data.chunks(7) {
                inc.update(ch);
            }
            assert_eq!(one_shot, inc.hex(), "mismatch at len {len}");
        }
        // Verify sidecar helper on both small (plain 64-hex) and large (>256 KiB merkle256k) payloads.
        let small_dig = artifact_sidecar_digest(b"hello-small");
        assert_eq!(small_dig.len(), 64);
        assert!(verify_artifact_sidecar(b"hello-small", &small_dig));
        assert!(!verify_artifact_sidecar(b"hello-smalL", &small_dig));

        let mut big = vec![0x5au8; MERKLE_CHUNK_BYTES * 2 + 17];
        for (i, b) in big.iter_mut().enumerate() {
            *b = (i.wrapping_mul(131) ^ (i >> 8)) as u8;
        }
        let big_dig = artifact_sidecar_digest(&big);
        assert!(big_dig.starts_with(MERKLE_PREFIX));
        assert!(verify_artifact_sidecar(&big, &big_dig));
        // Flip a byte in the middle chunk and confirm tamper detection.
        big[MERKLE_CHUNK_BYTES + 5] ^= 0x01;
        assert!(!verify_artifact_sidecar(&big, &big_dig));

        // Verify multi-chunk Merkle root against scalar `digest` leaf by leaf.
        let mut huge = vec![0u8; MERKLE_CHUNK_BYTES * 5 + 37];
        for (i, b) in huge.iter_mut().enumerate() {
            *b = (i.wrapping_mul(29) ^ (i >> 7)) as u8;
        }
        let huge_dig = artifact_sidecar_digest(&huge);
        assert!(verify_artifact_sidecar(&huge, &huge_dig));
        let mut manual = Sha256::new();
        manual.update(b"WTIME-MERKLE256K-1");
        manual.update(&(huge.len() as u64).to_le_bytes());
        for ch in huge.chunks(MERKLE_CHUNK_BYTES) {
            manual.update(&digest(ch));
        }
        assert_eq!(huge_dig, format!("{MERKLE_PREFIX}{}", manual.hex()));
    }
}
