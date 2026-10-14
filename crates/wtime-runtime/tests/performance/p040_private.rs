// Included as a child of lower_control.rs, with access to unpublished records.
#[cfg(test)]
mod p040_private {
    use super::*;
    fn integer(value: &ValType) -> bool { matches!(value, ValType::I32 | ValType::I64) }
    fn module(bytes: &[u8]) -> Module { Engine::new(Config::default()).unwrap().compile(bytes).unwrap() }
    fn loop_module() -> Module { module(include_bytes!("../fixtures/performance/p040/multivalue_loop.wasm")) }
    fn function(module: &Module) -> CompactFunction { CompactFunction::from_module(module, 0, MAX_WORK, MAX_BYTES).unwrap() }
    fn plans(function: &CompactFunction, module: &Module) -> ControlPlans {
        ControlPlans::prepare(function, module, integer, MAX_WORK, MAX_BYTES).unwrap()
    }
    #[test]
    fn p040_private_records_match_independent_block_arities_and_targets() {
        let module = loop_module(); let function = function(&module); let plans = plans(&function, &module);
        assert_eq!(plans.blocks.len(), 16);
        assert_eq!(plans.blocks[2], Some(BlockRecord { params: 2, results: 2, end: 14, other: NO_ELSE }));
        assert_eq!(plans.blocks[3], Some(BlockRecord { params: 2, results: 2, end: 13, other: NO_ELSE }));
        assert_eq!(plans.blocks.iter().filter(|record| record.is_some()).count(), 2);
        assert!(plans.verify(&function, &module, integer, MAX_WORK, MAX_BYTES));
        let module = self::module(include_bytes!("../fixtures/performance/p040/br_table_three_paths.wasm"));
        let function = self::function(&module); let plans = self::plans(&function, &module);
        assert_eq!(plans.blocks[0], Some(BlockRecord { params: 0, results: 1, end: 8, other: NO_ELSE }));
        assert_eq!(plans.blocks[1], Some(BlockRecord { params: 0, results: 1, end: 5, other: NO_ELSE }));
        let map = super::super::source_map::SourceMap::bounded(&function, &module, MAX_WORK, MAX_BYTES).unwrap();
        assert_eq!(map.targets(4), Some([6, 9, 12].as_slice()));
        assert!(plans.verify(&function, &module, integer, MAX_WORK, MAX_BYTES));
    }
    #[test]
    fn p040_private_corruption_and_wrong_source_identity_reject_before_publication() {
        let module = loop_module(); let function = function(&module);
        for damage in 0..10 {
            let mut plans = plans(&function, &module);
            match damage {
                0 => plans.blocks[2].as_mut().unwrap().params = 1,
                1 => plans.blocks[2].as_mut().unwrap().results = 1,
                2 => plans.blocks[2].as_mut().unwrap().end = 13,
                3 => plans.blocks[2].as_mut().unwrap().other = 4,
                4 => plans.blocks[2] = None,
                5 => plans.blocks[4] = plans.blocks[2],
                6 => plans.work += 1,
                7 => plans.retained_bytes += 1,
                8 => plans.engine = plans.engine.wrapping_add(1),
                _ => plans.body += 1,
            }
            assert!(!plans.verify(&function, &module, integer, MAX_WORK, MAX_BYTES), "damage {damage}");
        }
        let mut plans = plans(&function, &module);
        plans.source = Arc::from(plans.source.as_ref());
        assert!(!plans.verify(&function, &module, integer, MAX_WORK, MAX_BYTES));
        let other = loop_module();
        assert!(!self::plans(&function, &module).verify(&function, &other, integer, MAX_WORK, MAX_BYTES));
    }
    #[test]
    fn p040_private_work_bytes_and_indexed_arity_are_bounded_before_traversal() {
        let module = loop_module(); let function = function(&module);
        assert!(matches!(ControlPlans::prepare(&function, &module, integer, 0, MAX_BYTES), Err(Declined::Budget)));
        assert!(matches!(ControlPlans::prepare(&function, &module, integer, MAX_WORK, 0), Err(Declined::Budget)));
        let plans = ControlPlans::prepare(&function, &module, integer, usize::MAX, usize::MAX).unwrap();
        assert!(plans.work <= MAX_WORK && plans.retained_bytes <= MAX_BYTES);
        assert!(!plans.verify(&function, &module, integer, plans.work - 1, MAX_BYTES));
        assert!(!plans.verify(&function, &module, integer, MAX_WORK, plans.retained_bytes));
        let immediate = &module.data.inner.decoded().code[0].instructions[2].immediate;
        let mut work = 3;
        assert_eq!(arities(immediate, &module, integer, &mut work), Err(Declined::Budget));
        let mut work = 4;
        assert_eq!(arities(immediate, &module, integer, &mut work), Ok((2, 2)));
        assert_eq!(work, 0);
        let mut work = 4;
        assert_eq!(arities(immediate, &module, |_| false, &mut work), Err(Declined::Unsupported));
    }
}
