fn p138_emit(case: &str, event: &str, snapshot: &wtime_runtime::RegisterCacheSnapshot) {
    use std::fmt::Write;
    use wtime_runtime::RegisterCacheEntryState as State;
    let mut rows = String::new();
    for (index, entry) in snapshot.entries.iter().enumerate() {
        if index != 0 {
            rows.push(',');
        }
        let (state, cooldown, bytes) = match entry.state {
            State::Cold => ("cold", 0, 0),
            State::Compiling => ("compiling", 0, 0),
            State::Ready { retained_bytes } => ("ready", 0, retained_bytes),
            State::Cooldown { remaining } => ("cooldown", remaining, 0),
            State::Negative => ("negative", 0, 0),
        };
        write!(rows, "{{\"body\":{},\"entries\":{},\"attempts\":{},\"state\":\"{}\",\"cooldown\":{},\"ready_bytes\":{}}}",
               entry.body, entry.entries, entry.attempts, state, cooldown, bytes).unwrap();
    }
    let active = snapshot
        .active_body
        .map_or_else(|| "null".into(), |body| body.to_string());
    let m = snapshot.lifecycle;
    eprintln!(
        "P138_TRACE {{\"case\":\"{case}\",\"event\":\"{event}\",\"instrumented\":true,\"timing_evidence\":false,\"engine\":{},\"threshold\":{},\"entry_limit\":{},\"active_body\":{},\"remaining_attempted_work\":{},\"remaining_retained_work\":{},\"retained_bytes\":{},\"byte_limit\":{},\"lifecycle\":{{\"preparation_starts\":{},\"builder_starts\":{},\"completed\":{},\"published\":{},\"publication_failures\":{},\"declined\":{},\"exhausted\":{},\"invalid\":{},\"aborted\":{},\"evictions\":{}}},\"rows\":[{}]}}",
        snapshot.engine,
        snapshot.threshold,
        snapshot.entry_limit,
        active,
        snapshot.remaining_attempted_work,
        snapshot.remaining_retained_work,
        snapshot.retained_bytes,
        snapshot.byte_limit,
        m.preparation_starts,
        m.builder_starts,
        m.completed,
        m.published,
        m.publication_failures,
        m.declined,
        m.exhausted,
        m.invalid,
        m.aborted,
        m.evictions,
        rows
    );
}
