#![forbid(unsafe_code)]
use wtime_runtime::{Config, Engine};
mod memory_diagnostic_driver;

fn configured_engine(config: Config, mode: &str) -> Engine {
    assert_eq!(mode, "compact", "P043 diagnostics require compact policy");
    Engine::with_execution_policy(config, wtime_runtime::ExecutionPolicy::Compact).unwrap()
}
fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c <= '\u{1f}' => {
                use std::fmt::Write;
                write!(out, "\\u{:04x}", c as u32).unwrap();
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(memory_diagnostic_driver::try_main(&args), "expected --memory-diagnostic");
}
