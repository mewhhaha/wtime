# Checked Register memory access

P063 admits the 14 scalar load kinds and nine scalar store kinds for ordinary
unshared memory32. The typed lowering records the original memory index,
offset and alignment, plus explicit address and value slots. The immutable
verifier checks these against the original instruction and logical stack
types before publication. Memory64, shared memory, custom pages, SIMD, bulk,
atomics and growth continue through whole-function fallback.

Preparation charges memory argument storage and import lookup work against
the existing transactional budgets. Oversized or unsupported bodies may
consume the available preparation opportunity; later functions then remain
Decoded. Tests preserve this conservative rule and use separate small
fixtures to prove every scalar operation executes on Register.

Frame entry checks actual instance memory handles, Store generations and
backend properties. This includes ordinary memory that an earlier atomic
operation promoted to a synchronized backing. Such a frame declines before
guest work instead of reading the old byte vector. Each admitted access
resolves its current instance binding and current byte length again. No
memory slice, cached base or length survives an operation or invocation.

The executor computes the unsigned guest address and checked immediate sum,
checks the complete byte range, then reads or writes safe slices. Loads use
a fixed eight-byte buffer and explicit little-endian sign extension. Stores
write the low bytes of the typed raw value. Fuel, cancellation and logical
operand limits run before the access. A failed range check preserves earlier
stores and performs neither the failing access nor later stores.

The semantic gate checks 42 load snapshots, 28 store snapshots and five
trapping snapshots. Diagnostic memory prefixes are deliberately capped at
64 bytes, so these snapshots are compared as complete logical frame/operand
state alongside separate complete 65,536-byte memory images. Thirty store
prefix images come from an independently encoded byte oracle. Boundary
cases check the last valid full-width access and first invalid address.

Additional cases verify unsupported forms, backing promotion, distinct
instance bindings, imported aliases, reentry and growth. Nine executions of
the actual predictable, shuffled and read-scan kernels prove Register dispatch
and compare complete memory images with Decoded. The metadata tests reject
changed offsets, alignment, indices, result slots, unused entries and foreign
Store handles transactionally.

The paired decision is recorded in
[P063-records.json](../benchmarks/performance/P063-records.json). Register
remains nondefault; candidate evidence does not replace the portfolio gate.

P063's gate rejected promotion. The equally weighted target warm ratios were
0.82483 and 0.82639 in the two runs, with supported gains. Small and
zero-iteration preparation and cold calls regressed; some zero-iteration warm
controls also regressed. Keep this tested memory interface behind the
nondefault feature for the dependent exit, root and tier-selection work.
