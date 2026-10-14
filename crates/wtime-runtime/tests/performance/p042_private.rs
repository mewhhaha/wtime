// Source-only P042 adversarial checks, included inside compact/indirect.rs.
const TABLES: &[u8] = include_bytes!("../fixtures/performance/p042/table-mutations.wasm");

fn prepared() -> (Store, Module, Instance) {
    let engine =
        Engine::with_execution_policy(Config::default(), ExecutionPolicy::Compact).unwrap();
    let module = engine.compile(TABLES).unwrap();
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)
        .unwrap();
    (store, module, instance)
}
fn target(store: &Store, function: Func) -> CheckedTarget {
    let record = &store.funcs[store.check(function.0, &store.funcs).unwrap()];
    CheckedTarget {
        function,
        canonical: record.canonical,
        params: record.ty.params.len(),
        results: record.ty.results.len(),
        host: record.host.is_some(),
    }
}

#[test]
fn p042_private_capacity_work_and_round_robin_replacement_are_bounded() {
    let (store, module, instance) = prepared();
    let mut cache = CallSites::prepare(&store, &module).unwrap();
    // Original fixture declares a,b,c first and indirect wrappers fourth/fifth.
    assert_eq!(cache.sites.len(), 2);
    assert!(CallSites::bounded(&store, &module, cache.work - 1, MAX_BYTES).is_none());
    assert!(CallSites::bounded(&store, &module, MAX_WORK, cache.bytes - 1).is_none());
    let functions = &store.instances[instance.0.index].funcs;
    let a = target(&store, functions[0]);
    let b = target(&store, functions[1]);
    let c = target(&store, functions[2]);
    assert_eq!(
        cache.lookup(store.id, &module, 3, 1, 0x11, 0, a).unwrap(),
        Lookup::Miss
    );
    assert!(!cache.remember(store.id, &module, 3, 1, 0x11, 0, a).unwrap());
    assert!(!cache.remember(store.id, &module, 3, 1, 0x11, 0, b).unwrap());
    assert_eq!(
        cache.lookup(store.id, &module, 3, 1, 0x11, 0, a).unwrap(),
        Lookup::Hit
    );
    let capacity = cache.sites.capacity();
    for (index, current) in [c, a, b, c, a, b, c].into_iter().enumerate() {
        assert!(
            cache
                .remember(store.id, &module, 3, 1, 0x11, 0, current)
                .unwrap()
        );
        assert_eq!(
            cache
                .lookup(store.id, &module, 3, 1, 0x11, 0, current)
                .unwrap(),
            Lookup::Hit
        );
        assert_eq!(cache.sites.capacity(), capacity);
        assert_eq!(
            cache.sites[0]
                .entries
                .iter()
                .filter(|value| value.is_some())
                .count(),
            2
        );
        assert_eq!(cache.sites[0].next, (index + 1) % 2);
    }
    assert_eq!(
        cache.lookup(store.id, &module, 4, 1, 0x11, 0, a).unwrap(),
        Lookup::Miss
    );
    assert_eq!(
        cache.lookup(store.id, &module, 99, 1, 0x11, 0, a).unwrap(),
        Lookup::Declined
    );
}

#[test]
fn p042_private_context_signature_and_cached_metadata_corruption_fail_closed() {
    let (store, module, instance) = prepared();
    let a = target(&store, store.instances[instance.0.index].funcs[0]);
    let cache = CallSites::prepare(&store, &module).unwrap();
    let another = store.engine.compile(TABLES).unwrap();
    assert!(cache.lookup(store.id, &another, 3, 1, 0x11, 0, a).is_err());
    assert!(
        cache
            .lookup(store.id + 1, &module, 3, 1, 0x11, 0, a)
            .is_err()
    );
    assert!(cache.lookup(store.id, &module, 3, 1, 0x13, 0, a).is_err());
    assert!(cache.lookup(store.id, &module, 3, 1, 0x11, 1, a).is_err());
    for case in 0..4 {
        let mut broken = cache.clone();
        match case {
            0 => broken.sites[0].expected = CanonicalTypeId::test(store.engine.id, 999),
            1 => broken.sites[0].next = 2,
            2 => broken.work = MAX_WORK + 1,
            3 => broken.bytes = MAX_BYTES + 1,
            _ => unreachable!(),
        }
        assert!(broken.lookup(store.id, &module, 3, 1, 0x11, 0, a).is_err());
    }
    for case in 0..4 {
        let mut broken = cache.clone();
        let mut other = a;
        match case {
            0 => other.params += 1,
            1 => other.results += 1,
            2 => other.host = !other.host,
            3 => other.canonical = CanonicalTypeId::test(store.engine.id, 999),
            _ => unreachable!(),
        }
        broken.sites[0].entries[0] = Some(other);
        assert_eq!(
            broken.lookup(store.id, &module, 3, 1, 0x11, 0, a).unwrap(),
            Lookup::Miss
        );
    }
}

#[test]
fn p042_private_live_generation_and_negative_answers_never_reuse_success() {
    let (mut store, module, instance) = prepared();
    let function = store.instances[instance.0.index].funcs[0];
    assert!(
        store
            .compact_indirect_compatible(instance.0.index, &module, 3, 1, 0x11, 0, function)
            .unwrap()
    );
    assert!(
        store
            .compact_indirect_compatible(instance.0.index, &module, 3, 1, 0x11, 0, function)
            .unwrap()
    );
    let before = store.compact_indirect_hits;
    let removed = store
        .funcs
        .remove(function.0.index, function.0.generation)
        .unwrap();
    assert_eq!(
        store
            .compact_indirect_compatible(instance.0.index, &module, 3, 1, 0x11, 0, function)
            .unwrap_err()
            .kind,
        ErrorKind::Host
    );
    let (index, generation) = store.funcs.insert(removed);
    let replacement = Func(Handle {
        store: store.id,
        index,
        generation,
    });
    assert_ne!(generation, function.0.generation);
    assert!(
        store
            .compact_indirect_compatible(instance.0.index, &module, 3, 1, 0x11, 0, replacement)
            .unwrap()
    );
    assert_eq!(store.compact_indirect_hits, before);
    let wrong = store
        .create_host_func(
            FuncType::new(vec![ValType::I64], vec![ValType::I32]),
            |_| Ok(vec![Value::I32(0)]),
        )
        .unwrap();
    assert!(
        !store
            .compact_indirect_compatible(instance.0.index, &module, 3, 1, 0x11, 0, wrong)
            .unwrap()
    );
    assert!(
        !store
            .compact_indirect_compatible(instance.0.index, &module, 3, 1, 0x11, 0, wrong)
            .unwrap()
    );
    assert_eq!(store.compact_indirect_hits, before);
    let other = Store::new(&store.engine);
    let foreign = Func(Handle {
        store: other.id,
        index,
        generation,
    });
    assert_eq!(
        store
            .compact_indirect_compatible(instance.0.index, &module, 3, 1, 0x11, 0, foreign)
            .unwrap_err()
            .kind,
        ErrorKind::Host
    );
    assert_eq!(store.compact_indirect_hits, before);
}
