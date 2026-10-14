# Starting evidence and source map

This records inspection of the implementation on 2026-10-11. Costs below are visible operations, not profiled percentages or measured explanations of the full Wasmtime gap. Follow relocated symbols after predecessor patches; line numbers are starting hints only.

| Area | Current source | Observed behavior | First tasks |
| --- | --- | --- | --- |
| Module ownership | [lib.rs](../../wtime/crates/wtime-runtime/src/lib.rs) `Module`, `Engine::compile` | `Module` derives Clone over owned `control` and `canonical` vectors plus an Arc of validated input. | P017 |
| Instruction dispatch | [execution.rs](../../wtime/crates/wtime-runtime/src/execution.rs) `Store::invoke` | Each original instruction clones `self.instances[inst_id].module`, then clones its `Instruction`. The module clone copies control/type vectors. | P006/P007/P017/P023 |
| Branch results | `execution.rs` `keep`, `Label` | `split_off` allocates a result tail; catch vectors are owned/cloned. | P024 |
| Scalar/vector loads | `lib.rs` `Mem::read`; `execution.rs` `load_store`, `simd_memory` | A guest load allocates `Vec<u8>` for even a 1–16-byte read, then copies into a fixed buffer. Stores already copy from stack bytes without a read Vec. | P025/P026 |
| Calls | `execution.rs` `Store::frame`, call opcode range | Frame entry clones `Function`/signature, allocates locals, checks arguments; calls use `stack.split_off`. | P029–P032/P041 |
| Numeric stack | [numeric.rs](../../wtime/crates/wtime-runtime/src/numeric.rs) `exec` | General Value stack pop/push and opcode dispatch. | P028/P038/P061 |
| Decoded instructions | [instructions.rs](../../wtime/crates/wtime-decode/src/instructions.rs) `Instruction`, `Immediate` | Immediate includes Vec-backed branch tables/types/bytes, boxed try_table and handler vectors. This type is in decode, not types. | P023/P036 |
| Control metadata | `lib.rs` `lower_control`, `ControlMap` | Sparse `Vec<Vec<Option<(usize, Option<usize>)>>>` is indexed at runtime. | P021/P040 |
| Indirect calls/types | `execution.rs`; [canonical.rs](../../wtime/crates/wtime-runtime/src/canonical.rs) `is_subtype` | Mutex plus bounded declared-supertype walk on repeated indirect/call_ref queries. Exact/equal IDs need identity checks but no walk. | P033/P042 |
| GC local definitions | [gc_exec.rs](../../wtime/crates/wtime-runtime/src/gc_exec.rs) `gc_type`, `gc_field` | Local type groups are flattened, nth-selected and cloned for lookup. | P034 |
| Root snapshots | `execution.rs` host-call branch; `lib.rs` `collect`, `active_roots` | Owned snapshots copy numeric and reference Values; borrowed snapshots cannot safely cross mutable host reentry. | P068/P069 |
| Collection | [gc.rs](../../wtime/crates/wtime-runtime/src/gc.rs) `GcArena` | Nonmoving arena, generation checks and mark-on-enqueue; byte and `max_gc_mark_work` policies are deliberate. `CollectionStats` exposes live/reclaimed; `Store::gc_payload_bytes()` provides bytes separately. | P070/P071 |
| Continuations | [continuation.rs](../../wtime/crates/wtime-runtime/src/continuation.rs) `continuation_bytes`, `capture`, `segment_roots` | Byte checks scan retained continuations/capacities. Capture already moves detached frames/operands; it also owns root/signature metadata. There is no public host resume method; guest driver exports execute resume instructions. | P012/P072/P073 |
| Portable SIMD | [simd.rs](../../wtime/crates/wtime-runtime/src/simd.rs) `map`, `zip`, `execute` | Safe lane loops and generic byte conversions. Actual hot SIMD operations have not been benchmarked yet. | P010/P074/P075/P104 |
| Shared memory | [shared_memory.rs](../../wtime/crates/wtime-runtime/src/shared_memory.rs) | Safe mutexed byte Vec; global SC coordinator; waiters already indexed by address in a BTreeMap. P077 targets stale/linear per-address queue scanning, not creating the first address map. | P013/P076–P078 |

The exact current core source aggregate in [native/build evidence](../../wtime/docs/verification/build-final.json) is `e5a3a4eeba54a547ec8102acb6d03bb823de92643b7b34fc435817fcf0a3c75f`. That report records clean build 6.778 s, unchanged 0.099 s, touched CLI leaf 0.265 s, release 14.813 s, stripped CLI 1,002,024 bytes, and clean peak RSS 285,496 KiB. It differs from an older report under `docs/architecture/build-final.json`; use the current verification report and archived binary identities when establishing the new baseline. These are current-environment observations, not portable timing ceilings.

The benchmark tool is a separate workspace at [tools/wtime-bench](../../wtime/tools/wtime-bench/README.md), pinned to Wasmtime 49.0.2 with std/runtime/cranelift. It has an owned lockfile and currently supports four scalar runtime cases plus a prepare-only large module. Historical runs contain 21 samples per engine/stage; the new paired protocol increases the evidence requirement. Production dependencies remain zero external crates.

Existing acceptance sources include `crates/wtime-runtime/tests/{core,exceptions,feature_profiles,proposals}.rs`, unit tests in `lib.rs`, `canonical.rs`, `continuation.rs`, `gc.rs`, `gc_exec.rs`, `shared_memory.rs`, numeric/SIMD helpers, and the pinned conformance runner. Reuse them; performance tests add meaningful changed-path boundaries, not duplicate every old instruction test.

Native AArch64 execution evidence is absent in the original release gates. Neither this planning pass, cross-compilation, nor a future x86 prototype resolves that evidence gap.
