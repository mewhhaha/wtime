//! Run under a host address-space limit to test recoverable allocator failure.
#![forbid(unsafe_code)]
use wtime_runtime::{Config, Engine, Linker, Store};
use wtime_types::{ErrorKind, FeatureSet, IndexType, Limits, MemoryType};

fn guest_table_module(min_elements: u32) -> Vec<u8> {
    fn leb(out: &mut Vec<u8>, mut n: u32) {
        loop {
            let b = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                out.push(b);
                break;
            }
            out.push(b | 0x80);
        }
    }
    let mut payload = vec![1, 0x70, 0x00]; // 1 funcref table, min only
    leb(&mut payload, min_elements);
    let mut wasm = b"\0asm\x01\0\0\0".to_vec();
    wasm.push(4); // Table section
    leb(&mut wasm, payload.len() as u32);
    wasm.extend_from_slice(&payload);
    wasm
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    };
    // Two objects: the failed constructor must not consume one of them.
    config.limits.max_store_objects = 2;
    config.limits.max_memory_pages = 1 << 30;
    config.limits.max_memory_bytes = 1 << 46;
    config.limits.max_store_memory_bytes = 1 << 46;
    let engine = Engine::new(config)?;
    let mut store = Store::new(&engine);

    // 1. Construction. Unshared memory is stored as 64 KiB pages allocated on
    // first write, so creation allocates only the page table. For a 1 TiB
    // memory64 that table is 256 MiB; the external RLIMIT_AS of 128 MiB makes
    // its fallible reservation fail.
    let error = store
        .create_memory(MemoryType {
            limits: Limits::new(1 << 24, Some(1 << 24)),
            index_type: IndexType::I64,
            ..MemoryType::default()
        })
        .expect_err("run this worker with a 128 MiB address-space limit");
    assert_eq!(error.kind, ErrorKind::ResourceLimit);
    assert_eq!(error.message, "memory allocation failed");

    // Recovery, while address space remains. With this memory and the large
    // one below, both permitted objects are used: the failed constructor did
    // not consume a live arena slot or leave a half-object.
    let memory = store.create_memory(MemoryType {
        limits: Limits::new(1, Some(1)),
        ..MemoryType::default()
    })?;
    store.write_memory(memory, 65535, &[42])?;
    assert_eq!(store.read_memory(memory, 65535, 1)?, [42]);

    // 2. Pages. A 512 MiB memory is created without committing its pages;
    // writing one byte to each page must fail recoverably once the address
    // space is exhausted, leaving the bytes already written intact.
    let large = store.create_memory(MemoryType {
        limits: Limits::new(8192, Some(8192)),
        ..MemoryType::default()
    })?;
    let mut written = 0u64;
    let page_error = loop {
        if written == 8192 {
            panic!("all 512 MiB were written: run this worker with a 128 MiB address-space limit");
        }
        match store.write_memory(large, written * 65536, &[(written % 251) as u8 + 1]) {
            Ok(()) => written += 1,
            Err(error) => break error,
        }
    };
    assert_eq!(page_error.kind, ErrorKind::ResourceLimit);
    assert_eq!(page_error.message, "memory allocation failed");
    assert!(written > 0, "no page could be written");
    for page in [0, written / 2, written - 1] {
        assert_eq!(
            store.read_memory(large, page * 65536, 1)?,
            [(page % 251) as u8 + 1]
        );
    }
    assert_eq!(store.read_memory(large, written * 65536, 1)?, [0]);

    // 3. Guest-defined table instantiation. A 16M-element funcref table needs
    // 512 MiB (32 bytes/Value), exceeding the 128 MiB RLIMIT_AS; instantiation
    // must fail recoverably and leave the store usable for a small module.
    let mut tcfg = Config {
        features: FeatureSet::all_phase3_core(),
        ..Config::default()
    };
    tcfg.limits.max_table_elements = 1 << 24;
    tcfg.limits.max_store_table_elements = 1 << 24;
    let tengine = Engine::new(tcfg)?;
    let mut tstore = Store::new(&tengine);
    let huge_table_mod = tengine.compile(&guest_table_module(1 << 24))?;
    let table_err = Linker::new(&tengine)
        .instantiate(&mut tstore, &huge_table_mod)
        .expect_err("16M-element table must fail under 128 MiB RLIMIT_AS");
    assert_eq!(table_err.kind, ErrorKind::ResourceLimit);
    let small_table_mod = tengine.compile(&guest_table_module(4))?;
    Linker::new(&tengine).instantiate(&mut tstore, &small_table_mod)?;

    println!(
        "{{\"allocator_failed\":true,\"page_allocation_failed\":true,\"guest_table_allocation_failed\":true,\"pages_written\":{written},\"recovery_passed\":true,\"executed\":1}}"
    );
    Ok(())
}
