use wtime_runtime::*;
fn load(name: &str) -> (Store, Instance) {
    let e = Engine::new(Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    })
    .unwrap();
    let bytes: &[u8] = match name {
        "overlap_forward" => include_bytes!("../fixtures/performance/p027/overlap_forward.wasm"),
        "overlap_backward" => include_bytes!("../fixtures/performance/p027/overlap_backward.wasm"),
        "oob_precheck" => include_bytes!("../fixtures/performance/p027/oob_precheck.wasm"),
        "copy_distinct" => include_bytes!("../fixtures/performance/p027/copy_distinct.wasm"),
        "fill" => include_bytes!("../fixtures/performance/p027/fill.wasm"),
        "distinct_table_gc_copy" => {
            include_bytes!("../fixtures/performance/p027/distinct_table_gc_copy.wasm")
        }
        "shared_two_imports" => {
            include_bytes!("../fixtures/performance/p027/shared_two_imports.wasm")
        }
        _ => panic!("unknown P027 fixture"),
    };
    let module = e.compile(&bytes).unwrap();
    let mut s = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut s, &module).unwrap();
    (s, i)
}
fn memory(s: &Store, i: Instance, name: &str) -> Memory {
    let External::Memory(m) = i.export(s, name).unwrap() else {
        panic!()
    };
    m
}
fn run(s: &mut Store, i: Instance, args: &[Value]) -> Result<RunOutcome> {
    #[cfg(feature = "performance-counters")]
    s.reset_performance_counters();
    let External::Func(f) = i.export(s, "run").unwrap() else {
        panic!()
    };
    s.invoke(f, args)
}
fn pattern(n: usize, mul: usize, add: usize) -> Vec<u8> {
    (0..n)
        .map(|i| {
            (i.wrapping_mul(mul)
                .wrapping_add(add)
                .wrapping_add((i >> 16) * if mul == 29 { 53 } else { 37 })
                & 255) as u8
        })
        .collect()
}
#[test]
fn p027_overlap_full_memory_images() {
    for (name, src, dst) in [("overlap_forward", 0, 1), ("overlap_backward", 1, 0)] {
        let (mut s, i) = load(name);
        let m = memory(&s, i, "memory");
        let before = pattern(65536, 29, 7);
        s.write_memory(m, 0, &before).unwrap();
        let mut expected = before.clone();
        expected[dst..dst + 4096].copy_from_slice(&before[src..src + 4096]);
        assert_ne!(expected, before);
        assert_eq!(run(&mut s, i, &[]).unwrap(), RunOutcome::Returned(vec![]));
        copy_path(&s, 1);
        assert_eq!(s.read_memory(m, 0, 65536).unwrap(), expected);
    }
}
#[test]
fn p027_oob_precheck_preserves_prior_full_memory() {
    let (mut s, i) = load("oob_precheck");
    let m = memory(&s, i, "memory");
    let mut before = vec![0; 65536];
    before[..4].copy_from_slice(&0x11223344u32.to_le_bytes());
    before[65532..].copy_from_slice(&0x55667788u32.to_le_bytes());
    s.write_memory(m, 0, &before).unwrap();
    let error = run(&mut s, i, &[]).unwrap_err();
    assert_eq!(error.trap_code, Some(TrapCode::MemoryOutOfBounds));
    assert_eq!(s.read_memory(m, 0, 65536).unwrap(), before);
}
#[test]
fn p027_distinct_memory_copy_changes_destination_and_preserves_source() {
    for n in [64, 4096, 1048576] {
        let (mut s, i) = load("copy_distinct");
        let src = memory(&s, i, "source");
        let dst = memory(&s, i, "memory");
        let source = pattern(1048576, 29, 7);
        let before = pattern(1114112, 13, 101);
        s.write_memory(src, 0, &source).unwrap();
        s.write_memory(dst, 0, &before).unwrap();
        let mut expected = before.clone();
        expected[65536..65536 + n].copy_from_slice(&source[..n]);
        assert_ne!(expected, before);
        for repetitions in [1, 3] {
            assert_eq!(
                run(&mut s, i, &[Value::I32(n as i32), Value::I32(repetitions)]).unwrap(),
                RunOutcome::Returned(vec![])
            );
            copy_path(&s, ((n + 65535) / 65536 * repetitions as usize) as u64);
            assert_eq!(s.read_memory(src, 0, source.len()).unwrap(), source);
            assert_eq!(s.read_memory(dst, 0, expected.len()).unwrap(), expected);
        }
    }
}

