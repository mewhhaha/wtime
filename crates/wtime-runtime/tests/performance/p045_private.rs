fn p045_module(bytes: &[u8]) -> (Engine, Module) {
    let engine = Engine::with_execution_policy(Config {
        features: FeatureSet::all_phase3_core(), ..Config::default()
    }, ExecutionPolicy::Compact).unwrap();
    let module = engine.compile(bytes).unwrap();
    (engine, module)
}
fn p045_valid(prepared: &PreparedModule, module: &Module) -> bool {
    let calls = super::super::super::calls::CallTypes::prepare(module, MAX_WORK, MAX_BYTES).unwrap();
    let memories = super::super::memory::MemoryTypes::prepare(module, MAX_WORK, MAX_BYTES).unwrap();
    prepared.verify(module, &calls, &memories)
}
const P045_WHOLE: &[u8] = include_bytes!("../fixtures/performance/p043/scalar-widths.wasm");
const P045_PREFIX: &[u8] = include_bytes!("../fixtures/performance/p115/handoff.wasm");

#[test]
fn p045_aggregate_whole_and_prefix_certificates_reject_corruption() {
    let (_, module) = p045_module(P045_WHOLE);
    for case in 0..18 {
        let mut prepared = PreparedModule::prepare(&module).unwrap();
        assert!(p045_valid(&prepared, &module));
        if case < 9 {
            let PreparedFunction::Whole(p) = prepared.functions[0].as_mut().unwrap() else { panic!() };
            match case {
                0 => p.function.ops[0].form = u16::MAX,
                1 => p.function.ops.iter_mut().find(|op| op.form == SIDE).unwrap().aux = u32::MAX,
                2 => p.function.origins[0].checkpoint = false,
                3 => p.function.origins[0].fuel = 0,
                4 => p.function.source = Arc::from(p.function.source.as_ref()),
                5 => p.map.p045_corrupt(0),
                6 => p.control.p045_corrupt(),
                7 => p.memory.as_mut().unwrap().p045_corrupt(),
                8 => p.has_calls = !p.has_calls,
                _ => unreachable!(),
            }
        } else {
            match case {
                9 => prepared.retained_bytes = usize::MAX,
                10 => prepared.attempted_work += 1,
                11 => prepared.declined += 1,
                12 => { prepared.functions.pop(); }
                13 => prepared.functions.swap(0, 1),
                14..=16 => {
                    let PreparedFunction::Whole(p) = prepared.functions[0].as_mut().unwrap() else { panic!() };
                    p.map.p045_corrupt(case - 13);
                }
                17 => {
                    let PreparedFunction::Whole(p) = prepared.functions[0].as_mut().unwrap() else { panic!() };
                    p.function.origins[0].effect = classify(0xe6);
                }
                _ => unreachable!(),
            }
        }
        assert!(!p045_valid(&prepared, &module), "whole corruption {case}");
    }
    let (_, module) = p045_module(P045_PREFIX);
    for case in 0..6 {
        let mut prepared = PreparedModule::prepare(&module).unwrap();
        let prefix = prepared.functions.iter_mut().flatten().find_map(|p| match p {
            PreparedFunction::Prefix(p) => Some(p), _ => None
        }).unwrap();
        prefix.p045_corrupt(case);
        assert!(!p045_valid(&prepared, &module), "prefix corruption {case}");
    }
    let prepared = PreparedModule::prepare(&module).unwrap();
    let (_, other) = p045_module(P045_PREFIX);
    assert!(!p045_valid(&prepared, &other));
    assert_eq!(preparation_charge(&module.data.inner.decoded().code[0], usize::MAX, &module, MAX_WORK), None);
}

#[test]
fn p045_lowering_and_verifier_faults_discard_partial_publication() {
    let bytes = include_bytes!("../fixtures/performance/bench_wasm/integration_p113/integration_numeric_control.wasm");
    for fault in [verify::Fault::LowerAllocation, verify::Fault::LowerWork,
        verify::Fault::VerifyAllocation, verify::Fault::VerifyWork] {
        verify::inject(fault);
        let (engine, module) = p045_module(bytes);
        assert!(module.data.compact.is_none());
        let mut store = Store::new(&engine);
        let instance = Linker::new(&engine).instantiate(&mut store, &module).unwrap();
        let External::Func(func) = instance.export(&store, "run").unwrap() else { panic!() };
        assert_eq!(store.invoke(func, &[Value::I32(32)]).unwrap(), RunOutcome::Returned(vec![Value::I32(64)]));
        #[cfg(feature = "execution-reference")]
        assert_eq!(store.compact_dispatches(), 0);
    }
}

#[test]
fn p045_all_declines_and_feature_disabled_input_preserve_validation() {
    let (_, module) = p045_module(include_bytes!("../fixtures/performance/bench_wasm/integration_p113/integration_ref_entry_control.wasm"));
    let source_count = Arc::strong_count(&module.data.inner.decoded().bytes);
    assert!(PreparedModule::prepare(&module).is_none());
    assert_eq!(Arc::strong_count(&module.data.inner.decoded().bytes), source_count);
    let engine = Engine::with_execution_policy(Config { features: FeatureSet::mvp(), ..Config::default() }, ExecutionPolicy::Compact).unwrap();
    assert!(engine.compile(include_bytes!("../fixtures/performance/bench_wasm/integration_p113/integration_gc_prefix.wasm")).is_err());
    assert!(engine.compile(b"\0asm\x01\0\0\0\x0a\x01\xff").is_err());
    let (_, module) = p045_module(include_bytes!("../fixtures/performance/compact/long_metadata.wasm"));
    assert!(module.data.compact.is_none());
}
