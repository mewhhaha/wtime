//! `wtime-aot` library: ahead-of-time WebAssembly compilers (safe Rust, C99/Clang,
//! and pure-Rust Linux x86-64 / AArch64 ELF64 backends).
//!
//! All compiler and runtime emission code is `#![forbid(unsafe_code)]`.

#![forbid(unsafe_code)]

pub mod aarch64;
pub mod build;
pub mod cgen;
pub mod cli;
pub mod codegen;
pub mod pgo;
pub mod sha256;
pub mod x64;

/// Runs the `wtime-aot` command-line interface.
pub fn cli_main() {
    cli::main();
}
