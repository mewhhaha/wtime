//! Diagnostic tests compiled beside an exact copy of the current core.
//! The current runtime tree is never edited by this proof.
use super::*;

fn setup(policy: ExecutionPolicy, bytes: &[u8], fuel: Option<u64>) -> (Store, Instance) {
    let mut config = Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    };
    config.limits.fuel = fuel;
    let engine = Engine::with_execution_policy(config, policy).unwrap();
    let module = engine.compile(bytes).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine).instantiate(&mut store, &module).unwrap();
    (store, instance)
}

fn function(store: &Store, instance: Instance, name: &str) -> Func {
    let External::Func(function) = instance.export(store, name).unwrap() else { panic!("function export required") };
    function
}

#[test]
fn p127_actual_accounting_merges_saved_fuel_once_and_preserves_error_priority() {
    for (current, saved, expected) in [
        (None, None, None),
        (None, Some(3), Some(2)),
        (Some(5), None, Some(4)),
        (Some(5), Some(3), Some(2)),
        (Some(3), Some(5), Some(2)),
    ] {
        let engine = Engine::new(Config::default()).unwrap();
        let mut store = Store::new(&engine);
        store.resume_fuel = Some(saved);
        let mut fuel = current;
        let mut prompts = Vec::new();
        super::accounting::next_source(&mut store, &mut fuel, 0, 0, &mut prompts).unwrap();
        assert_eq!(fuel, expected);
        assert_eq!(store.current_fuel, expected);
        assert_eq!(store.resume_fuel, None);
        super::accounting::next_source(&mut store, &mut fuel, 0, 0, &mut prompts).unwrap();
        assert_eq!(fuel, expected.map(|n| n - 1));
    }
    for (cancel, budget, expected) in [
        (true, Some(0), "execution cancelled"),
        (false, Some(0), "fuel exhausted"),
        (false, Some(1), "operand stack limit"),
    ] {
        let mut config = Config::default();
        config.limits.max_stack_height = 0;
        let engine = Engine::new(config).unwrap();
        let mut store = Store::new(&engine);
        if cancel { store.cancellation_token().cancel(); }
        let mut fuel = budget;
        let error = super::accounting::next_source(&mut store, &mut fuel, 0, 1, &mut Vec::new()).unwrap_err();
        assert_eq!(error.kind, ErrorKind::ResourceLimit);
        assert_eq!(error.message, expected);
    }
    println!("P127_ACCOUNTING\t5\t3");
}

#[test]
fn p127_scalar_policies_match_reference_and_refill_each_invocation() {
    let plan = std::fs::read_to_string(std::env::var("P127_SCALAR_PLAN").unwrap()).unwrap();
    let mut rows = 0;
    for line in plan.lines() {
        let fields = line.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 4);
        let bytes = std::fs::read(fields[1]).unwrap();
        let input = fields[2].parse::<i32>().unwrap();
        let expected = fields[3].parse::<i32>().unwrap();
        let (mut reference, instance) = setup(ExecutionPolicy::Reference, &bytes, None);
        let export = function(&reference, instance, "run");
        let before = reference.reference_dispatches();
        assert_eq!(reference.invoke(export, &[Value::I32(input)]).unwrap(), RunOutcome::Returned(vec![Value::I32(expected)]));
        let source_events = reference.reference_dispatches() - before;
        assert!(source_events > 0 && source_events < 50_000_000);
        for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact, ExecutionPolicy::Reference] {
            for budget in [None, Some(50_000_000)] {
                let (mut store, instance) = setup(policy, &bytes, budget);
                let export = function(&store, instance, "run");
                for _ in 0..3 {
                    assert_eq!(store.invoke(export, &[Value::I32(input)]).unwrap(), RunOutcome::Returned(vec![Value::I32(expected)]));
                    assert_eq!(store.current_fuel, budget.map(|n| n - source_events));
                }
                println!("P127_ROW\t{}\t{:?}\t{}\t{}\t3", fields[0], policy, if budget.is_some() { "metered" } else { "unmetered" }, source_events);
            }
        }
        rows += 1;
    }
    assert_eq!(rows, 9);
}

