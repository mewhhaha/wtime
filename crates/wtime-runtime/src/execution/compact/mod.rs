//! Experimental immutable compact representation. No dispatch changes yet.
//! All constructors consume this module's admitted source and plans.
use super::*;
mod ir;
mod calls;
mod indirect;
pub(crate) use indirect::CallSites;
pub(crate) use calls::{CallBindings, guest_frame};
pub(crate) use ir::lower_scalar::PreparedModule;
pub(crate) use ir::execute::Step;
pub(crate) use ir::fuse::execute as execute_fusion;
pub(crate) use ir::inline::try_leaf as try_inline_leaf;
pub(crate) use ir::memory::MemoryBindings;
pub(crate) use ir::lower_control::keep;
