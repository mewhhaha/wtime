//! Prints how often each SIMD opcode occurs in a module (every opcode
//! with `--all`).
use std::collections::BTreeMap;
use wtime_types::{FeatureSet, ResourceLimits};
use wtime_validate::validate;

fn main() {
    let path = std::env::args().nth(1).expect("module.wasm");
    let all = std::env::args().any(|a| a == "--all");
    let bytes = std::fs::read(path).expect("read");
    let validated = validate(
        &bytes,
        &FeatureSet::all_phase3_core(),
        &ResourceLimits {
            max_module_bytes: 256 << 20,
            max_instructions: 100_000_000,
            ..ResourceLimits::default()
        },
    )
    .expect("valid");
    let mut counts = BTreeMap::new();
    for body in &validated.decoded().code {
        for i in &body.instructions {
            if all || i.opcode >= 0xfd00 {
                *counts.entry(i.opcode).or_insert(0u64) += 1;
            }
        }
    }
    for (op, n) in counts {
        println!("{op:#06x} {n}");
    }
}
