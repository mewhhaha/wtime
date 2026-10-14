//! P009 public-API memory observation driver.
//!
//! Routed from protocol.rs as `--memory-driver FILE CASE INPUT
//! EXPECTED_OUTCOME MODE PROFILE DRIVER`. It uses the public runtime API only.

use std::{hint::black_box, time::Instant};
use wtime_runtime::{ErrorKind, External, Linker, RunOutcome, Store, Value};
use wtime_types::{IndexType, Limits, MemoryType};

const PAGE_BYTES: usize = 65_536;
const MAX_PAGES: u64 = 17;
const MAX_READBACK_BYTES: usize = PAGE_BYTES * MAX_PAGES as usize;
const HEX: &[u8; 16] = b"0123456789abcdef";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Driver {
    Plain,
    Alias,
    Multi,
    SeededCopy,
    SeededFill,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Expected {
    I32(i32),
    Trap,
}

#[derive(Clone, Debug)]
enum Observed {
    I32(i32),
    Trap { kind: String, message: String },
}

impl Expected {
    fn parse(text: &str) -> Self {
        if text == "trap" {
            return Self::Trap;
        }
        let Some(value) = text.strip_prefix("i32:") else {
            panic!("expected outcome must be i32:N or trap")
        };
        Self::I32(value.parse().expect("invalid expected i32"))
    }

    fn check(&self, got: &Observed) {
        match (self, got) {
            (Self::I32(expected), Observed::I32(actual)) if expected == actual => {}
            (Self::Trap, Observed::Trap { .. }) => {}
            _ => panic!("memory outcome mismatch: expected {self:?}, got {got:?}"),
        }
    }

    fn text(&self) -> String {
        match self {
            Self::I32(value) => format!("i32:{value}"),
            Self::Trap => "trap".to_owned(),
        }
    }
}

fn is_width_alias(case: &str) -> bool {
    let Some((family, suffix)) = case.rsplit_once('_') else {
        return false;
    };
    if suffix.is_empty()
        || !suffix.bytes().all(|byte| byte.is_ascii_digit())
        || (suffix.len() > 1 && suffix.starts_with('0'))
        || suffix.parse::<u32>().is_err()
    {
        return false;
    }
    matches!(
        family,
        "memory_widths_predictable" | "memory_widths_shuffled" | "memory_widths_read_scan"
    )
}

fn is_repeatable_memory_case(case: &str) -> bool {
    matches!(
        case,
        "memory_widths_predictable" | "memory_widths_shuffled" | "memory_widths_read_scan"
    ) || is_width_alias(case)
        || case.starts_with("memory_copy_distinct_seeded_")
        || case.starts_with("memory_fill_seeded_")
}

#[cfg(test)]
mod repeatable_memory_case_tests {
    use super::is_repeatable_memory_case;

    #[test]
    fn accepts_width_aliases_and_preserves_existing_cases() {
        for case in [
            "memory_widths_predictable_0",
            "memory_widths_shuffled_17",
            "memory_widths_read_scan_1024",
            "memory_widths_predictable",
            "memory_widths_shuffled",
            "memory_widths_read_scan",
            "memory_copy_distinct_seeded_64",
            "memory_fill_seeded_4096",
        ] {
            assert!(is_repeatable_memory_case(case), "{case}");
        }
    }

    #[test]
    fn rejects_unknown_or_malformed_width_aliases() {
        for case in [
            "memory_widths_other_32",
            "memory_widths_predictable_x",
            "memory_widths_shuffled_01",
            "memory_widths_read_scan_4294967296",
        ] {
            assert!(!is_repeatable_memory_case(case), "{case}");
        }
    }
}

fn observed(store: &mut Store, func: wtime_runtime::Func, input: i32) -> Observed {
    match store.invoke(func, black_box(&[Value::I32(input)])) {
        Ok(RunOutcome::Returned(values)) => match values.as_slice() {
            [Value::I32(value)] => Observed::I32(*value),
            other => panic!("memory driver expected exactly one i32 result, got {other:?}"),
        },
        Ok(other) => panic!("unexpected memory workload outcome {other:?}"),
        Err(error) if error.kind == ErrorKind::Trap => Observed::Trap {
            kind: format!("{:?}", error.trap_code),
            message: error.message,
        },
        Err(error) => panic!("memory workload failed outside trap semantics: {error}"),
    }
}

fn seeded_bytes(len: usize, multiplier: u8, addend: u8, page_multiplier: u8) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(len)
        .expect("bounded memory seed allocation failed");
    bytes.extend((0..len).map(|index| {
        (index as u8)
            .wrapping_mul(multiplier)
            .wrapping_add(addend)
            .wrapping_add(((index >> 16) as u8).wrapping_mul(page_multiplier))
    }));
    bytes
}

