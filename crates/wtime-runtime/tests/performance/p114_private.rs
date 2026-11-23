fn admitted(bytes: &[u8]) -> (Module, CallTypes) {
    let engine = Engine::new(Config { features: FeatureSet::all_phase3_core(), ..Config::default() }).unwrap();
    let module = engine.compile(bytes).unwrap();
    let calls = CallTypes::prepare(&module, MAX_WORK, MAX_BYTES).unwrap();
    (module, calls)
}
const GC_PREFIX: &[u8] = include_bytes!("../fixtures/performance/bench_wasm/integration_p113/integration_gc_prefix.wasm");

#[test]
fn p114_exact_prefix_exit_and_entry_decline_are_distinct() {
    let (module, calls) = admitted(GC_PREFIX);
    let prefix = Prefix::prepare(&module, 0, &calls, MAX_WORK, MAX_BYTES).unwrap();
    assert_eq!(prefix.ops.len(), 67);
    assert_eq!(prefix.exit.pc, 67);
    assert_eq!(prefix.exit.opcode, 0xfb00);
    assert!(prefix.verify(&module, &calls, MAX_WORK, MAX_BYTES));
    let (module, calls) = admitted(include_bytes!("../fixtures/performance/bench_wasm/integration_p113/integration_ref_entry_control.wasm"));
    assert!(matches!(Prefix::prepare(&module, 0, &calls, MAX_WORK, MAX_BYTES), Err(Declined::Unsupported)));
}

#[test]
fn p114_corrupt_identity_records_exit_and_accounting_fail_closed() {
    let (module, calls) = admitted(GC_PREFIX);
    let (foreign, _) = admitted(GC_PREFIX);
    let prefix = Prefix::prepare(&module, 0, &calls, MAX_WORK, MAX_BYTES).unwrap();
    assert!(!prefix.verify(&foreign, &calls, MAX_WORK, MAX_BYTES));
    for case in 0..11 {
        let mut prefix = Prefix::prepare(&module, 0, &calls, MAX_WORK, MAX_BYTES).unwrap();
        match case {
            0 => prefix.engine ^= 1,
            1 => prefix.source = Arc::from(prefix.source.as_ref()),
            2 => prefix.body = usize::MAX,
            3 => prefix.type_index = usize::MAX,
            4 => prefix.ops[0].aux ^= 1,
            5 => prefix.exit.pc -= 1,
            6 => prefix.exit.opcode ^= 1,
            7 => prefix.exit.fuel = 0,
            8 => prefix.exit.checkpoint = false,
            9 => prefix.work = usize::MAX,
            10 => prefix.bytes += 1,
            _ => unreachable!(),
        }
        assert!(!prefix.verify(&module, &calls, MAX_WORK, MAX_BYTES), "corruption {case}");
    }
}

#[test]
fn p114_actual_capacity_work_and_overflow_are_bounded() {
    let (module, calls) = admitted(GC_PREFIX);
    let published_before = module.data.compact.is_some();
    let prefix = Prefix::prepare(&module, 0, &calls, MAX_WORK, MAX_BYTES).unwrap();
    assert!(matches!(Prefix::prepare(&module, 0, &calls, prefix.work - 1, MAX_BYTES), Err(Declined::Budget)));
    assert!(matches!(Prefix::prepare(&module, 0, &calls, MAX_WORK, prefix.bytes - 1), Err(Declined::Budget)));
    assert_eq!(cost(usize::MAX, 0), None);
    assert_eq!(cost(0, usize::MAX), None);
    assert_eq!(storage(usize::MAX), None);
    assert_eq!(module.data.compact.is_some(), published_before); // Factory failures never change publication.
}
