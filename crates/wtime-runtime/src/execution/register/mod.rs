//! Private numeric register execution tier (disabled by default).
#[allow(dead_code)]
mod cache;
mod control;
mod execute;
mod exits;
#[cfg(not(any(
    feature = "execution-reference",
    feature = "performance-counters",
    feature = "opcode-coverage"
)))]
pub(crate) mod fast;
mod float;
mod integer;
mod ir;
mod lower;
mod memory;
pub(crate) use cache::Cache;
#[cfg(feature = "execution-reference")]
pub(crate) use execute::materialize;
pub(crate) use execute::{Activation, PreparedModule, Step, activate_entry, logical_height, step};
