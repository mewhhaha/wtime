//! Run under a host address-space limit to test recoverable allocator failure.
#![forbid(unsafe_code)]
use wtime_runtime::{Config, Engine, Store};
use wtime_types::{ErrorKind, FeatureSet, IndexType, Limits, MemoryType};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config { features: FeatureSet::all_phase3_core(), ..Config::default() };
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
        assert_eq!(store.read_memory(large, page * 65536, 1)?, [(page % 251) as u8 + 1]);
    }
    assert_eq!(store.read_memory(large, written * 65536, 1)?, [0]);

    println!(
        "{{\"allocator_failed\":true,\"page_allocation_failed\":true,\"pages_written\":{written},\"recovery_passed\":true,\"executed\":1}}"
    );
    Ok(())
}
