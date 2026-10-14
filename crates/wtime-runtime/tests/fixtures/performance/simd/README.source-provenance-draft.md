# P026 original WAT fixtures

These four fixtures are newly authored for P026 and dedicated to this repository under CC0-1.0. They are not copied from the WebAssembly spec corpus. The pinned corpus is consulted only for instruction spelling/folded operand order. All four Wasm fixtures were encoded with the existing conformance encoder; runtime validation and execution remain pending.

## Fixtures and behavior

- `extension_loads.wat`: exports all six `v128.load8x8_{s,u}`, `load16x4_{s,u}`, and `load32x2_{s,u}` variants over a distinctive 16-byte data image. Every expected result is a full 16-byte hex string.
- `splat_zero_loads.wat`: exports all four splat widths, both zero widths, and full `v128.load`, including nonzero upper source bytes to catch accidental leakage into zero loads.
- `lane_loads_oob.wat`: exports load8/16/32/64 lane using nonzero lane indices and a caller vector distinct from memory; also exports a full-width load OOB after a prior store and an 8-byte lane load OOB after a prior store. After either OOB trap, offsets 8 and 9 are `aa a9`: the prefix store changes offset 8, while offset 9 retains its initialized `a9` byte. The complete first 16 memory bytes are `a0a1a2a3a4a5a6a7aaa9aaabacadaeaf`; all later bytes remain zero.
- `shared_vector_loads.wat`: imports and re-exports shared memory, with a full vector load plus load8-lane at address 5 into lane 8. Public API test supplies data bytes `10..1f`.

## Independent expected values

`manifest.json` carries SHA-256 for each WAT source, pinned WAST syntax-reference hashes, exact returned V128 bytes, and trap-prefix expectations. Expected bits were generated with a small independent Python byte model: parse each source byte string, use `int.from_bytes(..., 'little')`, explicit signed two's-complement extension for width-extending loads, byte repetition for splats, zero padding for zero loads, and a byte-range replacement for lane loads. No guest runtime is used in this computation.

The lane folded syntax was checked against the local pinned reference files at `.cache/spec/test/core/simd/simd_load{8,16,32,64}_lane.wast`; for example, that corpus uses `(v128.load8_lane 0 (local.get $address) (local.get $x))`. Its exact hashes are in the manifest. The P026 fixtures use indices 5, 3, 2, and 1 respectively, preserving the same `(lane immediate, address, old vector)` folded operand order.

After encoding is authorized, fill in Wasm SHA-256 values in a separate provenance record; do not change these WAT sources to match an implementation result.

The initial staged trap oracle incorrectly expected byte 9 to be `00`. The fixture initializes bytes 0 through 15 to `a0` through `af`, so byte 9 is `a9` after the prefix store at byte 8. The oracle was corrected before guest execution; no execution proof exists yet.
