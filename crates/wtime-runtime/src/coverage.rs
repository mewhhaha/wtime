//! Opt-in development instrumentation; absent from the default engine.
use std::sync::atomic::{AtomicU64, Ordering};
static EXECUTED: [AtomicU64; 1024] = [const { AtomicU64::new(0) }; 1024];
pub(crate) fn record(opcode: u16) {
    EXECUTED[opcode as usize / 64].fetch_or(1 << (opcode % 64), Ordering::Relaxed);
}
/// Resets the global executed-opcode coverage bitmap.
pub fn clear_opcode_coverage() {
    for word in &EXECUTED {
        word.store(0, Ordering::Relaxed);
    }
}
/// Returns the sorted list of opcode keys executed since the last [`clear_opcode_coverage`] call.
pub fn opcode_coverage() -> Vec<u16> {
    let mut result = Vec::new();
    for (index, word) in EXECUTED.iter().enumerate() {
        let mut bits = word.load(Ordering::Relaxed);
        while bits != 0 {
            let bit = bits.trailing_zeros();
            result.push((index * 64 + bit as usize) as u16);
            bits &= bits - 1;
        }
    }
    result
}
