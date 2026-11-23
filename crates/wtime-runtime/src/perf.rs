//! Bounded Store-local diagnostic events, compiled out of production builds.
//! Byte counters count explicitly copied payloads, not allocator-wide traffic.
use crate::Immediate;
use std::cell::Cell;
/// Point-in-time snapshot of Store-local diagnostic performance counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CounterSnapshot {
    /// Register-tier bytecode steps executed.
    pub register_steps: u64,
    /// Frames that fell back from Register execution to decoded execution.
    pub register_fallback_frames: u64,
    /// Source-level WebAssembly instructions dispatched.
    pub source_dispatches: u64,
    /// Compact-tier bytecode steps executed.
    pub compact_steps: u64,
    /// Direct calls executed through Compact pre-bound call targets.
    pub compact_bound_calls: u64,
    /// Shared linear memory accesses handled by the Compact executor.
    pub compact_shared_memory_accesses: u64,
    /// Linear memory accesses handled by the Compact executor.
    pub compact_memory_accesses: u64,
    /// Indirect-call inline cache hits in the Compact executor.
    pub compact_indirect_hits: u64,
    /// Indirect-call inline cache misses in the Compact executor.
    pub compact_indirect_misses: u64,
    /// Indirect-call inline cache evictions in the Compact executor.
    pub compact_indirect_evictions: u64,
    /// Frames that fell back from Compact execution to decoded execution.
    pub compact_fallback_frames: u64,
    /// Result value moves performed on Compact frame boundaries.
    pub compact_result_moves: u64,
    /// Deep-copy events of compiled module payloads.
    pub module_deep_copy_events: u64,
    /// Bytes copied during module deep-copy events.
    pub module_deep_copy_bytes: u64,
    /// Instruction clone events during execution.
    pub instruction_clone_events: u64,
    /// Immediate payload bytes copied by instruction clones.
    pub instruction_clone_bytes: u64,
    /// Allocating linear memory read operations (`Vec<u8>`).
    pub read_vec_operations: u64,
    /// Fixed-buffer non-allocating linear memory read operations.
    pub read_fixed_operations: u64,
    /// Guest call frames entered.
    pub frame_entries: u64,
    /// Heap allocations performed for call argument vectors.
    pub argument_allocations: u64,
    /// Heap allocations performed when snapshotting active GC roots.
    pub root_snapshot_allocations: u64,
    /// Runtime subtype checks performed.
    pub subtype_probes: u64,
    /// Continuation table entries scanned when accounting continuation bytes.
    pub continuation_scan_entries: u64,
    /// Heap allocations performed for branch result vectors.
    pub branch_result_allocations: u64,
    /// Chunk snapshots taken during overlapping `table.copy` operations.
    pub table_copy_snapshot_chunks: u64,
    /// Slice chunks processed during bulk memory operations.
    pub bulk_slice_chunks: u64,
}
impl CounterSnapshot {
    /// JSON for the diagnostic harness; no instrumentation is present by default.
    pub fn to_json(self) -> String {
        format!(
            concat!(
                "{{\"register_steps\":{},\"register_fallback_frames\":{},",
                "\"compact_indirect_hits\":{},\"compact_indirect_misses\":{},\"compact_indirect_evictions\":{},\"compact_memory_accesses\":{},\"compact_shared_memory_accesses\":{},\"compact_bound_calls\":{},\"compact_steps\":{},\"compact_fallback_frames\":{},\"compact_result_moves\":{},\"source_dispatches\":{},\"module_deep_copy_events\":{},",
                "\"module_deep_copy_bytes\":{},\"instruction_clone_events\":{},\"instruction_clone_bytes\":{},",
                "\"read_vec_operations\":{},\"read_fixed_operations\":{},\"frame_entries\":{},",
                "\"argument_allocations\":{},\"root_snapshot_allocations\":{},\"subtype_probes\":{},",
                "\"continuation_scan_entries\":{},\"branch_result_allocations\":{},",
                "\"table_copy_snapshot_chunks\":{},\"bulk_slice_chunks\":{}}}"
            ),
            self.register_steps,
            self.register_fallback_frames,
            self.compact_indirect_hits,
            self.compact_indirect_misses,
            self.compact_indirect_evictions,
            self.compact_memory_accesses,
            self.compact_shared_memory_accesses,
            self.compact_bound_calls,
            self.compact_steps,
            self.compact_fallback_frames,
            self.compact_result_moves,
            self.source_dispatches,
            self.module_deep_copy_events,
            self.module_deep_copy_bytes,
            self.instruction_clone_events,
            self.instruction_clone_bytes,
            self.read_vec_operations,
            self.read_fixed_operations,
            self.frame_entries,
            self.argument_allocations,
            self.root_snapshot_allocations,
            self.subtype_probes,
            self.continuation_scan_entries,
            self.branch_result_allocations,
            self.table_copy_snapshot_chunks,
            self.bulk_slice_chunks
        )
    }
}
#[derive(Clone, Copy)]
pub(super) enum Event {
    #[allow(dead_code)]
    RegisterStep,
    #[allow(dead_code)]
    RegisterFallbackFrame,
    Dispatch,
    #[allow(dead_code)]
    CompactStep,
    #[allow(dead_code)]
    CompactBoundCall,
    #[allow(dead_code)]
    CompactSharedMemoryAccess,
    #[allow(dead_code)]
    CompactMemoryAccess,
    #[allow(dead_code)]
    CompactIndirectEviction,
    #[allow(dead_code)]
    CompactIndirectMiss,
    #[allow(dead_code)]
    CompactIndirectHit,
    #[allow(dead_code)]
    CompactFallbackFrame,
    #[allow(dead_code)]
    CompactResultMove,
    #[allow(dead_code)]
    ModuleCopy,
    #[allow(dead_code)]
    ModuleBytes,
    InstructionClone,
    InstructionBytes,
    ReadVec,
    #[allow(dead_code)]
    ReadFixed,
    Frame,
    Arguments,
    Roots,
    Subtype,
    ContinuationScan,
    Branch,
    #[allow(dead_code)]
    TableCopySnapshotChunk,
    #[allow(dead_code)]
    BulkSliceChunk,
}
#[derive(Default)]
pub(super) struct Counters(Cell<CounterSnapshot>);
impl Counters {
    pub(super) fn snapshot(&self) -> CounterSnapshot {
        self.0.get()
    }
    pub(super) fn reset(&self) {
        self.0.set(CounterSnapshot::default());
    }
    pub(super) fn record(&self, event: Event, n: u64) {
        let mut s = self.0.get();
        let field = match event {
            Event::RegisterStep => &mut s.register_steps,
            Event::RegisterFallbackFrame => &mut s.register_fallback_frames,
            Event::Dispatch => &mut s.source_dispatches,
            Event::CompactStep => &mut s.compact_steps,
            Event::CompactBoundCall => &mut s.compact_bound_calls,
            Event::CompactSharedMemoryAccess => &mut s.compact_shared_memory_accesses,
            Event::CompactMemoryAccess => &mut s.compact_memory_accesses,
            Event::CompactIndirectEviction => &mut s.compact_indirect_evictions,
            Event::CompactIndirectMiss => &mut s.compact_indirect_misses,
            Event::CompactIndirectHit => &mut s.compact_indirect_hits,
            Event::CompactFallbackFrame => &mut s.compact_fallback_frames,
            Event::CompactResultMove => &mut s.compact_result_moves,
            Event::ModuleCopy => &mut s.module_deep_copy_events,
            Event::ModuleBytes => &mut s.module_deep_copy_bytes,
            Event::InstructionClone => &mut s.instruction_clone_events,
            Event::InstructionBytes => &mut s.instruction_clone_bytes,
            Event::ReadVec => &mut s.read_vec_operations,
            Event::ReadFixed => &mut s.read_fixed_operations,
            Event::Frame => &mut s.frame_entries,
            Event::Arguments => &mut s.argument_allocations,
            Event::Roots => &mut s.root_snapshot_allocations,
            Event::Subtype => &mut s.subtype_probes,
            Event::ContinuationScan => &mut s.continuation_scan_entries,
            Event::Branch => &mut s.branch_result_allocations,
            Event::TableCopySnapshotChunk => &mut s.table_copy_snapshot_chunks,
            Event::BulkSliceChunk => &mut s.bulk_slice_chunks,
        };
        *field = field.saturating_add(n);
        self.0.set(s);
    }
}
/// Heap bytes a clone of the immediate would copy. Since instructions became
/// 32 bytes (99e785a), `try_table` and `resume` count their boxed payload
/// struct as well, so their values are not comparable with earlier records.
pub(super) fn immediate_copy_bytes(i: &Immediate) -> u64 {
    let bytes = match i {
        Immediate::BrTable(v, _) => v.len().saturating_mul(std::mem::size_of::<u32>()),
        Immediate::Types(v) => v
            .len()
            .saturating_mul(std::mem::size_of::<crate::ValType>()),
        Immediate::Bytes(v) => v.len(),
        Immediate::TryTable(table) => std::mem::size_of::<wtime_validate::TryTableImmediate>()
            .saturating_add(
                table
                    .catches
                    .len()
                    .saturating_mul(std::mem::size_of::<wtime_validate::Catch>()),
            )
            .saturating_add(immediate_copy_bytes(&table.block) as usize),
        Immediate::Resume(resume) => std::mem::size_of::<wtime_validate::ResumeImmediate>()
            .saturating_add(
                resume
                    .handlers
                    .len()
                    .saturating_mul(std::mem::size_of::<(u32, Option<u32>)>()),
            ),
        _ => 0,
    };
    u64::try_from(bytes).unwrap_or(u64::MAX)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    fn run(bytes: &[u8]) -> CounterSnapshot {
        let e = Engine::new(Config::default()).unwrap();
        let m = e.compile(bytes).unwrap();
        let mut s = Store::new(&e);
        let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
        let External::Func(f) = i.export(&s, "run").unwrap() else {
            panic!()
        };
        s.reset_performance_counters();
        let out = s.invoke(f, &[Value::I32(0)]).unwrap();
        assert!(matches!(out, RunOutcome::Returned(_)));
        s.performance_counters()
    }
    const HEADER: &[u8] = b"\0asm\x01\0\0\0\x01\x06\x01\x60\x01\x7f\x01\x7f\x03\x02\x01\0";
    const EXPORT: &[u8] = b"\x07\x07\x01\x03run\0\0";
    #[test]
    fn p006_source_dispatch_count_and_no_memory_event() {
        let mut b = HEADER.to_vec();
        b.extend(EXPORT);
        b.extend(b"\x0a\x09\x01\x07\0\x20\0\x41\x01\x6a\x0b");
        let s = run(&b);
        assert_eq!(s.source_dispatches, 4);
        assert_eq!(s.module_deep_copy_events, 0);
        assert_eq!(s.module_deep_copy_bytes, 0);
        assert_eq!(s.instruction_clone_events, 4);
        assert_eq!(s.read_vec_operations, 0);
        assert_eq!(s.frame_entries, 1);
    }
    #[test]
    fn p006_load_records_one_allocating_read() {
        let mut b = HEADER.to_vec();
        b.extend(b"\x05\x03\x01\0\x01");
        b.extend(EXPORT);
        b.extend(b"\x0a\x09\x01\x07\0\x20\0\x28\x02\0\x0b");
        let s = run(&b);
        assert_eq!(s.source_dispatches, 3);
        assert_eq!(s.read_vec_operations, 1);
        assert_eq!(s.read_fixed_operations, 0);
    }
    #[test]
    fn p006_saturation_and_store_isolation() {
        let c = Counters::default();
        c.record(Event::Dispatch, u64::MAX);
        c.record(Event::Dispatch, 1);
        assert_eq!(c.snapshot().source_dispatches, u64::MAX);
        assert_eq!(Counters::default().snapshot(), CounterSnapshot::default());
        c.reset();
        assert_eq!(c.snapshot(), CounterSnapshot::default());
    }
}
