//! Process-isolated, bounded compile/execution adapter for development fuzz tools.
use std::{
    env, fs,
    panic::{AssertUnwindSafe, catch_unwind},
};
use wtime_runtime::*;
fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => out.push_str(&format!("\\u{:04x}", c as u32)),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}
fn value(v: Value) -> String {
    let (ty, bits) = match v {
        Value::I32(n) => ("i32", (n as u32).to_string()),
        Value::I64(n) => ("i64", (n as u64).to_string()),
        Value::F32(n) => ("f32", n.to_string()),
        Value::F64(n) => ("f64", n.to_string()),
        Value::V128(n) => ("v128", n.to_u128().to_string()),
        Value::Ref(RefValue::Null(_)) => ("ref", "null".into()),
        Value::Ref(_) => ("ref", "nonnull".into()),
    };
    format!("{{\"type\":{},\"bits\":{}}}", quote(ty), quote(&bits))
}
fn run() -> Result<String> {
    let mut raw: Vec<String> = env::args().skip(1).collect();
    let mode_value = if raw.len() >= 2 && raw[raw.len() - 2] == "--mode" {
        let value = raw.pop().unwrap();
        raw.pop();
        value
    } else {
        "decoded".to_owned()
    };
    let mode = mode_value.as_str();
    let policy = match mode {
        "decoded" => ExecutionPolicy::Decoded,
        "portable" => ExecutionPolicy::Portable,
        "reference" => ExecutionPolicy::Reference,
        "compact" => ExecutionPolicy::Compact,
        _ => return Err(Error::new(ErrorKind::Host, "unknown execution mode")),
    };
    let mut args = raw.into_iter();
    let profile = args
        .next()
        .ok_or_else(|| Error::new(ErrorKind::Host, "missing profile"))?;
    let file = args
        .next()
        .ok_or_else(|| Error::new(ErrorKind::Host, "missing input"))?;
    let names = args.next();
    let features = match profile.as_str() {
        "mvp" => FeatureSet::mvp(),
        "stable-core" => FeatureSet::stable_core(),
        "all-phase3-core" => FeatureSet::all_phase3_core(),
        _ => return Err(Error::new(ErrorKind::Host, "unknown profile")),
    };
    let limits = ResourceLimits {
        max_module_bytes: 1024 * 1024,
        max_types: 4096,
        max_functions: 4096,
        max_globals: 4096,
        max_locals: 4096,
        max_stack_height: 8192,
        max_call_depth: 128,
        max_memory_pages: 64,
        max_table_elements: 4096,
        max_type_work: 100_000,
        max_rec_group_size: 512,
        max_instructions: 100_000,
        fuel: Some(100_000),
        ..ResourceLimits::default()
    };
    let engine = Engine::with_execution_policy(Config { features, limits }, policy)?;
    let bytes = fs::read(file).map_err(|e| Error::new(ErrorKind::Host, e.to_string()))?;
    let module = engine.compile(&bytes)?;
    let Some(names) = names else {
        return Ok("{\"status\":\"valid\"}".into());
    };
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine).instantiate(&mut store, &module)?;
    let mut results = Vec::new();
    for name in names.split(',') {
        let External::Func(f) = instance.export(&store, name)? else {
            return Err(Error::new(ErrorKind::Host, "export is not a function"));
        };
        match store.invoke(f, &[])? {
            RunOutcome::Returned(values) => results.push(format!(
                "{{\"name\":{},\"values\":[{}]}}",
                quote(name),
                values.into_iter().map(value).collect::<Vec<_>>().join(",")
            )),
            RunOutcome::Exception(_) => return Ok("{\"status\":\"exception\"}".into()),
            RunOutcome::Suspended { .. } => return Ok("{\"status\":\"suspended\"}".into()),
        }
    }
    Ok(format!(
        "{{\"status\":\"returned\",\"results\":[{}]}}",
        results.join(",")
    ))
}
fn main() {
    let outcome = catch_unwind(AssertUnwindSafe(run));
    let output = match outcome {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            let status = match e.kind {
                ErrorKind::Decode => "malformed",
                ErrorKind::Validation => "invalid",
                ErrorKind::DisabledFeature => "unsupported",
                ErrorKind::ResourceLimit => "resource-limit",
                ErrorKind::Link => "link-error",
                ErrorKind::Trap => "trap",
                ErrorKind::Host => "host-error",
            };
            format!(
                "{{\"status\":{},\"message\":{},\"trap\":{}}}",
                quote(status),
                quote(&e.to_string()),
                e.trap_code
                    .map(|c| quote(&format!("{c:?}")))
                    .unwrap_or_else(|| "null".into())
            )
        }
        Err(_) => "{\"status\":\"panic\"}".into(),
    };
    let args: Vec<String> = env::args().collect();
    let mode = if args.len() >= 3 && args[args.len() - 2] == "--mode" {
        args.last().unwrap().as_str()
    } else {
        "decoded"
    };
    let output = output.strip_prefix('{').expect("probe JSON object");
    println!("{{\"execution_mode\":{},{}", quote(mode), output);
}
