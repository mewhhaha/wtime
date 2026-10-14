//! Diagnostic-only allocator accounting. Never used for production timings.
//! Counts requested live heap bytes, excluding allocator bookkeeping and RSS.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use wtime_runtime::{Config, Engine, ExecutionPolicy};
use wtime_types::FeatureSet;

struct RequestedBytes;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn added(bytes: usize) {
    let total = LIVE.fetch_add(bytes, Relaxed) + bytes;
    PEAK.fetch_max(total, Relaxed);
}

// SAFETY: every allocation operation forwards the original pointer/layout
// unchanged to System. Counters neither allocate nor change pointer ownership.
unsafe impl GlobalAlloc for RequestedBytes {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() { added(layout.size()); }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() { added(layout.size()); }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout); }
        LIVE.fetch_sub(layout.size(), Relaxed);
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let updated = unsafe { System.realloc(pointer, layout, new_size) };
        if !updated.is_null() {
            if new_size >= layout.size() { added(new_size-layout.size()); }
            else { LIVE.fetch_sub(layout.size()-new_size, Relaxed); }
        }
        updated
    }
}

#[global_allocator]
static ALLOCATOR: RequestedBytes = RequestedBytes;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert_eq!(args.len(), 3, "FILE MODE PROFILE");
    let bytes = std::fs::read(&args[0]).unwrap();
    let policy = match args[1].as_str() {
        "decoded" => ExecutionPolicy::Decoded,
        "compact" => ExecutionPolicy::Compact,
        _ => panic!("unsupported retained product"),
    };
    let features = match args[2].as_str() {
        "mvp" => FeatureSet::mvp(),
        "stable-core" => FeatureSet::stable_core(),
        "all-phase3-core" => FeatureSet::all_phase3_core(),
        "mvp-saturating" | "simd" | "simd-relaxed" => {
            let mut features=FeatureSet::mvp();
            features.saturating_float_to_int=true;
            features.simd=args[2] != "mvp-saturating";
            features.relaxed_simd=args[2] == "simd-relaxed";
            features
        },
        _ => panic!("unsupported retained profile"),
    };
    let before = LIVE.load(Relaxed);
    PEAK.store(before, Relaxed);
    let engine = Engine::with_execution_policy(Config { features, ..Config::default() }, policy).unwrap();
    let engine_bytes = LIVE.load(Relaxed).checked_sub(before).unwrap();
    let module = engine.compile(&bytes).unwrap();
    let retained = LIVE.load(Relaxed).checked_sub(before).unwrap();
    let peak = PEAK.load(Relaxed).checked_sub(before).unwrap();
    let counts = module.compact_preparation_counts();
    let source = module.source_bytes();
    let control = module.lowered_control_bytes();
    let plans = module.function_plan_bytes();
    drop(module);
    drop(engine);
    let after = LIVE.load(Relaxed);
    assert_eq!(before, after, "compiled graph did not release its requested heap bytes");
    let (accepted, declined, _, prepared) = counts.unwrap_or((0,0,0,0));
    let work = counts.map(|value| value.2.to_string()).unwrap_or_else(|| "null".into());
    println!("{{\"schema\":\"wtime-retained-v1\",\"instrumented\":true,\"timing_evidence\":false,\"mode\":\"{}\",\"profile\":\"{}\",\"engine_empty_requested_bytes\":{},\"compiled_graph_requested_bytes\":{},\"peak_requested_bytes\":{},\"graph_released\":true,\"source_bytes\":{},\"control_capacity_bytes\":{},\"plan_budget_bytes\":{},\"compact_published\":{},\"compact_accepted\":{},\"compact_declined\":{},\"compact_known_work\":{},\"compact_artifact_budget_bytes\":{},\"compact_work_unknown\":{}}}",
        args[1], args[2], engine_bytes, retained, peak, source, control, plans,
        counts.is_some(), accepted, declined, work, prepared, counts.is_none() && policy==ExecutionPolicy::Compact);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p046_retained_tracks_real_allocation_and_release() {
        let before=LIVE.load(Relaxed);
        let allocation=Vec::<u8>::with_capacity(4096);
        assert_eq!(LIVE.load(Relaxed)-before, allocation.capacity());
        drop(allocation);
        assert_eq!(LIVE.load(Relaxed),before);
    }

    #[test]
    fn p046_retained_tracks_reallocation_growth_and_shrink() {
        let before=LIVE.load(Relaxed);
        let mut allocation=Vec::<u8>::with_capacity(128);
        allocation.push(7);
        allocation.reserve_exact(2048);
        assert_eq!(LIVE.load(Relaxed)-before,allocation.capacity());
        allocation.shrink_to_fit();
        assert_eq!(LIVE.load(Relaxed)-before,allocation.capacity());
        assert_eq!(allocation.as_slice(), &[7]);
        drop(allocation);
        assert_eq!(LIVE.load(Relaxed),before);
    }

    #[test]
    fn p046_retained_compiled_graph_releases_in_both_policies() {
        const WASM: &[u8]=b"\0asm\x01\0\0\0\x01\x05\x01\x60\0\x01\x7f\x03\x02\x01\0\x07\x07\x01\x03run\0\0\x0a\x06\x01\x04\0\x41\x07\x0b";
        for policy in [ExecutionPolicy::Decoded,ExecutionPolicy::Compact] {
            let before=LIVE.load(Relaxed);
            let engine=Engine::with_execution_policy(Config::default(),policy).unwrap();
            let module=engine.compile(WASM).unwrap();
            assert!(LIVE.load(Relaxed)>before);
            if policy==ExecutionPolicy::Compact {
                let counts=module.compact_preparation_counts().unwrap();
                assert_eq!(counts.0,1);
                assert!(counts.3>0);
            }
            drop(module);
            drop(engine);
            assert_eq!(LIVE.load(Relaxed),before);
        }
    }
}
