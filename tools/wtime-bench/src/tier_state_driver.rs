//! Bounded public state observations for P014; this is never a timing driver.
use wtime_runtime::{Config, External, RefValue, RunOutcome, Store, Value};
use wtime_types::{FeatureSet, ResourceLimits};

fn value(value: Value) -> String {
    match value {
        Value::I32(n) => format!("i32:{n}"),
        Value::I64(n) => format!("i64:{n}"),
        Value::F32(n) => format!("f32_bits:{n:08x}"),
        Value::F64(n) => format!("f64_bits:{n:016x}"),
        Value::V128(n) => format!("v128_bits:{:032x}", n.to_u128()),
        Value::Ref(RefValue::Null(_)) => "null".into(),
        Value::Ref(RefValue::Func(_)) => "function".into(),
        Value::Ref(RefValue::Gc(_)) => "gc".into(),
        other => panic!("unsupported state reference {other:?}"),
    }
}
fn strings(values: impl IntoIterator<Item = String>) -> String {
    format!(
        "[{}]",
        values
            .into_iter()
            .map(|v| super::quote(&v))
            .collect::<Vec<_>>()
            .join(",")
    )
}
pub fn try_main(args: &[String]) -> bool {
    if args.first().map(String::as_str) != Some("--tier-state-driver") {
        return false;
    }
    assert_eq!(
        args.len(),
        7,
        "--tier-state-driver FILE CASE MODE PROFILE FUEL PC"
    );
    assert!(
        !cfg!(feature = "performance-counters"),
        "state observations require counters disabled"
    );
    assert_eq!(args[4], "all-phase3-core");
    let fuel: u64 = args[5].parse().expect("invalid fuel");
    let engine = super::configured_engine(
        Config {
            features: FeatureSet::all_phase3_core(),
            limits: ResourceLimits {
                fuel: Some(fuel),
                ..ResourceLimits::default()
            },
        },
        &args[3],
    );
    let path = std::path::Path::new(&args[1]);
    assert!(std::fs::metadata(path).unwrap().len() <= 1 << 20);
    let module = engine.compile(&std::fs::read(path).unwrap()).unwrap();
    let mut store = Store::new(&engine);
    let instance = wtime_runtime::Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    let External::Func(run) = instance.export(&store, "run").unwrap() else {
        panic!("missing run")
    };
    #[cfg(feature = "execution-reference")]
    if args[6] != "none" {
        assert_eq!(
            engine.execution_policy(),
            wtime_runtime::ExecutionPolicy::Decoded
        );
        store
            .request_materialization_roundtrip(run, args[6].parse().unwrap())
            .unwrap();
    }
    #[cfg(not(feature = "execution-reference"))]
    assert_eq!(args[6], "none", "cursor feature unavailable");
    let (outcome, results, trap) = match store.invoke(run, &[]) {
        Ok(RunOutcome::Returned(values)) => {
            let result = strings(values.iter().copied().map(value));
            store.release_published(&values);
            ("returned".to_owned(), result, "null".to_owned())
        }
        Ok(other) => panic!("unexpected state fixture outcome {other:?}"),
        Err(error) => (
            format!("{:?}", error.kind),
            "[]".into(),
            error
                .trap_code
                .map(|t| super::quote(&format!("{t:?}")))
                .unwrap_or("null".into()),
        ),
    };
    let External::Memory(memory) = instance.export(&store, "memory").unwrap() else {
        panic!()
    };
    let image = store.read_memory(memory, 0, 65536).unwrap();
    let hex: String = image.iter().map(|b| format!("{b:02x}")).collect();
    let External::Global(global) = instance.export(&store, "state").unwrap() else {
        panic!()
    };
    let state = value(store.get_global(global).unwrap());
    let External::Table(table) = instance.export(&store, "table").unwrap() else {
        panic!()
    };
    let table = strings((0..2).map(|index| value(store.read_table(table, index).unwrap())));
    let mut graph = "null".to_owned();
    let mut root = "null".to_owned();
    if outcome == "returned" && args[2] == "state_graph" {
        let External::Global(global) = instance.export(&store, "root").unwrap() else {
            panic!()
        };
        root = super::quote(&value(store.get_global(global).unwrap()));
        let External::Func(project) = instance.export(&store, "graph").unwrap() else {
            panic!()
        };
        let RunOutcome::Returned(values) = store.invoke(project, &[]).unwrap() else {
            panic!()
        };
        graph = strings(values.iter().copied().map(value));
        store.release_published(&values);
    }
    let collection = store.collect().unwrap();
    #[cfg(feature = "execution-reference")]
    let cursor = store.execution_snapshot().map(|view| format!(
        "{{\"source_pc\":{},\"prepared_pc\":{},\"remaining_fuel\":{},\"charged\":{},\"state\":{}}}",
        view.cursor.source_pc, view.cursor.prepared_pc,
        view.remaining_fuel.map(|n| n.to_string()).unwrap_or("null".into()),
        view.pending_boundary_charged, super::quote(&format!("{view:?}")))).unwrap_or("null".into());
    #[cfg(not(feature = "execution-reference"))]
    let cursor = "null";
    println!(
        "{{\"schema_version\":1,\"diagnostic\":true,\"instrumented\":false,\"case\":{},\"mode\":{},\"resolved_policy\":{},\"fuel_budget\":{},\"outcome\":{},\"values\":{},\"trap\":{},\"memory_hex\":{},\"global\":{},\"table\":{},\"root\":{},\"graph\":{},\"collection\":{{\"live\":{},\"reclaimed\":{}}},\"cursor\":{}}}",
        super::quote(&args[2]),
        super::quote(&args[3]),
        super::quote(&format!("{:?}", engine.execution_policy())),
        fuel,
        super::quote(&outcome),
        results,
        trap,
        super::quote(&hex),
        super::quote(&state),
        table,
        root,
        graph,
        collection.live,
        collection.reclaimed,
        cursor
    );
    true
}
