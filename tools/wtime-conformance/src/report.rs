#[cfg(feature = "performance-counters")]
use crate::adapter::PreparationCounts;

#[derive(Default, Debug)]
pub struct Report {
    pub execution_mode: String,
    pub selected: usize,
    pub executed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub unsupported: usize,
    pub text_errors: usize,
    pub errors: Vec<String>,
    pub compatibility: Vec<String>,
    pub compatibility_assertions: usize,
    pub executed_opcodes: Option<Vec<u16>>,
    pub binary_sha256: String,
    pub execution_counts: Option<ExecutionCounts>,
    pub preparation_summary: Option<PreparationSummary>,
    pub worker_stores_observed: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExecutionCounts {
    pub total_dispatched_source_events: u64,
    pub optimized_compact_source_events: u64,
    pub decoded_fallback_source_events: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreparationSummary {
    pub compile_count: usize,
    pub published_artifacts: usize,
    pub accepted: usize,
    pub declined: usize,
    pub attempted_work_known_sum: usize,
    pub unpublished_artifacts: usize,
    pub attempted_work_unknown_artifacts: usize,
    pub artifact_retained_budget_bytes: usize,
    pub peak_artifact_retained_budget_bytes: usize,
}

pub fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl Report {
    pub fn ok(&self) -> bool {
        self.selected > 0
            && self.executed > 0
            && self.failed == 0
            && self.unsupported == 0
            && self.skipped == 0
    }

    pub fn collector_error(&mut self, message: impl Into<String>) {
        self.failed = self.failed.saturating_add(1);
        self.errors.push(message.into());
    }

    pub fn merge_execution_counts(&mut self, delta: ExecutionCounts) {
        let Some(current) = self.execution_counts else {
            self.execution_counts = Some(delta);
            return;
        };
        let Some(total) = current
            .total_dispatched_source_events
            .checked_add(delta.total_dispatched_source_events)
        else {
            self.collector_error("execution counter overflow: total dispatched source events");
            return;
        };
        let Some(optimized) = current
            .optimized_compact_source_events
            .checked_add(delta.optimized_compact_source_events)
        else {
            self.collector_error("execution counter overflow: optimized Compact source events");
            return;
        };
        let Some(fallback) = current
            .decoded_fallback_source_events
            .checked_add(delta.decoded_fallback_source_events)
        else {
            self.collector_error("execution counter overflow: decoded fallback source events");
            return;
        };
        self.execution_counts = Some(ExecutionCounts {
            total_dispatched_source_events: total,
            optimized_compact_source_events: optimized,
            decoded_fallback_source_events: fallback,
        });
    }

    pub fn merge_preparation_counts(&mut self, delta: PreparationSummary) {
        let current = self.preparation_summary.unwrap_or_default();
        let mut next = current;
        macro_rules! checked_sum {
            ($field:ident) => {
                if let Some(value) = next.$field.checked_add(delta.$field) {
                    next.$field = value;
                } else {
                    self.collector_error(concat!(
                        "preparation counter overflow: ",
                        stringify!($field)
                    ));
                    return;
                }
            };
        }
        checked_sum!(compile_count);
        checked_sum!(published_artifacts);
        checked_sum!(accepted);
        checked_sum!(declined);
        checked_sum!(attempted_work_known_sum);
        checked_sum!(unpublished_artifacts);
        checked_sum!(attempted_work_unknown_artifacts);
        checked_sum!(artifact_retained_budget_bytes);
        next.peak_artifact_retained_budget_bytes = next
            .peak_artifact_retained_budget_bytes
            .max(delta.peak_artifact_retained_budget_bytes);
        self.preparation_summary = Some(next);
    }

    pub fn merge_worker_stores(&mut self, delta: usize) {
        if let Some(total) = self.worker_stores_observed.checked_add(delta) {
            self.worker_stores_observed = total;
        } else {
            self.collector_error("worker Store counter overflow");
        }
    }

    pub fn json(
        &self,
        profile: &str,
        suite: &str,
        hash: &str,
        revision: &str,
        replay: &str,
    ) -> String {
        let execution_counts = self.execution_counts.map(|c| format!(
            "{{\"total_dispatched_source_events\":{},\"optimized_compact_source_events\":{},\"decoded_fallback_source_events\":{}}}",
            c.total_dispatched_source_events, c.optimized_compact_source_events, c.decoded_fallback_source_events
        )).unwrap_or_else(|| "null".into());
        let preparation = self.preparation_summary.map(|p| format!(
            "{{\"compile_count\":{},\"published_artifacts\":{},\"accepted\":{},\"declined\":{},\"attempted_work_known_sum\":{},\"unpublished_artifacts\":{},\"attempted_work_unknown_artifacts\":{},\"artifact_retained_budget_bytes\":{},\"artifact_retained_budget_bytes_basis\":\"conservative capacity plus headers, not allocator physical bytes\",\"peak_artifact_retained_budget_bytes\":{}}}",
            p.compile_count, p.published_artifacts, p.accepted, p.declined, p.attempted_work_known_sum,
            p.unpublished_artifacts, p.attempted_work_unknown_artifacts, p.artifact_retained_budget_bytes,
            p.peak_artifact_retained_budget_bytes
        )).unwrap_or_else(|| "null".into());
        format!(
            "{{\"profile\":{},\"execution_mode\":{},\"instrumented\":{},\"suite\":{},\"source_hash\":{},\"engine_revision\":{},\"text_tool\":\"wast 261.0.0\",\"text_tool_errors\":{},\"resource_limits\":{{\"fuel_per_invoke\":10000000,\"max_call_depth\":512,\"max_memory_pages\":18446744073709551615,\"max_memory_bytes\":67108864,\"worker_watchdog_ms\":30000}},\"binary_sha256\":{},\"implementation_sources\":{},\"executed_opcodes\":{},\"compatibility\":[{}],\"compatibility_assertions\":{},\"selected\":{},\"executed\":{},\"failed\":{},\"skipped\":{},\"unsupported\":{},\"replay\":{},\"errors\":[{}],\"execution_counts\":{},\"preparation_summary\":{},\"worker_stores_observed\":{}}}\n",
            quote(profile),
            quote(if self.execution_mode.is_empty() {
                "decoded"
            } else {
                &self.execution_mode
            }),
            cfg!(any(feature = "opcode-coverage", feature = "performance-counters")),
            quote(suite),
            quote(hash),
            quote(revision),
            self.text_errors,
            quote(&self.binary_sha256),
            env!("WTIME_IMPLEMENTATION_SOURCES"),
            self.executed_opcodes
                .as_ref()
                .map(|v| format!(
                    "[{}]",
                    v.iter().map(u16::to_string).collect::<Vec<_>>().join(",")
                ))
                .unwrap_or_else(|| "null".into()),
            self.compatibility
                .iter()
                .map(|s| quote(s))
                .collect::<Vec<_>>()
                .join(","),
            self.compatibility_assertions,
            self.selected,
            self.executed,
            self.failed,
            self.skipped,
            self.unsupported,
            quote(replay),
            self.errors
                .iter()
                .map(|s| quote(s))
                .collect::<Vec<_>>()
                .join(","),
            execution_counts,
            preparation,
            self.worker_stores_observed
        )
    }
}

#[cfg(feature = "performance-counters")]
impl From<PreparationCounts> for PreparationSummary {
    fn from(value: PreparationCounts) -> Self {
        Self {
            compile_count: value.compile_count,
            published_artifacts: value.published_artifacts,
            accepted: value.accepted,
            declined: value.declined,
            attempted_work_known_sum: value.attempted_work_known_sum,
            unpublished_artifacts: value.unpublished_artifacts,
            attempted_work_unknown_artifacts: value.attempted_work_unknown_artifacts,
            artifact_retained_budget_bytes: value.artifact_retained_budget_bytes,
            peak_artifact_retained_budget_bytes: value.peak_artifact_retained_budget_bytes,
        }
    }
}
