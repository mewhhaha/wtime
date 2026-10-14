//! Private numeric register format. P058 supplies trusted lowering; later tasks
//! supply execution. Compiling this feature does not select a runtime policy.
mod control;
mod ir;
mod lower;