fn named(s: &mut Store, i: Instance, name: &str, args: &[Value]) -> Result<RunOutcome> {
    let External::Func(f) = i.export(s, name).unwrap() else {
        panic!()
    };
    #[cfg(feature = "performance-counters")]
    s.reset_performance_counters();
    s.invoke(f, args)
}
#[test]
fn p027_distinct_table_copy_roots_references_across_chunk_boundary() {
    let (mut s, i) = load("distinct_table_gc_copy");
    named(&mut s, i, "seed", &[]).unwrap();
    named(&mut s, i, "copy_all", &[]).unwrap();
    table_path(&s, 2);
    named(&mut s, i, "clear_source", &[]).unwrap();
    assert_eq!(s.collect().unwrap().live, 2);
    for (index, expected) in [(0, 73), (4096, 91)] {
        assert_eq!(
            named(&mut s, i, "read_destination", &[Value::I32(index)]).unwrap(),
            RunOutcome::Returned(vec![Value::I32(expected)])
        );
    }
    named(&mut s, i, "clear_destination", &[]).unwrap();
    let stats = s.collect().unwrap();
    assert_eq!(stats.live, 0);
    assert_eq!(stats.reclaimed, 2);
}
#[test]
fn p027_distinct_shared_capabilities_keep_aliased_memmove_semantics() {
    for (name, src, dst) in [("copy_forward", 0usize, 1usize), ("copy_backward", 1, 0)] {
        let e = Engine::new(Config {
            features: FeatureSet::all_phase3_core(),
            ..Config::default()
        })
        .unwrap();
        let module = e
            .compile(include_bytes!(
                "../fixtures/performance/p027/shared_two_imports.wasm"
            ))
            .unwrap();
        let mut s = Store::new(&e);
        let memory = s
            .create_memory(MemoryType {
                limits: Limits::new(2, Some(2)),
                shared: true,
                ..Default::default()
            })
            .unwrap();
        let token = s.share_memory(memory).unwrap();
        let left = s.import_shared_memory(&token).unwrap();
        let right = s.import_shared_memory(&token).unwrap();
        let mut linker = Linker::new(&e);
        linker
            .define("mem", "left", External::Memory(left))
            .unwrap();
        linker
            .define("mem", "right", External::Memory(right))
            .unwrap();
        let i = linker.instantiate(&mut s, &module).unwrap();
        let original: Vec<u8> = (0..131072usize)
            .map(|j| (j * 29 + 7 + (j >> 16) * 53) as u8)
            .collect();
        s.write_memory(memory, 0, &original).unwrap();
        let mut expected = original.clone();
        expected[dst..dst + 100000].copy_from_slice(&original[src..src + 100000]);
        named(&mut s, i, name, &[Value::I32(100000)]).unwrap();
        shared_path(&s, 2);
        assert_eq!(s.read_memory(left, 0, 131072).unwrap(), expected);
        assert_eq!(s.read_memory(right, 0, 131072).unwrap(), expected);
    }
}

#[test]
fn p027_mixed_memory64_custom_page_copy_prechecks_both_full_ranges() {
    let e = Engine::new(Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    })
    .unwrap();
    let module = e
        .compile(include_bytes!("../fixtures/memory-proposals.wasm"))
        .unwrap();
    let mut s = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut s, &module).unwrap();
    let bytes = memory(&s, i, "bytes");
    let wide = memory(&s, i, "wide");
    let source = pattern(65536, 29, 7);
    s.write_memory(wide, 0, &source).unwrap();
    let before = [101, 114, 127, 140];
    s.write_memory(bytes, 0, &before).unwrap();
    for args in [
        [Value::I32(1), Value::I64(32), Value::I32(4)],
        [Value::I32(0), Value::I64(65534), Value::I32(4)],
    ] {
        let error = named(&mut s, i, "copy", &args).unwrap_err();
        assert_eq!(error.trap_code, Some(TrapCode::MemoryOutOfBounds));
        assert_eq!(s.read_memory(bytes, 0, 4).unwrap(), before);
        assert_eq!(s.read_memory(wide, 0, 65536).unwrap(), source);
    }
    named(
        &mut s,
        i,
        "copy",
        &[Value::I32(0), Value::I64(32), Value::I32(4)],
    )
    .unwrap();
    copy_path(&s, 1);
    assert_eq!(s.read_memory(bytes, 0, 4).unwrap(), source[32..36]);
    assert_eq!(s.read_memory(wide, 0, 65536).unwrap(), source);
    named(
        &mut s,
        i,
        "copy",
        &[Value::I32(4), Value::I64(65536), Value::I32(0)],
    )
    .unwrap();
    assert_eq!(s.read_memory(bytes, 0, 4).unwrap(), source[32..36]);
}

