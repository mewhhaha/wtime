#![forbid(unsafe_code)]
//! Bounded WebAssembly binary decoding. All spans refer to the owned module bytes.
mod instructions;
mod module;
mod reader;
pub use instructions::{Catch, Immediate, Instruction, decode_instruction};
pub use module::*;
pub use reader::Reader;

mod metadata;
pub use metadata::*;