fn memory_type() -> MemoryType {
    MemoryType {
        limits: Limits::new(MAX_PAGES, Some(MAX_PAGES)),
        index_type: IndexType::I32,
        shared: false,
        page_size_log2: 16,
    }
}

struct Actual {
    elapsed_ns: f64,
    batch: usize,
    outcome: Observed,
    pages: u64,
    page_size_log2: u8,
    memory_hex: String,
    all_memory_hex: Vec<String>,
    memory_count: usize,
    exported_memory_index: usize,
    host_setup_ns: Option<f64>,
    seeded_policy: bool,
}

fn execute_once(
    bytes: &[u8],
    input: i32,
    mode: &str,
    profile: &str,
    driver: Driver,
    stage: &str,
    requested_batch: usize,
) -> Actual {
    // Engine initialization is intentionally outside the load_and_call timer.
    // Unsupported selectors fail through the public Engine policy constructor.
    let engine = super::typed_engine(mode, profile);
    let start = Instant::now();
    let module = engine.compile(bytes).expect("memory module compile failed");
    let prepare_ns = start.elapsed().as_nanos().max(1) as f64;
    let instantiate_start = Instant::now();
    let mut store = Store::new(&engine);
    let mut linker = Linker::new(&engine);
    let alias_memory = if driver == Driver::Alias {
        let memory = store
            .create_memory(memory_type())
            .expect("alias memory creation failed");
        linker
            .define("mem", "left", External::Memory(memory))
            .expect("left memory import definition failed");
        linker
            .define("mem", "right", External::Memory(memory))
            .expect("right memory import definition failed");
        Some(memory)
    } else {
        None
    };
    let instance = linker
        .instantiate(&mut store, &module)
        .expect("memory module instantiation failed");
    let func = match instance.export(&store, "run").expect("missing run export") {
        External::Func(func) => func,
        other => panic!("run export is not a function: {other:?}"),
    };
    let instantiate_ns = instantiate_start.elapsed().as_nanos().max(1) as f64;
    // Seeded P027 workloads initialize complete, distinct memories through the
    // public host API after instantiation and before any timed guest call.
    let host_setup_ns = match driver {
        Driver::SeededCopy => {
            let setup_start = Instant::now();
            let source = exported_memory(&instance, &store, "source");
            let destination = exported_memory(&instance, &store, "memory");
            let source_bytes = seeded_bytes(16 * PAGE_BYTES, 29, 7, 53);
            let destination_bytes = seeded_bytes(17 * PAGE_BYTES, 13, 101, 37);
            store
                .write_memory(source, 0, &source_bytes)
                .expect("source seed write failed");
            store
                .write_memory(destination, 0, &destination_bytes)
                .expect("destination seed write failed");
            Some(setup_start.elapsed().as_nanos().max(1) as f64)
        }
        Driver::SeededFill => {
            let setup_start = Instant::now();
            let memory = exported_memory(&instance, &store, "memory");
            let initial = seeded_bytes(17 * PAGE_BYTES, 13, 101, 37);
            store
                .write_memory(memory, 0, &initial)
                .expect("fill seed write failed");
            Some(setup_start.elapsed().as_nanos().max(1) as f64)
        }
        _ => None,
    };
    let first_start = Instant::now();
    let mut outcome = observed(&mut store, func, input);
    let first_ns = first_start.elapsed().as_nanos().max(1) as f64;
    let seeded_policy = host_setup_ns.is_some();
    // Historical drivers retain their original wall timer. P027 reports the
    // guest/load stages as an explicit sum which excludes host seed writes.
    let load_ns = if seeded_policy {
        prepare_ns + instantiate_ns + first_ns
    } else {
        start.elapsed().as_nanos().max(1) as f64
    };
    let mut batch = 1;
    let elapsed_ns = match stage {
        "prepare" => prepare_ns,
        "instantiate" => instantiate_ns,
        "first_call" => first_ns,
        "load_and_call" => load_ns,
        "warm_call" => {
            assert!(
                matches!(outcome, Observed::I32(_)),
                "trapping workloads cannot be warmed"
            );
            let expected_result = match outcome {
                Observed::I32(value) => value,
                _ => unreachable!(),
            };
            batch = requested_batch.max(1);
            let total = loop {
                let warm_start = Instant::now();
                for _ in 0..batch {
                    outcome = observed(&mut store, func, input);
                }
                let elapsed = warm_start.elapsed().as_nanos().max(1) as f64;
                assert!(
                    matches!(outcome, Observed::I32(value) if value == expected_result),
                    "warm workload changed its scalar result"
                );
                if requested_batch != 0 || elapsed >= 20_000_000.0 || batch >= 100_000 {
                    break elapsed;
                }
                batch = (batch * 2).min(100_000);
            };
            total / batch as f64
        }
        _ => panic!("unknown memory timing stage"),
    };

    // Export inspection, full-memory readback and hex formatting are outside
    // the load_and_call timing interval. The multi fixture has an explicit
    // memory0/memory export contract; the exported memory name is not used to
    // guess its index.
    let memories = match driver {
        Driver::Alias => vec![alias_memory.expect("alias memory was not created")],
        Driver::Plain => vec![exported_memory(&instance, &store, "memory")],
        Driver::Multi => vec![
            exported_memory(&instance, &store, "memory0"),
            exported_memory(&instance, &store, "memory"),
        ],
        Driver::SeededCopy => vec![
            exported_memory(&instance, &store, "source"),
            exported_memory(&instance, &store, "memory"),
        ],
        Driver::SeededFill => vec![exported_memory(&instance, &store, "memory")],
    };
    let mut all_memory_hex = Vec::new();
    all_memory_hex
        .try_reserve_exact(memories.len())
        .expect("bounded memory list allocation failed");
    let mut combined_bytes = 0usize;
    let mut selected_pages = 0;
    let mut selected_page_size_log2 = 0;
    for (index, memory) in memories.iter().copied().enumerate() {
        let wtime_runtime::ExternalType::Memory(ty) = store
            .external_type(External::Memory(memory))
            .expect("memory type query failed")
        else {
            unreachable!()
        };
        let pages = ty.limits.min;
        let page_bytes = 1u64
            .checked_shl(ty.page_size_log2 as u32)
            .expect("memory page size overflow");
        let byte_len_u64 = pages
            .checked_mul(page_bytes)
            .expect("memory byte length overflow");
        let byte_len = usize::try_from(byte_len_u64).expect("memory does not fit host usize");
        assert!(
            byte_len <= MAX_READBACK_BYTES,
            "memory exceeds 17-page readback byte bound"
        );
        combined_bytes = combined_bytes
            .checked_add(byte_len)
            .expect("combined memory byte length overflow");
        let combined_bound = MAX_READBACK_BYTES
            .checked_mul(memories.len())
            .expect("combined memory byte bound overflow");
        assert!(
            combined_bytes <= combined_bound,
            "memories exceed combined readback byte bound"
        );
        let bytes_out = store
            .read_memory(memory, 0, byte_len)
            .expect("full memory readback failed");
        assert_eq!(bytes_out.len(), byte_len);

        let hex_len = byte_len.checked_mul(2).expect("memory hex length overflow");
        let mut memory_hex = String::new();
        memory_hex
            .try_reserve_exact(hex_len)
            .expect("bounded memory hex allocation failed");
        for byte in bytes_out {
            memory_hex.push(HEX[(byte >> 4) as usize] as char);
            memory_hex.push(HEX[(byte & 0x0f) as usize] as char);
        }
        if index == memories.len() - 1 {
            selected_pages = pages;
            selected_page_size_log2 = ty.page_size_log2;
        }
        all_memory_hex.push(memory_hex);
    }
    let memory_hex = all_memory_hex
        .last()
        .expect("driver must select at least one memory")
        .clone();

    Actual {
        elapsed_ns,
        batch,
        outcome,
        pages: selected_pages,
        page_size_log2: selected_page_size_log2,
        memory_hex,
        memory_count: memories.len(),
        exported_memory_index: if matches!(driver, Driver::Multi | Driver::SeededCopy) {
            1
        } else {
            0
        },
        all_memory_hex,
        host_setup_ns,
        seeded_policy,
    }
}

