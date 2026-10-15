//! One capture with a declared number of independently rooted live segments.
use std::{hint::black_box, time::Instant};
use wtime_runtime::{Config, External, Linker, RunOutcome, Store, Value};
use wtime_types::FeatureSet;

const COUNTS: [usize; 4] = [1, 8, 73, 512];

fn parse(args: &[String]) -> Result<(usize, &str), &'static str> {
    if args.len() != 7 || args[0] != "--continuation-live-set" {
        return Err("expected FILE CASE COUNT STAGE MODE PROFILE");
    }
    let count = args[3].parse::<usize>().map_err(|_| "invalid live count")?;
    if !COUNTS.contains(&count)
        || args[2] != format!("continuation_live_{count}")
        || !matches!(args[4].as_str(), "setup" | "capture")
        || args[5] != "decoded"
        || args[6] != "all-phase3-core"
    {
        return Err("unsupported live continuation request");
    }
    Ok((count, &args[4]))
}

struct Observation {
    setup_ns: u128,
    capture_ns: u128,
    payloads: Vec<String>,
    results: Vec<String>,
    bytes_before: usize,
    bytes_after_capture: usize,
    bytes_after_drop: usize,
    gc_before: usize,
    gc_peak: usize,
    gc_after: usize,
}

fn execute(bytes: &[u8], count: usize) -> Observation {
    assert!(COUNTS.contains(&count));
    let start = Instant::now();
    let engine = super::configured_engine(
        Config {
            features: FeatureSet::all_phase3_core(),
            ..Config::default()
        },
        "decoded",
    );
    let module = engine
        .compile(bytes)
        .expect("live continuation compile failed");
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let func = |name| match instance.export(&store, name).unwrap() {
        External::Func(value) => value,
        _ => panic!("continuation function export is missing"),
    };
    let run = func("run");
    let resume = func("resume");
    let mut roots = Vec::with_capacity(count + 1);
    let mut payloads = Vec::with_capacity(count + 1);
    let mut retain = |store: &mut Store, outcome| {
        let RunOutcome::Suspended {
            payload,
            continuation,
            ..
        } = outcome
        else {
            panic!("live continuation run did not suspend")
        };
        assert_eq!(payload, [Value::I32(7)], "live suspension oracle mismatch");
        payloads.push("i32:7".to_owned());
        let root = store.root(continuation).unwrap();
        store.release_published(&payload);
        store.release_published(&[Value::Ref(wtime_runtime::RefValue::Gc(continuation))]);
        root
    };
    for _ in 0..count {
        let outcome = store.invoke(run, &[]).expect("live set population failed");
        roots.push(retain(&mut store, outcome));
    }
    // Remove unreachable consumed Start headers before measuring admission into
    // a registry containing exactly the declared rooted suspended segments.
    let gc_before = store.collect().unwrap().live;
    assert_eq!(gc_before, count);
    let setup_ns = start.elapsed().as_nanos().max(1);
    let bytes_before = store.continuation_bytes();
    assert!(bytes_before > 0);

    let capture_start = Instant::now();
    let outcome = store
        .invoke(run, black_box(&[]))
        .expect("measured capture failed");
    let capture_ns = capture_start.elapsed().as_nanos().max(1);

    // Oracles, root transfer, byte queries, GC and resumption are outside capture.
    roots.push(retain(&mut store, outcome));
    let bytes_after_capture = store.continuation_bytes();
    assert!(bytes_after_capture > bytes_before);
    let gc_peak = store.collect().unwrap().live;
    assert_eq!(gc_peak, count + 1);
    let mut results = Vec::with_capacity(count + 1);
    for root in &roots {
        let outcome = store
            .invoke(
                resume,
                &[Value::Ref(wtime_runtime::RefValue::Gc(root.reference()))],
            )
            .unwrap();
        let RunOutcome::Returned(values) = outcome else {
            panic!("live resume did not return")
        };
        assert_eq!(values, [Value::I32(1009)], "live resume oracle mismatch");
        results.push("i32:1009".to_owned());
        store.release_published(&values);
    }
    drop(roots);
    let gc_after = store.collect().unwrap().live;
    let bytes_after_drop = store.continuation_bytes();
    assert_eq!((gc_after, bytes_after_drop), (0, 0));
    Observation {
        setup_ns,
        capture_ns,
        payloads,
        results,
        bytes_before,
        bytes_after_capture,
        bytes_after_drop,
        gc_before,
        gc_peak,
        gc_after,
    }
}