#[test]
fn p127_low_fuel_prefixes_bind_memory_cursor_and_remaining_budget() {
    let bytes = std::fs::read(std::env::var("P127_PREFIX_WASM").unwrap()).unwrap();
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact, ExecutionPolicy::Reference] {
        for budget in 0..=7 {
            let (mut store, instance) = setup(policy, &bytes, Some(budget));
            let export = function(&store, instance, "run");
            let capture = policy != ExecutionPolicy::Reference && (1..=6).contains(&budget);
            if capture { store.request_materialization_roundtrip(export, budget as usize - 1).unwrap(); }
            let result = store.invoke(export, &[]);
            if budget < 6 {
                let error = result.unwrap_err();
                assert_eq!(error.kind, ErrorKind::ResourceLimit);
                assert_eq!(error.message, "fuel exhausted");
            } else {
                assert_eq!(result.unwrap(), RunOutcome::Returned(vec![Value::I32(7)]));
                assert_eq!(store.current_fuel, Some(budget - 6));
            }
            let External::Memory(memory) = instance.export(&store, "memory").unwrap() else { panic!() };
            assert_eq!(store.read_memory(memory, 0, 4).unwrap(), if budget >= 3 { vec![7,0,0,0] } else { vec![0,0,0,0] });
            if capture {
                let snapshot = store.execution_snapshot().unwrap();
                assert_eq!(snapshot.cursor.source_pc, budget as usize - 1);
                assert_eq!(snapshot.remaining_fuel, Some(0));
                assert!(snapshot.pending_boundary_charged);
                assert_eq!(snapshot.memory_prefixes[0][..4], if budget >= 4 { [7,0,0,0] } else { [0,0,0,0] });
            }
        }
    }
    println!("P127_PREFIXES\t24\t12");
}

#[test]
fn p127_real_continuations_preserve_none_some_fuel_transitions() {
    // The admitted unhandled-suspension module used by t145: two capture
    // events, three resume-caller events, then the two implicit ends.
    const UNHANDLED: &[u8] = &[0,97,115,109,1,0,0,0,1,18,4,96,0,1,127,96,1,127,1,127,93,1,96,1,100,2,1,127,3,3,2,0,3,13,3,1,0,1,7,16,2,3,114,117,110,0,0,6,114,101,115,117,109,101,0,1,10,18,2,6,0,65,7,226,0,11,9,0,65,9,32,0,227,2,0,11];
    for policy in [ExecutionPolicy::Decoded, ExecutionPolicy::Compact, ExecutionPolicy::Reference] {
        for capture_budget in [None, Some(2), Some(20)] {
            for resume_budget in [None, Some(5), Some(50_000_000)] {
                let (mut store, instance) = setup(policy, UNHANDLED, capture_budget);
                let run = function(&store, instance, "run");
                let RunOutcome::Suspended { payload, continuation, .. } = store.invoke(run, &[]).unwrap() else { panic!("expected suspension") };
                assert_eq!(payload, vec![Value::I32(7)]);
                store.engine.config.limits.fuel = resume_budget;
                let resume = function(&store, instance, "resume");
                let saved = capture_budget.map(|n| n - 2);
                let current = resume_budget.map(|n| n - 3);
                let merged = match (current, saved) { (Some(a), Some(b)) => Some(a.min(b)), (a, None) => a, (None, b) => b };
                let result = store.invoke(resume, &[Value::Ref(RefValue::Gc(continuation))]);
                if merged.is_some_and(|n| n < 2) {
                    let error = result.unwrap_err();
                    assert_eq!(error.kind, ErrorKind::ResourceLimit);
                    assert_eq!(error.message, "fuel exhausted");
                } else {
                    assert_eq!(result.unwrap(), RunOutcome::Returned(vec![Value::I32(9)]));
                    assert_eq!(store.current_fuel, merged.map(|n| n - 2));
                }
                println!("P127_RESUME\t{:?}\t{:?}\t{:?}", policy, capture_budget, resume_budget);
            }
        }
    }
}
