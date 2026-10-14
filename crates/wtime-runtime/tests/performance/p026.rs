use wtime_runtime::*;
fn engine() -> Engine {
    Engine::new(Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    })
    .unwrap()
}
fn fixture(name: &str) -> &'static [u8] {
    match name {
        "extension_loads" => include_bytes!("../fixtures/performance/simd/extension_loads.wasm"),
        "splat_zero_loads" => include_bytes!("../fixtures/performance/simd/splat_zero_loads.wasm"),
        "lane_loads_oob" => include_bytes!("../fixtures/performance/simd/lane_loads_oob.wasm"),
        "shared_vector_loads" => {
            include_bytes!("../fixtures/performance/simd/shared_vector_loads.wasm")
        }
        _ => panic!("unknown P026 fixture: {name}"),
    }
}
fn load(name: &str) -> (Store, Instance) {
    let e = engine();
    let m = e.compile(fixture(name)).unwrap();
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
fn vector(s: &mut Store, i: Instance, name: &str, args: &[Value], want: [u8; 16]) {
    #[cfg(feature = "performance-counters")]
    s.reset_performance_counters();
    assert_eq!(
        run(s, i, name, args).unwrap(),
        RunOutcome::Returned(vec![Value::V128(V128(want))]),
        "{name}"
    );
    #[cfg(feature = "performance-counters")]
    {
        let counters = s.performance_counters();
        assert_eq!(
            counters.read_fixed_operations, 0,
            "restored allocating path: {name}"
        );
        assert_eq!(counters.read_vec_operations, 1, "restored allocating path: {name}");
    }
}
fn memory(s: &Store, i: Instance) -> Memory {
    let External::Memory(m) = i.export(s, "memory").unwrap() else {
        panic!()
    };
    m
}
#[test]
fn p026_extension_loads() {
    let (mut s, i) = load("extension_loads");
    vector(
        &mut s,
        i,
        "v128.load8x8_s",
        &[],
        [
            128, 255, 255, 255, 127, 0, 129, 255, 254, 255, 2, 0, 128, 255, 255, 255,
        ],
    );
    vector(
        &mut s,
        i,
        "v128.load8x8_u",
        &[],
        [128, 0, 255, 0, 127, 0, 129, 0, 254, 0, 2, 0, 128, 0, 255, 0],
    );
    vector(
        &mut s,
        i,
        "v128.load16x4_s",
        &[],
        [
            128, 255, 255, 255, 127, 129, 255, 255, 254, 2, 0, 0, 128, 255, 255, 255,
        ],
    );
    vector(
        &mut s,
        i,
        "v128.load16x4_u",
        &[],
        [128, 255, 0, 0, 127, 129, 0, 0, 254, 2, 0, 0, 128, 255, 0, 0],
    );
    vector(
        &mut s,
        i,
        "v128.load32x2_s",
        &[],
        [
            128, 255, 127, 129, 255, 255, 255, 255, 254, 2, 128, 255, 255, 255, 255, 255,
        ],
    );
    vector(
        &mut s,
        i,
        "v128.load32x2_u",
        &[],
        [128, 255, 127, 129, 0, 0, 0, 0, 254, 2, 128, 255, 0, 0, 0, 0],
    );
    let bytes = s.read_memory(memory(&s, i), 0, 65536).unwrap();
    assert_eq!(
        &bytes[..16],
        &[
            128, 255, 127, 129, 254, 2, 128, 255, 52, 18, 205, 171, 0, 128, 255, 127
        ]
    );
    assert!(bytes[16..].iter().all(|b| *b == 0));
}
#[test]
fn p026_splat_zero_loads() {
    let (mut s, i) = load("splat_zero_loads");
    vector(
        &mut s,
        i,
        "v128.load8_splat",
        &[],
        [
            128, 128, 128, 128, 128, 128, 128, 128, 128, 128, 128, 128, 128, 128, 128, 128,
        ],
    );
    vector(
        &mut s,
        i,
        "v128.load16_splat",
        &[],
        [
            128, 18, 128, 18, 128, 18, 128, 18, 128, 18, 128, 18, 128, 18, 128, 18,
        ],
    );
    vector(
        &mut s,
        i,
        "v128.load32_splat",
        &[],
        [
            128, 18, 52, 86, 128, 18, 52, 86, 128, 18, 52, 86, 128, 18, 52, 86,
        ],
    );
    vector(
        &mut s,
        i,
        "v128.load64_splat",
        &[],
        [
            128, 18, 52, 86, 120, 154, 188, 222, 128, 18, 52, 86, 120, 154, 188, 222,
        ],
    );
    vector(
        &mut s,
        i,
        "v128.load32_zero",
        &[],
        [128, 18, 52, 86, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    );
    vector(
        &mut s,
        i,
        "v128.load64_zero",
        &[],
        [128, 18, 52, 86, 120, 154, 188, 222, 0, 0, 0, 0, 0, 0, 0, 0],
    );
    vector(
        &mut s,
        i,
        "v128.load",
        &[],
        [
            128, 18, 52, 86, 120, 154, 188, 222, 1, 35, 69, 103, 137, 171, 205, 239,
        ],
    );
    let bytes = s.read_memory(memory(&s, i), 0, 65536).unwrap();
    assert_eq!(
        &bytes[..16],
        &[
            128, 18, 52, 86, 120, 154, 188, 222, 1, 35, 69, 103, 137, 171, 205, 239
        ]
    );
    assert!(bytes[16..].iter().all(|b| *b == 0));
}
#[test]
fn p026_lanes_and_oob_prefix() {
    let (mut s, i) = load("lane_loads_oob");
    let input = Value::V128(V128([
        16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
    ]));
    vector(
        &mut s,
        i,
        "v128.load8_lane_nonzero",
        &[Value::I32(0), input],
        [
            16, 17, 18, 19, 20, 160, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
        ],
    );
    vector(
        &mut s,
        i,
        "v128.load16_lane_nonzero",
        &[Value::I32(0), input],
        [
            16, 17, 18, 19, 20, 21, 160, 161, 24, 25, 26, 27, 28, 29, 30, 31,
        ],
    );
    vector(
        &mut s,
        i,
        "v128.load32_lane_nonzero",
        &[Value::I32(0), input],
        [
            16, 17, 18, 19, 20, 21, 22, 23, 160, 161, 162, 163, 28, 29, 30, 31,
        ],
    );
    vector(
        &mut s,
        i,
        "v128.load64_lane_nonzero",
        &[Value::I32(0), input],
        [
            16, 17, 18, 19, 20, 21, 22, 23, 160, 161, 162, 163, 164, 165, 166, 167,
        ],
    );
    for (export, args) in [
        ("full_width_oob_after_prefix", vec![]),
        ("load64_lane_oob_after_prefix", vec![input]),
    ] {
        let e = run(&mut s, i, export, &args).unwrap_err();
        assert_eq!(e.trap_code, Some(TrapCode::MemoryOutOfBounds));
        let bytes = s.read_memory(memory(&s, i), 0, 65536).unwrap();
        assert_eq!(
            &bytes[..16],
            &[
                160, 161, 162, 163, 164, 165, 166, 167, 170, 169, 170, 171, 172, 173, 174, 175
            ]
        );
        assert!(bytes[16..].iter().all(|b| *b == 0));
    }
}
#[test]
fn p026_shared_vectors() {
    let e = engine();
    let bytes = fixture("shared_vector_loads");
    let m = e.compile(&bytes).unwrap();
    let mut s = Store::new(&e);
    let mem = s
        .create_memory(MemoryType {
            limits: Limits::new(1, Some(1)),
            index_type: IndexType::I32,
            shared: true,
            page_size_log2: 16,
        })
        .unwrap();
    s.write_memory(mem, 0, &(16u8..32).collect::<Vec<_>>())
        .unwrap();
    let mut l = Linker::new(&e);
    l.define("env", "memory", External::Memory(mem)).unwrap();
    let i = l.instantiate(&mut s, &m).unwrap();
    drop(m);
    vector(
        &mut s,
        i,
        "v128.load",
        &[],
        [
            16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
        ],
    );
    vector(
        &mut s,
        i,
        "v128.load8_lane_nonzero",
        &[Value::V128(V128([
            128, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138, 139, 140, 141, 142, 143,
        ]))],
        [
            128, 129, 130, 131, 132, 133, 134, 135, 21, 137, 138, 139, 140, 141, 142, 143,
        ],
    );
    let bytes = s.read_memory(mem, 0, 65536).unwrap();
    assert_eq!(&bytes[..16], &(16u8..32).collect::<Vec<_>>());
    assert!(bytes[16..].iter().all(|b| *b == 0));
}

#[cfg(feature = "execution-reference")]
#[test]
fn p026_reference_extending_load_uses_same_checked_backend() {
    let e = Engine::with_execution_policy(
        Config {
            features: FeatureSet::all_phase3_core(),
            ..Config::default()
        },
        ExecutionPolicy::Reference,
    )
    .unwrap();
    let m = e.compile(fixture("extension_loads")).unwrap();
    let mut store = Store::new(&e);
    let instance = Linker::new(&e).instantiate(&mut store, &m).unwrap();
    drop(m);
    vector(
        &mut store,
        instance,
        "v128.load8x8_s",
        &[],
        [
            128, 255, 255, 255, 127, 0, 129, 255, 254, 255, 2, 0, 128, 255, 255, 255,
        ],
    );
    let image = store
        .read_memory(memory(&store, instance), 0, 65536)
        .unwrap();
    assert_eq!(
        &image[..16],
        &[
            128, 255, 127, 129, 254, 2, 128, 255, 52, 18, 205, 171, 0, 128, 255, 127
        ]
    );
    assert!(image[16..].iter().all(|byte| *byte == 0));
}
