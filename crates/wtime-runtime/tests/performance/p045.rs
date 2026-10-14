//! Deterministic valid MVP binaries, independent arithmetic/state oracles, and
//! retained byte-for-byte replay records. No encoder process per generated case.
#![cfg(all(feature = "compact-executor", feature = "execution-reference"))]
use wtime_runtime::*;
const SEED: u32 = 20445;
const POLICIES: [ExecutionPolicy; 3] = [ExecutionPolicy::Decoded, ExecutionPolicy::Reference, ExecutionPolicy::Compact];
fn leb(out: &mut Vec<u8>, mut value: u32) {
    loop { let byte = (value & 127) as u8; value >>= 7; out.push(byte | if value == 0 { 0 } else { 128 }); if value == 0 { break; } }
}
fn signed(out: &mut Vec<u8>, mut value: i32) {
    loop { let byte = (value & 127) as u8; value >>= 7; let done = (value == 0 && byte & 64 == 0) || (value == -1 && byte & 64 != 0);
        out.push(byte | if done { 0 } else { 128 }); if done { break; } }
}
fn constant(out: &mut Vec<u8>, value: i32) { out.push(0x41); signed(out, value); }
fn section(out: &mut Vec<u8>, id: u8, payload: &[u8]) { out.push(id); leb(out, payload.len() as u32); out.extend(payload); }
fn name(out: &mut Vec<u8>, text: &str) { leb(out, text.len() as u32); out.extend(text.bytes()); }
fn hex(bytes: &[u8]) -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() }
struct Case {
    index: usize, family: &'static str, bytes: Vec<u8>, body: Vec<u8>, input: i32,
    expected: i32, events: u64, snapshot_pc: usize, prefix: bool, address: usize, stored: i32,
}
fn recipe(index: usize, random: u32) -> Case {
    let input = match index % 7 { 0 => 0, 1 => 1, 2 => -1, 3 => i32::MIN, 4 => i32::MAX, _ => random as i32 };
    let k = random.rotate_left(11) as i32;
    let op = [0x6a,0x6b,0x6c,0x71,0x72,0x73,0x74,0x75,0x76,0x77,0x78][index % 11];
    let base = match op {
        0x6a => input.wrapping_add(k), 0x6b => input.wrapping_sub(k), 0x6c => input.wrapping_mul(k),
        0x71 => input & k, 0x72 => input | k, 0x73 => input ^ k,
        0x74 => input.wrapping_shl(k as u32 & 31), 0x75 => input.wrapping_shr(k as u32 & 31),
        0x76 => (input as u32).wrapping_shr(k as u32 & 31) as i32,
        0x77 => input.rotate_left(k as u32 & 31), 0x78 => input.rotate_right(k as u32 & 31), _ => unreachable!(),
    };
    let family = ["scalar", "local", "if", "branch", "memory", "trap"][index % 6];
    let mut body = if family == "local" || family == "if" { vec![1,1,0x7f] } else { vec![0] };
    let address = (random as usize % 16383) * 4;
    if family == "branch" { body.extend([2,0x7f]); }
    if family == "memory" { constant(&mut body, address as i32); }
    body.extend([0x20,0]); constant(&mut body, k); body.push(op);
    let (expected, events, snapshot_pc) = match family {
        "scalar" => (base,4,2),
        "local" => { body.extend([0x22,1,0x20,1,0x6a]); (base.wrapping_add(base),7,3) }
        "if" => {
            body.extend([0x21,1,0x20,0,4,0x7f,0x20,1,5,0x20,1]);
            constant(&mut body, k); body.extend([0x73,0x0b]);
            (if input == 0 { base ^ k } else { base }, if input == 0 { 11 } else { 10 },5)
        }
        "branch" => { body.extend([0x0c,0,0x0b]); (base,6,4) }
        "memory" => {
            body.extend([0x36,2,0]); constant(&mut body,0); body.extend([0x40,0,0x1a]);
            constant(&mut body,address as i32); body.extend([0x28,2,0]); (base,11,4)
        }
        "trap" => { body.extend([0x1a,0]); (0,5,4) }
        _ => unreachable!(),
    };
    body.push(0x0b);
    let mut bytes = b"\0asm\x01\0\0\0".to_vec();
    section(&mut bytes,1,&[1,0x60,1,0x7f,1,0x7f]); section(&mut bytes,3,&[1,0]);
    if family == "memory" { section(&mut bytes,5,&[1,0,1]); }
    let mut exports = vec![if family == "memory" { 2 } else { 1 }];
    name(&mut exports,&format!("case_{index}")); exports.extend([0,0]);
    if family == "memory" { name(&mut exports,"memory"); exports.extend([2,0]); }
    section(&mut bytes,7,&exports);
    let mut code = vec![1]; leb(&mut code,body.len() as u32); code.extend(&body); section(&mut bytes,10,&code);
    Case { index, family, bytes, body, input, expected, events, snapshot_pc, prefix: family == "memory", address, stored: base }
}
fn load(case: &Case, policy: ExecutionPolicy, fuel: u64) -> (Store, Func, Option<Memory>) {
    let mut config = Config { features: FeatureSet::mvp(), ..Config::default() }; config.limits.fuel = Some(fuel);
    let engine = Engine::with_execution_policy(config,policy).unwrap();
    // Normal admission and validation always precede compact preparation.
    let module = engine.compile(&case.bytes).unwrap_or_else(|e| panic!("case={} seed={SEED} body={} invalid: {e:?}",case.index,hex(&case.body)));
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine).instantiate(&mut store,&module).unwrap();
    let External::Func(function) = instance.export(&store,&format!("case_{}",case.index)).unwrap() else { panic!() };
    let memory = if case.prefix { let External::Memory(memory) = instance.export(&store,"memory").unwrap() else { panic!() }; Some(memory) } else { None };
    (store,function,memory)
}
fn outcome(result: Result<RunOutcome>) -> std::result::Result<RunOutcome,(ErrorKind,String)> { result.map_err(|e| (e.kind,e.message)) }
fn check(case: &Case) -> u64 {
    let mut full_compact = 0;
    for fuel in 0..=case.events+1 {
        for policy in POLICIES {
            let (mut store,function,memory) = load(case,policy,fuel);
            let actual = outcome(store.invoke(function,&[Value::I32(case.input)]));
            let expected = if fuel < case.events { Err((ErrorKind::ResourceLimit,"fuel exhausted".into())) }
                else if case.family == "trap" { Err((ErrorKind::Trap,"unreachable".into())) }
                else { Ok(RunOutcome::Returned(vec![Value::I32(case.expected)])) };
            assert_eq!(actual,expected,"case={} seed={SEED} body={} input={} {policy:?} fuel={fuel}",case.index,hex(&case.body),case.input);
            if let Some(memory) = memory {
                let image = store.read_memory(memory,0,65536).unwrap(); let mut wanted = vec![0;65536];
                if fuel >= 5 { wanted[case.address..case.address+4].copy_from_slice(&case.stored.to_le_bytes()); }
                assert_eq!(image,wanted,"memory case={} {policy:?} fuel={fuel}",case.index);
            }
            let steps = if policy == ExecutionPolicy::Compact { fuel.min(if case.prefix { 4 } else { case.events }) } else { 0 };
            assert_eq!(store.compact_dispatches(),steps,"path case={} {policy:?} fuel={fuel}",case.index);
            if case.prefix {
                assert_eq!(store.compact_prefix_counts(), (steps,[0,0,0,u64::from(policy == ExecutionPolicy::Compact && fuel>=5)],
                    if policy == ExecutionPolicy::Compact && fuel>=5 { Some((4,0x36)) } else { None }));
            }
            if policy == ExecutionPolicy::Compact && fuel == case.events+1 { full_compact = steps; }
        }
    }
    // Compare canonical locals, operands, labels and charged pending cursor.
    let mut snapshots = vec![];
    for policy in [ExecutionPolicy::Decoded,ExecutionPolicy::Compact] {
        let (mut store,function,_) = load(case,policy,case.events+1);
        store.request_materialization_roundtrip(function,case.snapshot_pc).unwrap();
        let _ = store.invoke(function,&[Value::I32(case.input)]);
        let snapshot = store.execution_snapshot().unwrap().clone();
        assert_eq!(snapshot.cursor.source_pc,case.snapshot_pc); assert!(snapshot.pending_boundary_charged);
        snapshots.push(snapshot);
    }
    assert_eq!(snapshots[0],snapshots[1],"snapshot case={}",case.index);
    assert!(full_compact > 0); full_compact
}
fn generate(count: usize, record: bool) {
    let mut random = SEED;
    for index in 0..count {
        random ^= random << 13; random ^= random >> 17; random ^= random << 5;
        let case = recipe(index,random); let steps = check(&case);
        if record {
            let expected = if case.family == "trap" { String::from("\"expected_kind\":\"trap\"") }
                else { format!("\"expected_kind\":\"returned\",\"expected_i32\":{}",case.expected) };
            let memory = if case.prefix { format!(",\"memory_export\":\"memory\",\"expected_memory\":{{\"bytes\":65536,\"writes\":[{{\"offset\":{},\"data_hex\":\"{}\"}}]}}",case.address,hex(&case.stored.to_le_bytes())) } else { String::new() };
            println!("\nP045_CASE {{\"schema\":\"p045-corpus-v1\",\"seed\":{SEED},\"case_index\":{index},\"family\":\"{}\",\"body_hex\":\"{}\",\"module_hex\":\"{}\",\"export\":\"case_{index}\",\"input_i32\":{},\"events\":{},\"policy_count\":3,\"fuel_probes\":{},\"compact_steps\":{steps},{expected}{memory}}}",case.family,hex(&case.body),hex(&case.bytes),case.input,case.events,case.events+2);
        }
    }
    if record { println!("\nP045_SUMMARY {{\"schema\":\"p045-corpus-v1\",\"seed\":{SEED},\"case_count\":{count},\"binary_path\":{:?}}}",std::env::current_exe().unwrap().to_str().unwrap()); }
}

#[test]
fn p045_smoke_all_generated_families_and_boundary_inputs() { generate(42,false); }
#[test]
fn p045_generated_thousand_cases_match_three_policies_state_and_every_fuel() { generate(1000,true); }
#[test]
fn p045_precancellation_has_no_compact_or_memory_effects() {
    for index in 0..6 {
        let case = recipe(index,SEED);
        for policy in POLICIES {
            let (mut store,function,memory) = load(&case,policy,0); store.cancellation_token().cancel();
            assert_eq!(outcome(store.invoke(function,&[Value::I32(case.input)])),Err((ErrorKind::ResourceLimit,"execution cancelled".into())));
            assert_eq!(store.compact_dispatches(),0);
            if let Some(memory) = memory { assert!(store.read_memory(memory,0,65536).unwrap().iter().all(|&b|b==0)); }
        }
    }
}
