//! P043 source-only compact memory path-counter harness draft.
//!
//! Route from protocol.rs only for `--memory-diagnostic`. Keep the existing
//! --memory-driver and --memory-staged branches unchanged. This harness emits
//! no durations and separates an oracle preflight Store from one fresh counted
//! Store invocation.
use std::hint::black_box;
use wtime_runtime::{Config, ErrorKind, External, Linker, RunOutcome, Store, Value};
use wtime_types::FeatureSet;

const PAGE: usize = 65_536;
const HEX: &[u8; 16] = b"0123456789abcdef";

#[derive(Clone, Debug, PartialEq, Eq)]
enum Seen { I32(i32), Trap }

fn engine(profile: &str) -> wtime_runtime::Engine {
    let mut config = Config::default();
    config.features = match profile {
        "all-phase3-core" => FeatureSet::all_phase3_core(),
        "mvp" => FeatureSet::mvp(),
        _ => panic!("unsupported P043 diagnostic feature profile"),
    };
    super::configured_engine(config, "compact")
}
fn invoke(store: &mut Store, instance: wtime_runtime::Instance, input: i32) -> Seen {
    let func = match instance.export(store, "run").expect("missing run export") {
        External::Func(f) => f,
        other => panic!("run export is not a function: {other:?}"),
    };
    match store.invoke(func, black_box(&[Value::I32(input)])) {
        Ok(RunOutcome::Returned(values)) => match values.as_slice() {
            [Value::I32(value)] => Seen::I32(*value),
            other => panic!("memory benchmark expected one i32: {other:?}"),
        },
        Err(e) if e.kind == ErrorKind::Trap => Seen::Trap,
        Err(e) => panic!("non-trap memory execution error: {e}"),
        Ok(other) => panic!("unexpected memory outcome: {other:?}"),
    }
}
fn seed(len: usize, mul: u8, add: u8, page_mul: u8) -> Vec<u8> {
    (0..len).map(|i| (i as u8).wrapping_mul(mul).wrapping_add(add)
        .wrapping_add(((i >> 16) as u8).wrapping_mul(page_mul))).collect()
}
fn memory_type() -> wtime_types::MemoryType {
    wtime_types::MemoryType { limits: wtime_types::Limits::new(17, Some(17)),
        index_type: wtime_types::IndexType::I32, shared: false, page_size_log2: 16 }
}
fn get_memory(instance: &wtime_runtime::Instance, store: &Store, name: &str) -> wtime_runtime::Memory {
    match instance.export(store, name).unwrap_or_else(|_| panic!("missing memory export {name}")) {
        External::Memory(m) => m,
        other => panic!("{name} is not memory: {other:?}"),
    }
}
fn memory_exports(case: &str, instance: &wtime_runtime::Instance, store: &Store,
    alias: Option<wtime_runtime::Memory>) -> Vec<(String, wtime_runtime::Memory)> {
    match case {
        "memory_multi_memory_second_export" => vec![
            ("memory0".into(), get_memory(instance,store,"memory0")),
            ("memory".into(), get_memory(instance,store,"memory"))],
        id if id.starts_with("memory_copy_distinct_seeded_") => vec![
            ("source".into(),get_memory(instance,store,"source")),
            ("memory".into(),get_memory(instance,store,"memory"))],
        _ if alias.is_some() => vec![("aliased".into(),alias.unwrap())],
        _ => vec![("memory".into(),get_memory(instance,store,"memory"))],
    }
}
fn image_hex(store: &Store, memory: wtime_runtime::Memory) -> (u64,u8,String) {
    let wtime_runtime::ExternalType::Memory(ty)=store.external_type(External::Memory(memory)).expect("memory type lookup") else { unreachable!() };
    let pages=ty.limits.min;
    let bytes_u64=pages.checked_shl(ty.page_size_log2 as u32).expect("memory length overflow");
    let len=usize::try_from(bytes_u64).expect("memory exceeds host address space");
    assert!(len <= 17*PAGE, "diagnostic memory image exceeds 17 pages");
    let image=store.read_memory(memory,0,len).expect("full memory readback");
    let mut hex=String::with_capacity(image.len().checked_mul(2).unwrap());
    for b in image { hex.push(HEX[(b>>4) as usize] as char); hex.push(HEX[(b&15) as usize] as char); }
    (pages,ty.page_size_log2,hex)
}
fn one_invocation(bytes: &[u8], case: &str, input: i32, profile: &str, counted: bool)
    -> (Seen, Vec<(String,u64,u8,String)>, wtime_runtime::CounterSnapshot) {
    let e=engine(profile); let module=e.compile(bytes).expect("memory fixture compile");
    let mut store=Store::new(&e); let mut linker=Linker::new(&e);
    let alias=if case.starts_with("memory_copy_aliased_") {
        let m=store.create_memory(memory_type()).expect("alias memory create");
        linker.define("mem","left",External::Memory(m)).unwrap();
        linker.define("mem","right",External::Memory(m)).unwrap(); Some(m)
    } else { None };
    let instance=linker.instantiate(&mut store,&module).expect("memory instantiate");
    if case.starts_with("memory_copy_distinct_seeded_") {
        let source=get_memory(&instance,&store,"source");
        let destination=get_memory(&instance,&store,"memory");
        store.write_memory(source,0,&seed(16*PAGE,29,7,53)).unwrap();
        store.write_memory(destination,0,&seed(17*PAGE,13,101,37)).unwrap();
    } else if case.starts_with("memory_fill_seeded_") {
        let memory=get_memory(&instance,&store,"memory");
        store.write_memory(memory,0,&seed(17*PAGE,13,101,37)).unwrap();
    }
    if counted { store.reset_performance_counters(); }
    let seen=invoke(&mut store,instance,input);
    // Snapshot before public read_memory() observes the result, because that
    // host API itself contributes read-vector events under diagnostics.
    let counters=store.performance_counters();
    let images=memory_exports(case,&instance,&store,alias).into_iter().map(|(name,m)| {
        let (pages,log2,hex)=image_hex(&store,m); (name,pages,log2,hex)
    }).collect();
    (seen,images,counters)
}
pub fn try_main(args: &[String]) -> bool {
    if args.first().map(String::as_str)!=Some("--memory-diagnostic") { return false; }
    assert_eq!(args.len(),6,"--memory-diagnostic FILE CASE INPUT EXPECTED PROFILE");
    assert!(cfg!(all(feature="compact-executor",feature="execution-reference",feature="performance-counters")),
        "P043 diagnostic requires compact, execution-reference and performance-counters features");
    let bytes=std::fs::read(&args[1]).expect("read memory fixture");
    assert!(bytes.len()<=1<<20,"memory fixture exceeds one MiB");
    let case=&args[2]; let input=args[3].parse::<i32>().expect("invalid i32 input");
    let expected=&args[4]; let profile=&args[5];
    let expected_seen=if expected=="trap" { Seen::Trap } else { Seen::I32(expected.strip_prefix("i32:").expect("typed expected i32").parse().unwrap()) };
    // The first invocation is disposable and its result is checked before the
    // fresh counted instance is constructed. No counters or timings are kept.
    let (preflight,_,_)=one_invocation(&bytes,case,input,profile,false);
    assert_eq!(preflight,expected_seen,"independent memory oracle preflight mismatch");
    let (seen,images,counters)=one_invocation(&bytes,case,input,profile,true);
    assert_eq!(seen,expected_seen,"fresh counted result differs from oracle");
    let result=match seen { Seen::I32(x)=>format!("i32:{x}"),Seen::Trap=>"trap".into() };
    let image_json=images.iter().map(|(name,pages,log2,hex)| format!(
        "{{\"name\":{},\"pages\":{},\"page_size_log2\":{},\"hex\":{}}}",
        super::quote(name),pages,log2,super::quote(hex))).collect::<Vec<_>>().join(",");
    println!("{{\"schema\":\"wtime-p043-compact-memory-diagnostic-v1\",\"instrumented\":true,\"mode\":\"compact\",\"case\":{},\"result\":{},\"memory_count\":{},\"exported_memory_index\":{},\"all_memory_images\":[{}],\"oracle_preflight_guest_invocations\":1,\"counted_guest_invocations\":1,\"counter_reset_count\":1,\"counters\":{}}}",
        super::quote(case),super::quote(&result),images.len(),if case=="memory_multi_memory_second_export" {1} else {0},image_json,counters.to_json());
    true
}
