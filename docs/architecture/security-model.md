# Security & Multi-Tenant Worker Isolation Model

`wtime` is designed to work cleanly in **both** everyday local execution (full speed, normal clocks, zero extra overhead by default) and **untrusted multi-tenant worker platforms** (such as running untrusted guest workers side by side on a shared Linux host).

## 1. Isolation Boundaries

| Boundary | What It Guarantees | What It Does **Not** Guarantee |
| --- | --- | --- |
| **`wtime-runtime::Store`** | Architectural Wasm memory/table/GC/type safety in `#![forbid(unsafe_code)]` Rust, zero external production deps, source-level fuel, cancellation, and explicit `ResourceLimits`. No ambient filesystem, network, or clock access unless linked by the host. | Separate OS virtual address space or CPU transient-execution (Spectre) isolation between mutually hostile tenants in the same process. |
| **`wtime-aot` `BlotHandle` (`--blot-cdylib`)** | Separate `mmap`'d linear memories, guarded guest stack, tables, globals, and exception slab per handle. | Cross-tenant transient-execution isolation inside one process (unless compiled with `--spectre-mitigations` and paired with strict host policy), or state scrubbing on `blot_aot_call_reset`. |
| **OS process per worker** | Separate virtual address spaces, distinct Linux UIDs, namespaces, Landlock, `seccomp-bpf`, and cgroups v2 limits. | Hardware cache/SMT partitioning across sibling cores on the same physical CPU. |
| **MicroVM per trust domain** (Firecracker / Cloud Hypervisor) | Separate guest kernel + address space + reduced host attack surface. | Physical LLC or SMT partitioning unless SMT is disabled or core scheduling (`PR_SCHED_CORE`) is configured on the host. |

> **Critical lifecycle rule — never pool handles across tenants via `blot_aot_call_reset`:**
> `blot_aot_call_reset` invokes the guest module's own `blot:reset` export. It is **not** a host-enforced tenant memory scrub and does not zero linear memories, extra memories, tables, globals, the exception slab, or the guest stack. Between tenants, always destroy the process or call `blot_aot_drop` and instantiate a fresh handle.

---

## 2. Opt-In Hardening Controls (Zero Overhead When Off)

### Core Interpreter (`wtime-runtime` / `wtime-cli`)
- **`Config::untrusted_worker()`** (`FeatureSet::untrusted_worker()` + `ResourceLimits::untrusted_worker()`):
  - Disables Wasm `threads` / `SharedMemory` so untrusted modules cannot create concurrent atomic counter-thread timers.
  - Enforces tight per-worker budgets: 16 MiB module cap, 64 MiB linear memory, 32 MiB GC heap, 16 MiB continuations, 65,536 table elements, 1,024 call depth, 16,384 stack height, 10,000,000 default fuel, and `compile_threads = 1`.
  - Leaves all host capabilities (clocks, files, env, network) unlinked by default.
- **CLI**: `wtime-cli --profile untrusted-worker [--fuel N] run module.wasm export`

### Standalone & Goose Host Clock Policy (`wtime-aot` & `wtime-aot-rt`)
Default behavior is unchanged (full nanosecond monotonic and wall clocks for local runs and benchmarking). For untrusted or side-channel-sensitive workers, pass one of:
- **`--no-clock`**:
  - `wasi_snapshot_preview1::clock_time_get` returns WASI `EACCES` (`2`) without writing the output pointer.
  - `env::bench_clock`, `env::gs_os_clock_ns`, and `env::gs_os_time_ns` return `0` without issuing `clock_gettime`.
  - When `--timing-json` is also omitted, standalone `x64`/`aarch64` ELFs emit **zero `clock_gettime` syscalls**, allowing `seccomp-bpf` to deny `clock_gettime` completely; the Goose (`wtime-aot-rt`) runner likewise skips internal `Instant::now()` / `SystemTime::now()` calls when `--timing-json` and `--time-limit` are omitted.
