# Security Policy

## Threat Model & Trust Boundaries

`wtime` separates its **safe-Rust core interpreter** from its **optional ahead-of-time
compiler tooling**:

1. **Core interpreter (`wtime-types`, `wtime-decode`, `wtime-validate`,
   `wtime-runtime`, `wtime-cli`)**:
   - Written in `#![forbid(unsafe_code)]` Rust with zero third-party production
     dependencies.
   - Designed to decode, validate, instantiate, and execute untrusted binary
     WebAssembly modules under explicit `FeatureSet` and `ResourceLimits`
     policies (module size, instruction/local/stack/frame/table/memory caps,
     source-level fuel metering, and cancellation).
   - Malformed binaries, invalid modules, disabled features, resource exhaustion,
     guest traps, guest exceptions, and host failures are reported as distinct
     structured errors without panicking or aborting on guest-controlled sizes.

2. **Ahead-of-time compiler (`tools/wtime-aot`)**:
   - Standalone opt-in tool excluded from the core workspace dependency graph.
   - Generated artifacts execute **unmetered** (no per-instruction fuel or
     in-process cancellation); untrusted workloads must be isolated with OS
     process/cgroup limits and `--time-limit` / `--memory-limit-pages` /
     `--output-limit` / `--stack-limit-mib`.
   - `--backend rust` generates `#![forbid(unsafe_code)]` Rust for all Wasm
     functions and `wtime-aot-rt`; `--blot-cdylib` adds a thin C-ABI FFI wrapper
     for opaque handles and MXCSR/stack-floor guards.
   - `--backend c` invokes the operator's host C compiler (`clang`/`gcc`) and
     linker (`ld.lld`/`ld`) on generated C99; compiler bugs and operator-owned
     toolchain/header integrity are part of the build trust boundary.
   - `--backend x64` emits Linux x86-64 LP64 machine code (`.so` and static
     `ET_EXEC`) directly in pure safe Rust with zero external compilers or
     `libc`. Generated `.so` modules run on a dedicated `mmap`ed guest stack
     with a `PROT_NONE` guard page and enforce CPUID + `rdsspq` CET
     shadow-stack checks at runtime.
   - `--backend aarch64` emits Linux AArch64 ELF64 machine code (Blot ABI 8
     `.so` and static `ET_EXEC`) directly in pure safe Rust with zero external
     compilers or `libc`.
   - Run `wtime-aot build` on untrusted modules with `--time-limit` and an OS
     memory cgroup, using a private cache directory (`0700`) owned solely by the
     operator.

## Supported Versions

| Version | Supported |
| --- | --- |
| `0.1.x` | Yes (`x86_64-unknown-linux-gnu`) |

## Reporting a Vulnerability

Please report suspected security issues (sandbox escapes, memory safety bugs,
out-of-bounds memory/table accesses, unhandled panics/DoS on malformed Wasm, or
ABI/FFI soundness bugs) privately through the repository's GitHub Security
Advisories workflow (`https://github.com/mewhhaha/wtime/security/advisories/new`)
rather than opening a public issue.

Include:
- Affected crate or tool (`wtime-decode`, `wtime-validate`, `wtime-runtime`,
  `wtime-cli`, or `tools/wtime-aot` backend) and exact commit SHA or version.
- Minimal `.wat` / `.wasm` reproducer and exact invocation/flags.
- Observed vs. expected outcome (error category, crash, or wrong result).
