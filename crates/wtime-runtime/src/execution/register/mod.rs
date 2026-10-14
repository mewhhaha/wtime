//! Private numeric register format. P058 supplies trusted lowering; later tasks
//! supply execution. Compiling this feature does not select a runtime policy.
// Standalone preparation foundation; P066 will connect entry-point selection.
#[allow(dead_code)]
mod cache;
mod control;
mod execute;
mod exits;
mod float;
mod integer;
mod ir;
mod lower;
mod memory;
#[cfg(feature = "execution-reference")]
pub(crate) use execute::materialize;
pub(crate) use execute::{
    Activation, PreparedModule, Step, activate, bindings_admitted, logical_height, step,
};