- **`--clock-coarsen-ns N`** (positive integer `N >= 1`, e.g. `1000000` for 1 ms):
  - Truncates all guest-visible timestamps down to multiples of `N` nanoseconds (`t - (t % N)`).
- In Rust embedders using `wtime-aot-rt`, set `goose::Policy::clock` to `ClockPolicy::Deny` or `ClockPolicy::CoarsenNs(n)`.

### Opt-In Spectre v1 Hardening (`wtime-aot --spectre-mitigations`)
Default is off (zero performance cost for normal local builds). Passing `--spectre-mitigations` to `wtime-aot` or `wtime-aot build`:
- Binds the flag into all build and incremental cache keys so hardened and unhardened artifacts never collide.
- **`--backend x64`**: Disables unchecked guard-page bounds elision (`use_guard_pages = false`) and Stage 2 loop BCE/vectorization, emits `lfence` (`0x0f 0xae 0xe8`) on the in-bounds path after linear memory (`memory32`, `memory64`, extra memories), table (`table.get`/`set`), indirect call (`call_indirect`/`call_ref`), `br_table`, and all shared bulk memory/table runtime helper bounds checks (`memory.copy`/`fill`/`init`, extra-memory `copy`/`fill`, `table.init`/`copy`/`grow`/`fill`), and zeroes committed linear memory, extra memories, active dynamic tables, used exception-slab bytes, and `Ctx` before `munmap` in `blot_aot_drop` and failing `blot_aot_new` cleanup.
- **`--backend aarch64`**: Disables unchecked guard-page bounds elision and Stage 2 loop BCE/vectorization, emits `dsb nsh; isb` (`0xd503379f; 0xd5033fdf`) after linear memory, table, indirect call, `br_table`, and all shared bulk memory/table runtime helper checks, and scrubs committed linear memory, extra memories, active dynamic tables, used exception-slab bytes, and `Ctx` before `munmap` in `blot_aot_drop` and failing `blot_aot_new` cleanup.
- **`--backend c`**: Defines `WT_SPECTRE_MITIGATIONS`, inserts `WT_SPEC_BARRIER()` (`lfence` on x86-64, `dsb nsh; isb` on AArch64) after memory, table, `call_indirect`, and `br_table` checks, and volatile-scrubs committed `mem` and `ctx` before freeing in `blot_aot_drop` and failing `blot_aot_new` cleanup.
- **`--backend rust`**: Fails closed (safe `#![forbid(unsafe_code)]` Rust cannot emit hardware speculation barriers; use `--backend x64`, `aarch64`, or `c` when `--spectre-mitigations` is required).

#### Measured `--spectre-mitigations` Overhead (`x86_64`)
Measured on AMD Ryzen 7 7800X3D (Linux `x86_64`, pinned CPU 0, median of 7 alternating rounds, `--blot-cdylib --target-cpu native`, bit-exact checksum parity verified between default and hardened builds):

| Kernel | `x64` Default (ms) | `x64` Spectre (ms) | `x64` Ratio | `c -O2` Default (ms) | `c -O2` Spectre (ms) | `c` Ratio |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `graph_bfs` | `0.0000` | `0.0112` | `703.06x` | `0.0097` | `0.0118` | `1.21x` |
| `parser_hash` | `0.0066` | `0.0604` | `9.17x` | `0.0058` | `0.0594` | `10.17x` |
| `image_filter` | `0.0003` | `0.1586` | `576.81x` | `0.0093` | `0.1585` | `16.96x` |
| `ecs_kernel` | `0.0077` | `0.0792` | `10.32x` | `0.0185` | `0.1222` | `6.59x` |
| `matmul_f64` | `0.0746` | `6.4198` | `86.00x` | `0.1988` | `6.1909` | `31.14x` |
| `heapsort_i32` | `0.0721` | `1.0891` | `15.11x` | `0.1191` | `0.8813` | `7.40x` |
| `dispatch_table` | `0.1782` | `2.0241` | `11.36x` | `0.2442` | `2.0394` | `8.35x` |

