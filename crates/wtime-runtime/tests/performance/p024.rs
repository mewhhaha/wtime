use wtime_runtime::*;
fn engine() -> Engine {
    Engine::new(Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    })
    .unwrap()
}
fn load(name: &str) -> (Store, Instance) {
    let e = engine();
    let bytes = match name {
        "overlap_multivalue" => {
            include_bytes!("../fixtures/performance/branch/overlap_multivalue.wasm").as_slice()
        }
        "loop_params_br_table" => {
            include_bytes!("../fixtures/performance/branch/loop_params_br_table.wasm").as_slice()
        }
        "nested_catch_order" => {
            include_bytes!("../fixtures/performance/branch/nested_catch_order.wasm").as_slice()
        }
        "captured_try_table_one" => {
            include_bytes!("../fixtures/performance/branch/captured_try_table_one.wasm").as_slice()
        }
        "captured_try_table_four" => {
            include_bytes!("../fixtures/performance/branch/captured_try_table_four.wasm").as_slice()
        }
        "try_table_normal" => {
            include_bytes!("../fixtures/performance/branch/try_table_normal.wasm").as_slice()
        }
        _ => panic!("unknown fixture"),
    };
    let m = e.compile(&bytes).unwrap();
    let mut s = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
    drop(m);
    (s, i)
}
fn run(s: &mut Store, i: Instance, name: &str, args: &[Value]) -> Result<RunOutcome> {
    let External::Func(f) = i.export(s, name).unwrap() else {
        panic!()
    };
    s.invoke(f, args)
}
#[test]
fn p024_overlap_and_zero() {
    let (mut s, i) = load("overlap_multivalue");
    assert_eq!(
        run(&mut s, i, "pair", &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(7), Value::I64(9)])
    );
    assert_eq!(
        run(&mut s, i, "zero", &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(11)])
    );
}
#[test]
fn p024_loop_and_table_depth() {
    let (mut s, i) = load("loop_params_br_table");
    for n in [0, 17] {
        assert_eq!(
            run(&mut s, i, "loop_params", &[Value::I32(n)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(0), Value::I64(i64::from(n))])
        );
    }
    for (choice, path) in [(0, 1), (2, 0)] {
        assert_eq!(
            run(&mut s, i, "table", &[Value::I32(choice)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(7), Value::I64(70)])
        );
        let External::Global(g) = i.export(&s, "path").unwrap() else {
            panic!()
        };
        assert_eq!(s.get_global(g).unwrap(), Value::I32(path));
    }
}
#[test]
fn p024_nested_catch_order() {
    let (mut s, i) = load("nested_catch_order");
    for (n, want) in [(0, 1105), (1, 1117)] {
        assert_eq!(
            run(&mut s, i, "run", &[Value::I32(n)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(want)])
        );
    }
}
#[test]
fn p024_captured_module_lifetime() {
    for name in ["captured_try_table_one", "captured_try_table_four"] {
        let (mut s, i) = load(name);
        let RunOutcome::Suspended {
            payload,
            continuation,
            ..
        } = run(&mut s, i, "run", &[]).unwrap()
        else {
            panic!()
        };
        assert_eq!(payload, vec![Value::I32(7)]);
        let bytes = s.continuation_bytes();
        assert!(bytes > 0);
        let root = s.root(continuation).unwrap();
        let arg = Value::Ref(RefValue::Gc(root.reference()));
        s.release_published(&[arg]);
        assert!(s.collect().unwrap().live > 0);
        assert_eq!(
            run(&mut s, i, "resume", &[arg]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(29)])
        );
        assert_eq!(
            run(&mut s, i, "resume", &[arg]).unwrap_err().kind,
            ErrorKind::Trap
        );
        drop(root);
        s.collect().unwrap();
        assert_eq!(s.continuation_bytes(), 0);
        println!("{name}: capture bytes {bytes}");
    }
}

