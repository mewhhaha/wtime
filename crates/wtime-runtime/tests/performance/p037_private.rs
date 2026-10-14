// SPDX-License-Identifier: Apache-2.0 OR MIT
// Paste/include as a child of source_map.rs, NOT external integration scope.
#[cfg(test)]
mod p037_tests {
    use super::*;
    fn module(bytes: &[u8]) -> Module {
        let mut config = Config::default();
        config.features = FeatureSet::all_phase3_core();
        Engine::new(config).unwrap().compile(bytes).unwrap()
    }
    // Original small binary generator; normal admission precedes all map checks.
    fn tiny(types: &[u8], bodies: &[&[u8]]) -> Module {
        let mut bytes = b"\0asm\x01\0\0\0".to_vec();
        bytes.extend([1, u8::try_from(types.len()).unwrap()]);
        bytes.extend_from_slice(types);
        bytes.extend([3, u8::try_from(bodies.len() + 1).unwrap(), u8::try_from(bodies.len()).unwrap()]);
        bytes.extend(std::iter::repeat_n(0, bodies.len()));
        let mut code = vec![u8::try_from(bodies.len()).unwrap()];
        for body in bodies {
            code.push(u8::try_from(body.len()).unwrap());
            code.extend_from_slice(body);
        }
        bytes.extend([10, u8::try_from(code.len()).unwrap()]);
        bytes.extend(code);
        module(&bytes)
    }
    fn fixture() -> Module {
        // Root must encode/validate and install this fixture before compiling.
        module(include_bytes!("../fixtures/performance/p037/control-edges.wasm"))
    }
    fn function(module: &Module) -> CompactFunction {
        CompactFunction::from_module(module, 0, MAX_WORK, MAX_BYTES).unwrap()
    }
    #[test]
    fn p037_independent_control_edges_identity_and_terminal() {
        let module=fixture(); let function=function(&module);
        let map=SourceMap::bounded(&function,&module,MAX_WORK,MAX_BYTES).unwrap();
        // Hand-derived WAT instruction ordinals, including implicit function end.
        let expected: &[&[usize]]=&[
            &[1], &[2], &[3], &[4,14], &[5], &[6,8], &[7], &[9],
            &[9], &[10], &[11], &[2,14,16,14], &[13], &[14], &[16], &[16],
        ];
        assert_eq!(module.data.inner.decoded().code[0].instructions.len(),16);
        for (pc, targets) in expected.iter().enumerate() {
            assert_eq!(map.targets(pc),Some(*targets),"pc {pc}");
            assert_eq!(map.events(pc),Some(pc..pc+1));
            assert_eq!(map.destination(pc),Some(pc));
            assert_eq!(map.next_source(pc),Some(pc));
        }
        assert_eq!(map.destination(16),Some(16));
        assert_eq!(map.next_source(16),Some(16));
        for pc in [16,17,usize::MAX] {assert_eq!(map.events(pc),None);assert_eq!(map.targets(pc),None);}
        assert_eq!(map.destination(usize::MAX),None);
        assert_eq!(map.next_source(usize::MAX),None);
        assert!(map.verify(&function,&module,MAX_WORK,MAX_BYTES));
        // const; if without else; nop; end; return_call; function end.
        let tail = tiny(&[1,0x60,0,0], &[&[0,0x41,0,4,0x40,1,0x0b,0x12,1,0x0b], &[0,1,0x0b]]);
        let compact = self::function(&tail);
        let map = SourceMap::bounded(&compact,&tail,MAX_WORK,MAX_BYTES).unwrap();
        for (pc, targets) in [&[1][..], &[2,3], &[3], &[4], &[6], &[6]].iter().enumerate() {
            assert_eq!(map.targets(pc), Some(*targets), "no-else/tail pc {pc}");
        }
        assert!(map.verify(&compact,&tail,MAX_WORK,MAX_BYTES));
    }
    #[test]
    fn p037_corrupted_maps_and_wrong_module_identity_reject() {
        let module=fixture();let function=function(&module);
        let make=||SourceMap::bounded(&function,&module,MAX_WORK,MAX_BYTES).unwrap();
        let mut map=make();map.source_to_compact[0]=1;
        assert!(!map.verify(&function,&module,MAX_WORK,MAX_BYTES));
        let mut map=make();map.targets[0]=usize::MAX;
        assert!(!map.verify(&function,&module,MAX_WORK,MAX_BYTES));
        let mut map=make();map.successors[0].first=usize::MAX;
        assert_eq!(map.targets(0),None);
        assert!(!map.verify(&function,&module,MAX_WORK,MAX_BYTES));
        let other=fixture(); // Equal bytes, different Engine and Arc identity.
        assert!(!make().verify(&function,&other,MAX_WORK,MAX_BYTES));
        let mut map=make();map.source=Arc::from(map.source.as_ref());
        assert!(!map.verify(&function,&module,MAX_WORK,MAX_BYTES));
        let mut map=make();map.engine=map.engine.wrapping_add(1);
        assert!(!map.verify(&function,&module,MAX_WORK,MAX_BYTES));
    }
    #[test]
    fn p037_aggregate_limits_and_absolute_ceilings_reject() {
        let module=fixture();let function=function(&module);
        let map=SourceMap::bounded(&function,&module,MAX_WORK,MAX_BYTES).unwrap();
        assert!(matches!(SourceMap::bounded(&function,&module,function.work,MAX_BYTES),Err(Declined::Budget)));
        assert!(matches!(SourceMap::bounded(&function,&module,MAX_WORK,function.retained_bytes),Err(Declined::Budget)));
        assert!(!map.verify(&function,&module,map.work,map.retained_bytes));
        let mut function=function;
        function.work=MAX_WORK+1;
        assert!(matches!(SourceMap::bounded(&function,&module,usize::MAX,usize::MAX),Err(Declined::Budget)));
        function.work=0;function.retained_bytes=MAX_BYTES+1;
        assert!(matches!(SourceMap::bounded(&function,&module,usize::MAX,usize::MAX),Err(Declined::Budget)));
        // Dynamic handlers/captured origins and typed reference branches must
        // decline whole admitted functions, without guessed sequential edges.
        let reference_null = tiny(&[1,0x60,1,0x6f,0], &[&[0,2,0x40,0x20,0,0xd5,0,0x1a,0x0b,0x0b]]);
        let reference_nonnull = tiny(&[1,0x60,1,0x6f,1,0x6f], &[&[0,2,0x6f,0x20,0,0xd6,0,0xd0,0x6f,0x0b,0x0b]]);
        for (unsupported, body) in [
            (reference_null, 0), (reference_nonnull, 0),
            (self::module(include_bytes!("../fixtures/performance/branch/try_table_normal.wasm")), 0),
            (self::module(include_bytes!("../fixtures/performance/compact/continuation_root.wasm")), 1),
            (self::module(include_bytes!("../fixtures/performance/compact/cast_branch.wasm")), 0),
        ] {
            let compact = CompactFunction::from_module(&unsupported, body, MAX_WORK, MAX_BYTES).unwrap();
            let owners = Arc::strong_count(&compact.source);
            assert!(matches!(SourceMap::bounded(&compact,&unsupported,MAX_WORK,MAX_BYTES), Err(Declined::Unsupported)));
            assert_eq!(Arc::strong_count(&compact.source), owners);
        }
    }
}
