#[test]
fn p063_invalid_memory_metadata_and_binding_declines() {
    use crate::*;
    let bytes = include_bytes!("../fixtures/performance/p063/scalar_loads.wasm");
    let engine =
        Engine::with_execution_policy(Config::default(), ExecutionPolicy::Register).unwrap();
    let module = engine.compile(bytes).unwrap();
    for corruption in 0..5 {
        let mut initial = Budget::new(MAX_WORK, MAX_BYTES);
        let lowered =
            super::super::lower::LoweredFunction::lower(&module, 0, &mut initial).unwrap();
        let RegisterFunction {
            admission,
            mut draft,
            ..
        } = lowered.program;
        match corruption {
            0 => draft.memory[0].offset = 1,
            1 => draft.memory[0].memory = u32::MAX,
            2 => draft.memory[0].align = u32::MAX,
            3 => {
                let Operation::Memory { value, .. } = &mut draft.operations[1] else {
                    panic!()
                };
                *value = 0;
            }
            _ => draft.memory.push(draft.memory[0]),
        }
        let mut budget = Budget::new(MAX_WORK, MAX_BYTES);
        assert!(matches!(
            RegisterFunction::from_draft(&module, admission, draft, &mut budget),
            Err(Declined::Metadata)
        ));
        assert_eq!(budget.remaining(), (MAX_WORK, MAX_BYTES));
    }
    for (work, bytes) in [(0, MAX_BYTES), (MAX_WORK, 0)] {
        let mut budget = Budget::new(work, bytes);
        assert!(super::super::lower::LoweredFunction::lower(&module, 0, &mut budget).is_err());
        assert_eq!(budget.remaining(), (work, bytes));
    }
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    assert!(super::super::execute::bindings_admitted(
        &store,
        instance.0.index,
        0
    ));
    assert!(!super::super::execute::bindings_admitted(
        &store,
        usize::MAX,
        0
    ));
    assert!(!super::super::execute::bindings_admitted(
        &store,
        instance.0.index,
        usize::MAX
    ));
    let mut foreign = Store::new(&engine);
    let foreign_instance = Linker::new(&engine)
        .instantiate(&mut foreign, &module)
        .unwrap();
    store.instances[instance.0.index].mems[0] = foreign.instances[foreign_instance.0.index].mems[0];
    assert!(!super::super::execute::bindings_admitted(
        &store,
        instance.0.index,
        0
    ));
}