**Takeaway:** Pipeline-draining `lfence` on every linear-memory access puts both `x64` and `c` Spectre builds at nearly identical absolute speeds (`~7x–15x` on mixed/dispatch kernels; `~31x–86x` on tight FP memory loops like `matmul_f64`; `graph_bfs` and `image_filter` show higher `x64` ratios because default `x64` applies Stage 2 inlining and AVX2 blur/sum vectorization that Spectre mode disables). For multi-tenant worker platforms, isolating tenants in separate OS processes or microVMs lets workers run fast unmitigated builds without sharing a host address space; reserve `--spectre-mitigations` for defense-in-depth when untrusted handles share an address space with sensitive host or peer state.

---

## 3. Exact Linux `seccomp-bpf` Syscall Allowlists for `wtime-aot`

Because standalone `--backend x64` and `--backend aarch64` ELFs have zero `libc` / `DT_NEEDED` dependencies, a worker platform can apply a minimal `seccomp-bpf` allowlist for the worker process. If you install the filter **before** `execve` (the normal launcher pattern), also allow `execve` (`59` on `x86_64`, `221` on `aarch64`) — or `execveat` (`322` / `281`) — for the initial launch:

### Minimal Compute-Only Standalone Worker (`--no-clock`, no `--allow-files`, no `--timing-json`)
| Architecture | Post-exec Runtime Syscalls (Name / Number) |
| --- | --- |
| **`x86_64`** | `read` (0), `write` (1), `mmap` (9), `mprotect` (10), `munmap` (11), `rt_sigaction` (13), `rt_sigreturn` (15), `exit_group` (231) — **8 syscalls** *(+ `execve` 59 if filter is installed before exec)* |
| **`aarch64`** | `read` (63), `write` (64), `exit_group` (94), `rt_sigaction` (134), `munmap` (215), `mmap` (222), `mprotect` (226) — **7 syscalls** *(plus `rt_sigreturn` 139 if a guard-page signal returns, and `execve` 221 if filter is installed before exec)* |

*(Add `clock_gettime` (`228` on x86-64 / `113` on AArch64) if clocks or `--timing-json` are enabled. Add `openat`, `openat2`, `close`, `fstat`, `getcwd`, `readlinkat` only when `--allow-files` or `--timing-json` is granted.)*

### In-Process `BlotCdylib` (`.so`) Runtime Syscalls
| Architecture | Required Syscalls |
| --- | --- |
| **`x86_64`** | `mmap` (9), `mprotect` (10), `munmap` (11) — **3 syscalls** |
| **`aarch64`** | `munmap` (215), `mmap` (222), `mprotect` (226) — **3 syscalls** |

---

## 4. Recommended Multi-Tenant Platform Deployment

1. **Process-per-worker Linux baseline**:
   - Run one OS process and distinct UID per tenant; never share writable AOT cache directories or tenant-specific `.so` mappings across tenants.
   - Isolate AOT compilation (`wtime-aot build`, including any PGO `--train` runs) in a separate sandboxed compiler service with per-tenant cache directories.
   - Enforce Linux namespaces (mount/pid/net/user + `hidepid` `/proc`), Landlock, `seccomp-bpf` (using the tables above), and cgroups v2 (CPU/RAM/PIDs/I/O) alongside `wtime` fuel/memory limits.
   - Pass `--no-clock` or `--clock-coarsen-ns` (or withhold clock imports in `wtime-runtime`) unless high-precision timing is required.
2. **Hostile multi-tenancy / high-value secrets**:
   - Run each tenant or trust domain inside a **microVM** (Firecracker or Cloud Hypervisor).
   - **Disable SMT** on multi-tenant worker hosts (strongest defense against sibling-core L1/BTB/MDS leakage), or configure Linux core scheduling (`PR_SCHED_CORE`) with per-tenant cookies where partial SMT isolation fits the threat model.
   - Enable `PR_SET_SPECULATION_CTRL` and keep host CPU microcode and kernel mitigations current.