#[cfg(feature = "performance-counters")]
const EXPECT_DIRECT_BULK: bool = false;
/// Unshared memory.copy and memory.fill copy slices directly (one reused
/// buffer between two distinct memories); tables still snapshot.
#[cfg(feature = "performance-counters")]
const EXPECT_DIRECT_MEMORY_BULK: bool = true;
fn copy_path(_s: &Store, _chunks: u64) {
    #[cfg(feature = "performance-counters")]
    {
        let c = _s.performance_counters();
        assert_eq!(
            c.read_vec_operations,
            if EXPECT_DIRECT_MEMORY_BULK { 0 } else { _chunks }
        );
        assert_eq!(
            c.bulk_slice_chunks,
            if EXPECT_DIRECT_MEMORY_BULK { _chunks } else { 0 }
        );
        assert_eq!(c.table_copy_snapshot_chunks, 0);
    }
}
fn table_path(_s: &Store, _chunks: u64) {
    #[cfg(feature = "performance-counters")]
    {
        let c = _s.performance_counters();
        assert_eq!(
            c.table_copy_snapshot_chunks,
            if EXPECT_DIRECT_BULK { 0 } else { _chunks }
        );
        assert_eq!(
            c.bulk_slice_chunks,
            if EXPECT_DIRECT_BULK { _chunks } else { 0 }
        );
        assert_eq!(c.read_vec_operations, 0);
    }
}
fn shared_path(_s: &Store, _chunks: u64) {
    #[cfg(feature = "performance-counters")]
    {
        let c = _s.performance_counters();
        assert_eq!(c.read_vec_operations, _chunks);
        assert_eq!(c.bulk_slice_chunks, 0);
    }
}
#[test]
fn p027_fill_exact_images_zero_odd_and_multiple_chunks() {
    for n in [0usize, 17, 64, 4096, 1048576] {
        let (mut s, i) = load("fill");
        let m = memory(&s, i, "memory");
        let original = pattern(1114112, 13, 101);
        s.write_memory(m, 0, &original).unwrap();
        let mut expected = original.clone();
        expected[4096..4096 + n].fill(90);
        for reps in [1usize, 3] {
            assert_eq!(
                run(&mut s, i, &[Value::I32(n as i32), Value::I32(reps as i32)]).unwrap(),
                RunOutcome::Returned(vec![])
            );
            #[cfg(feature = "performance-counters")]
            {
                let c = s.performance_counters();
                assert_eq!(c.read_vec_operations, 0);
                assert_eq!(
                    c.bulk_slice_chunks,
                    if EXPECT_DIRECT_MEMORY_BULK {
                        ((n + 65535) / 65536 * reps) as u64
                    } else {
                        0
                    }
                );
            }
            assert_eq!(s.read_memory(m, 0, 1114112).unwrap(), expected);
        }
    }
}

#[cfg(feature = "execution-reference")]
#[test]
fn p027_reference_distinct_copy_uses_same_bounded_backend() {
    let e = Engine::with_execution_policy(
        Config {
            features: FeatureSet::all_phase3_core(),
            ..Config::default()
        },
        ExecutionPolicy::Reference,
    )
    .unwrap();
    let module = e
        .compile(include_bytes!(
            "../fixtures/performance/p027/copy_distinct.wasm"
        ))
        .unwrap();
    let mut s = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut s, &module).unwrap();
    let source = memory(&s, i, "source");
    let destination = memory(&s, i, "memory");
    let original = pattern(1048576, 29, 7);
    let mut expected = pattern(1114112, 13, 101);
    s.write_memory(source, 0, &original).unwrap();
    s.write_memory(destination, 0, &expected).unwrap();
    expected[65536..65536 + 131089].copy_from_slice(&original[..131089]);
    run(&mut s, i, &[Value::I32(131089), Value::I32(1)]).unwrap();
    copy_path(&s, 3);
    assert_eq!(s.read_memory(source, 0, 1048576).unwrap(), original);
    assert_eq!(s.read_memory(destination, 0, 1114112).unwrap(), expected);
}
