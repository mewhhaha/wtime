# Dependency policy

Production crates use only Rust std and workspace crates. The conformance binary
uses pinned `wast` for text encoding and `sha2` for portable source/evidence hashes,
including its build script. These are isolated in Cargo.toml and Cargo.lock. Their transitive
licenses are emitted by `python3 tools/checks/dependency_budget.py` from Cargo
metadata. No production crate has dev dependencies on parser or engine libraries.

Unsafe code has a budget of zero. The checker conservatively flags unsafe
blocks, functions, implementations, traits and extern declarations in production
source. Any future exception requires a reviewed inventory and a reason the safe
implementation cannot meet the requirement. No exception is currently approved.