#[test]
fn p024_restored_captures_account_for_owned_catches() {
    let mut sizes = vec![];
    for name in ["captured_try_table_one", "captured_try_table_four"] {
        let (mut store, instance) = load(name);
        assert!(matches!(
            run(&mut store, instance, "run", &[]).unwrap(),
            RunOutcome::Suspended { .. }
        ));
        sizes.push(store.continuation_bytes());
    }
    assert!(sizes[0] > 0);
    // Rejected candidate shared these payloads. The restored predecessor owns them.
    assert!(sizes[1] > sizes[0], "owned catch payloads must be charged");
}
#[test]
fn p024_restored_try_table_normal_path_keeps_result() {
    let (mut store, instance) = load("try_table_normal");
    #[cfg(feature = "performance-counters")]
    store.reset_performance_counters();
    assert_eq!(
        run(&mut store, instance, "run", &[Value::I32(32)]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(496)])
    );
    #[cfg(feature = "performance-counters")]
    {
        let counters = store.performance_counters();
        assert!(counters.branch_result_allocations > 0);
        assert!(counters.instruction_clone_bytes > 0);
        assert_eq!(counters.source_dispatches, counters.instruction_clone_events);
    }
}
#[cfg(feature = "execution-reference")]
#[test]
fn p024_reference_captures_keep_owned_catch_accounting() {
    let engine = Engine::with_execution_policy(
        Config {
            features: FeatureSet::all_phase3_core(),
            ..Config::default()
        },
        ExecutionPolicy::Reference,
    )
    .unwrap();
    let mut sizes = vec![];
    for bytes in [
        include_bytes!("../fixtures/performance/branch/captured_try_table_one.wasm").as_slice(),
        include_bytes!("../fixtures/performance/branch/captured_try_table_four.wasm").as_slice(),
    ] {
        let module = engine.compile(bytes).unwrap();
        let mut store = Store::new(&engine);
        let instance = Linker::new(&engine)
            .instantiate(&mut store, &module)
            .unwrap();
        drop(module);
        let RunOutcome::Suspended { continuation, .. } =
            run(&mut store, instance, "run", &[]).unwrap()
        else {
            panic!()
        };
        sizes.push(store.continuation_bytes());
        assert_eq!(
            run(
                &mut store,
                instance,
                "resume",
                &[Value::Ref(RefValue::Gc(continuation))]
            )
            .unwrap(),
            RunOutcome::Returned(vec![Value::I32(29)])
        );
    }
    assert!(
        sizes[1] > sizes[0],
        "frozen reference catch vectors remain owned and charged"
    );
}

#[test]
fn p024_deep_captured_handlers_keep_order_and_release_bytes() {
    let engine = Engine::new(Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    })
    .unwrap();
    let mut sizes = vec![];
    for depth in [1, 4, 5] {
        let bytes = match depth {
            1 => include_bytes!("../fixtures/performance/branch/try_table_depth_1.wasm").as_slice(),
            4 => include_bytes!("../fixtures/performance/branch/try_table_depth_4.wasm").as_slice(),
            5 => include_bytes!("../fixtures/performance/branch/try_table_depth_5.wasm").as_slice(),
            _ => unreachable!(),
        };
        let module = engine.compile(&bytes).unwrap();
        let mut store = Store::new(&engine);
        let i = Linker::new(&engine)
            .instantiate(&mut store, &module)
            .unwrap();
        drop(module);
        let External::Func(f) = i.export(&store, "run").unwrap() else {
            panic!()
        };
        let RunOutcome::Suspended {
            payload,
            continuation,
            ..
        } = store.invoke(f, &[]).unwrap()
        else {
            panic!()
        };
        assert_eq!(payload, vec![Value::I32(7)]);
        sizes.push(store.continuation_bytes());
        let root = store.root(continuation).unwrap();
        let arg = Value::Ref(RefValue::Gc(root.reference()));
        store.release_published(&[arg]);
        store.collect().unwrap();
        let External::Func(resume) = i.export(&store, "resume").unwrap() else {
            panic!()
        };
        assert_eq!(
            store.invoke(resume, &[arg]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(29)])
        );
        drop(root);
        store.collect().unwrap();
        assert_eq!(store.continuation_bytes(), 0);
    }
    println!("depth 1/4/5 captured bytes: {sizes:?}");
    assert!(sizes[0] < sizes[1] && sizes[1] <= sizes[2]);
}
