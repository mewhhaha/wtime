#![forbid(unsafe_code)]
#[path = "../../src/protocol.rs"]
mod protocol;
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(protocol::try_main(&args), "unsupported profiler command");
}
