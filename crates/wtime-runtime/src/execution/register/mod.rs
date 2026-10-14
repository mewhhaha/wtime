//! Private numeric register format. P058 supplies trusted lowering; later tasks
//! supply execution. Compiling this feature does not select a runtime policy.
mod control;
mod execute;
mod float;
mod integer;
mod ir;
mod lower;
#[cfg(feature = "execution-reference")]
pub(crate) use execute::materialize;
pub(crate) use execute::{Activation, PreparedModule, Step, activate, logical_height, step};
