//! Run under a host address-space limit to test recoverable allocator failure.
#![forbid(unsafe_code)]
use wtime_runtime::{Config, Engine, Store};
use wtime_types::{ErrorKind, Limits, MemoryType};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::default();
    config.limits.max_store_objects = 1;
    let engine = Engine::new(config)?;
    let mut store = Store::new(&engine);
    // 512 MiB is within every configured semantic/resource limit. The external
    // RLIMIT_AS of 128 MiB forces Vec::try_reserve_exact to return AllocError.
    let error = store
        .create_memory(MemoryType {
            limits: Limits::new(8192, Some(8192)),
            ..MemoryType::default()
        })
        .expect_err("run this worker with a 128 MiB address-space limit");
    assert_eq!(error.kind, ErrorKind::ResourceLimit);
    assert_eq!(error.message, "memory allocation failed");
    // Only one object is permitted: successful recovery also proves the failed
    // constructor did not consume a live arena slot or leave a half-object.
    let memory = store.create_memory(MemoryType {
        limits: Limits::new(1, Some(1)),
        ..MemoryType::default()
    })?;
    store.write_memory(memory, 65535, &[42])?;
    assert_eq!(store.read_memory(memory, 65535, 1)?, [42]);
    println!("{{\"allocator_failed\":true,\"recovery_passed\":true,\"executed\":1}}");
    Ok(())
}