fn strings(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|v| super::quote(v))
            .collect::<Vec<_>>()
            .join(",")
    )
}

pub fn try_main(args: &[String]) -> bool {
    if args.first().map(String::as_str) != Some("--continuation-live-set") {
        return false;
    }
    let (count, stage) = parse(args).expect("invalid live continuation request");
    assert!(
        !cfg!(feature = "performance-counters") && !cfg!(feature = "execution-reference"),
        "live continuation timing requires a production build"
    );
    let metadata = std::fs::metadata(&args[1]).expect("cannot stat live continuation fixture");
    assert!(metadata.len() <= 4 * 1024 * 1024);
    let bytes = std::fs::read(&args[1]).expect("cannot read live continuation fixture");
    let o = execute(&bytes, count);
    println!(
        concat!(
            "{{\"schema\":\"wtime-continuation-live-v1\",\"schema_version\":1,",
            "\"case\":{},\"stage\":{},\"live_count\":{count},\"mode\":\"decoded\",",
            "\"profile\":\"all-phase3-core\",\"batch\":1,\"instrumented\":false,\"report_complete\":true,",
            "\"setup_ns\":{},\"capture_ns\":{},\"payloads\":{},\"results\":{},",
            "\"roots_before\":{count},\"roots_peak\":{},\"gc_live_before\":{},\"gc_live_peak\":{},",
            "\"gc_live_after_drop\":{},\"bytes_before\":{},\"bytes_after_capture\":{},\"bytes_after_drop\":{}}}"
        ),
        super::quote(&args[2]),
        super::quote(stage),
        o.setup_ns,
        o.capture_ns,
        strings(&o.payloads),
        strings(&o.results),
        count + 1,
        o.gc_before,
        o.gc_peak,
        o.gc_after,
        o.bytes_before,
        o.bytes_after_capture,
        o.bytes_after_drop,
        count = count
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &[u8] = include_bytes!("../fixtures/continuations/depth_0.wasm");

    #[test]
    fn p072_live_count_and_cleanup_are_checked() {
        for count in COUNTS {
            let o = execute(FIXTURE, count);
            assert_eq!((o.payloads.len(), o.results.len()), (count + 1, count + 1));
            assert_eq!(
                (o.gc_before, o.gc_peak, o.gc_after, o.bytes_after_drop),
                (count, count + 1, 0, 0)
            );
            assert!(o.setup_ns > 0 && o.capture_ns > 0);
        }
    }

    #[test]
    fn p072_live_oracles_reject_corrupted_guest_results() {
        for (pattern, replacement) in [
            (&[0x41, 7][..], &[0x41, 8][..]),
            (&[0x41, 0xe8, 7][..], &[0x41, 0xe9, 7][..]),
        ] {
            let positions = FIXTURE
                .windows(pattern.len())
                .enumerate()
                .filter_map(|(i, bytes)| (bytes == pattern).then_some(i))
                .collect::<Vec<_>>();
            assert_eq!(positions.len(), 1);
            let mut bad = FIXTURE.to_vec();
            bad[positions[0]..positions[0] + pattern.len()].copy_from_slice(replacement);
            assert!(std::panic::catch_unwind(|| execute(&bad, 1)).is_err());
        }
    }

    #[test]
    fn p072_live_parser_bounds_and_policy_are_checked() {
        let good = [
            "--continuation-live-set",
            "nonexistent.wasm",
            "continuation_live_8",
            "8",
            "capture",
            "decoded",
            "all-phase3-core",
        ]
        .map(str::to_owned)
        .to_vec();
        assert_eq!(parse(&good).unwrap(), (8, "capture"));
        for (index, value) in [
            (3, "0"),
            (3, "9"),
            (3, "18446744073709551616"),
            (2, "continuation_live_73"),
            (4, "warm_call"),
            (5, "register"),
            (6, "mvp"),
        ] {
            let mut args = good.clone();
            args[index] = value.to_owned();
            assert!(parse(&args).is_err());
        }
        assert!(parse(&good[..6]).is_err());
    }
}