/// Handle `--memory-driver FILE CASE INPUT EXPECTED_OUTCOME MODE PROFILE DRIVER`.
pub fn try_main(args: &[String]) -> bool {
    let staged = args.first().map(String::as_str) == Some("--memory-staged");
    if !staged && args.first().map(String::as_str) != Some("--memory-driver") {
        return false;
    }
    assert!(
        !cfg!(feature = "performance-counters") && !cfg!(feature = "execution-reference"),
        "memory timing requires a noninstrumented default build"
    );
    assert_eq!(
        args.len(),
        if staged { 10 } else { 8 },
        "--memory-driver FILE CASE INPUT EXPECTED_OUTCOME MODE PROFILE DRIVER"
    );
    let file = &args[1];
    let case = &args[2];
    let input_text = &args[3];
    let input: i32 = input_text
        .strip_prefix("i32:")
        .expect("memory input must be typed i32:N")
        .parse()
        .expect("invalid memory input i32");
    let expected = Expected::parse(&args[4]);
    let stage = if staged {
        args[5].as_str()
    } else {
        "load_and_call"
    };
    let batch: usize = if staged {
        args[6].parse().expect("invalid batch")
    } else {
        1
    };
    assert!(batch <= 100_000, "batch exceeds bound");
    assert!([
        "prepare",
        "instantiate",
        "first_call",
        "warm_call",
        "load_and_call"
    ]
    .contains(&stage));
    assert!(
        stage == "warm_call" || batch == 1,
        "cold memory stages require batch one"
    );
    if stage == "warm_call" {
        assert!(
            is_repeatable_memory_case(case),
            "warm memory case must have a predeclared repeatable-state contract"
        );
    }
    let offset = if staged { 2 } else { 0 };
    let mode = &args[5 + offset];
    let profile = &args[6 + offset];
    let driver = match args[7 + offset].as_str() {
        "plain" => Driver::Plain,
        "alias" => Driver::Alias,
        "multi" => Driver::Multi,
        "seeded-copy" => Driver::SeededCopy,
        "seeded-fill" => Driver::SeededFill,
        _ => panic!("driver must be plain, alias, multi, seeded-copy, or seeded-fill"),
    };

    assert!(
        match driver {
            Driver::SeededCopy => case.starts_with("memory_copy_distinct_seeded_"),
            Driver::SeededFill => case.starts_with("memory_fill_seeded_"),
            _ =>
                !case.starts_with("memory_copy_distinct_seeded_")
                    && !case.starts_with("memory_fill_seeded_"),
        },
        "seeded P027 workload and driver do not match"
    );
    let bytes = std::fs::read(file).expect("cannot read memory fixture");
    assert!(bytes.len() <= 1 << 20, "memory fixture exceeds one MiB");
    let preflight = execute_once(&bytes, input, mode, profile, driver, "load_and_call", 1);
    expected.check(&preflight.outcome);

    // The actual run starts with fresh Engine/Store/memory state. Engine setup
    // stays outside timing; compile/instantiate/import setup/call are timed.
    let actual = execute_once(&bytes, input, mode, profile, driver, stage, batch);
    expected.check(&actual.outcome);
    let result = actual.outcome_text();
    let (trap_kind, trap_message) = match &actual.outcome {
        Observed::Trap { kind, message } => (Some(kind.as_str()), Some(message.as_str())),
        Observed::I32(_) => (None, None),
    };
    let seeded_fields = if actual.seeded_policy {
        format!(",\"host_setup_ns\":{:.3},\"host_setup_excluded\":true,\"measurement_policy\":\"prepare_plus_instantiate_plus_first_call_excluding_host_seed\"", actual.host_setup_ns.expect("seeded driver setup time missing"))
    } else {
        String::new()
    };
    println!(
        "{{\"schema_version\":2,\"case\":{},\"stage\":{},\"input\":{},\"expected\":{},\"result\":{},\"batch\":{},\"ns_per_operation\":{:.3},\"mode\":{},\"profile\":{},\"driver\":{},\"memory_pages\":{},\"memory_page_size_log2\":{},\"memory_hex\":{},\"memory_count\":{},\"exported_memory_index\":{},\"all_memory_hex\":{},\"trap_kind\":{},\"trap_message\":{}{}}}",
        super::quote(case),
        super::quote(stage),
        super::quote(input_text),
        super::quote(&expected.text()),
        super::quote(&result),
        actual.batch,
        actual.elapsed_ns,
        super::quote(mode),
        super::quote(profile),
        super::quote(match driver {
            Driver::Plain => "plain",
            Driver::Alias => "alias",
            Driver::Multi => "multi",
            Driver::SeededCopy => "seeded-copy",
            Driver::SeededFill => "seeded-fill",
        }),
        actual.pages,
        actual.page_size_log2,
        super::quote(&actual.memory_hex),
        actual.memory_count,
        actual.exported_memory_index,
        format!(
            "[{}]",
            actual
                .all_memory_hex
                .iter()
                .map(|value| super::quote(value))
                .collect::<Vec<_>>()
                .join(",")
        ),
        trap_kind
            .map(super::quote)
            .unwrap_or_else(|| "null".to_owned()),
        trap_message
            .map(super::quote)
            .unwrap_or_else(|| "null".to_owned()),
        seeded_fields,
    );
    true
}

fn exported_memory(
    instance: &wtime_runtime::Instance,
    store: &Store,
    name: &str,
) -> wtime_runtime::Memory {
    match instance
        .export(store, name)
        .unwrap_or_else(|_| panic!("missing {name} memory export"))
    {
        External::Memory(memory) => memory,
        other => panic!("{name} export has unexpected kind: {other:?}"),
    }
}

impl Actual {
    fn outcome_text(&self) -> String {
        match &self.outcome {
            Observed::I32(value) => format!("i32:{value}"),
            Observed::Trap { .. } => "trap".to_owned(),
        }
    }
}
