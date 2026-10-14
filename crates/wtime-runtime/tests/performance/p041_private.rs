// Unexecuted original private P041 checks, included by compact/calls.rs.
const PROVIDER: &[u8] = include_bytes!("../fixtures/performance/p041/provider-distinct.wasm");

fn prepared() -> (Store, Module, Instance) {
    let engine =
        Engine::with_execution_policy(Config::default(), ExecutionPolicy::Compact).unwrap();
    let module = engine.compile(PROVIDER).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    (store, module, instance)
}

#[test]
fn p041_instance_binding_work_and_peak_bytes_decline_without_partial_publication() {
    let (store, module, instance) = prepared();
    let inst = &store.instances[instance.0.index];
    let plan = CallBindings::prepare(&store, &module, &inst.funcs).unwrap();
    assert_eq!(plan.resolve(&store, inst, 0).unwrap(), (inst.funcs[0], 1));
    assert!(CallBindings::bounded(&store, &module, &inst.funcs, 0, CALL_MAX_BYTES).is_none());
    assert!(
        CallBindings::bounded(&store, &module, &inst.funcs, plan.work - 1, CALL_MAX_BYTES)
            .is_none()
    );
    assert!(CallBindings::bounded(&store, &module, &inst.funcs, CALL_MAX_WORK, 0).is_none());
    // Peak construction also includes the complete temporary type directory.
    assert!(
        CallBindings::bounded(
            &store,
            &module,
            &inst.funcs,
            CALL_MAX_WORK,
            plan.retained_bytes
        )
        .is_none()
    );
    assert!(CallBindings::prepare(&store, &module, &inst.funcs[..1]).is_none());
    assert!(
        inst.call_bindings.is_none(),
        "pure provider needs no persistent instance plan"
    );
}

#[test]
fn p041_private_bindings_reject_foreign_sources_and_retired_or_reused_generations() {
    let (mut store, module, instance) = prepared();
    let plan =
        CallBindings::prepare(&store, &module, &store.instances[instance.0.index].funcs).unwrap();
    let mut source_copy = store.instances[instance.0.index].clone();
    source_copy.module = store.engine.compile(PROVIDER).unwrap();
    assert_eq!(
        plan.resolve(&store, &source_copy, 0).unwrap_err().message,
        "compact call binding identity"
    );
    let other = Store::new(&store.engine);
    assert_eq!(
        plan.resolve(&other, &store.instances[instance.0.index], 0)
            .unwrap_err()
            .message,
        "compact call binding identity"
    );
    let target = store.instances[instance.0.index].funcs[0];
    let removed = store
        .funcs
        .remove(target.0.index, target.0.generation)
        .unwrap();
    assert_eq!(
        plan.resolve(&store, &store.instances[instance.0.index], 0)
            .unwrap_err()
            .kind,
        ErrorKind::Host
    );
    let (index, generation) = store.funcs.insert(removed);
    assert_eq!(index, target.0.index);
    assert_ne!(generation, target.0.generation);
    assert_eq!(
        plan.resolve(&store, &store.instances[instance.0.index], 0)
            .unwrap_err()
            .kind,
        ErrorKind::Host
    );
    assert_eq!(
        store.invoke(target, &[Value::I32(7)]).unwrap_err().kind,
        ErrorKind::Host
    );
    store.funcs.retire_from(index);
    assert_eq!(
        plan.resolve(&store, &store.instances[instance.0.index], 0)
            .unwrap_err()
            .kind,
        ErrorKind::Host
    );
}

#[test]
fn p041_private_binding_context_and_signature_corruption_fail_closed() {
    let (store, module, instance) = prepared();
    let inst = &store.instances[instance.0.index];
    for case in 0..7 {
        let mut plan = CallBindings::prepare(&store, &module, &inst.funcs).unwrap();
        match case {
            0 => plan.entries[0].as_mut().unwrap().target = inst.funcs[1],
            1 => {
                plan.entries[0].as_mut().unwrap().actual =
                    CanonicalTypeId::test(store.engine.id, 999)
            }
            2 => {
                plan.entries[0].as_mut().unwrap().expected =
                    CanonicalTypeId::test(store.engine.id, 999)
            }
            3 => plan.entries[0].as_mut().unwrap().params += 1,
            4 => plan.entries[0].as_mut().unwrap().results += 1,
            5 => plan.entries[0].as_mut().unwrap().host = true,
            6 => plan.entries[0] = None,
            _ => unreachable!(),
        }
        assert!(
            plan.resolve(&store, inst, 0).is_err(),
            "corruption case {case}"
        );
    }
}
