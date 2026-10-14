use wtime_runtime::{Config, Engine, External, Linker, RunOutcome, Store, Value};
use wtime_types::{FuncType, ValType};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::default();
    config.limits.fuel = Some(10_000);
    config.limits.max_memory_bytes = 16 * 1024 * 1024;
    let engine = Engine::new(config)?;
    let mut store = Store::new(&engine);
    let add = store.create_host_func(
        FuncType::new(vec![ValType::I32, ValType::I32], vec![ValType::I32]),
        |args| {
            let [Value::I32(a), Value::I32(b)] = args else {
                unreachable!("runtime checks the host signature")
            };
            Ok(vec![Value::I32(a.wrapping_add(*b))])
        },
    )?;
    let mut linker = Linker::new(&engine);
    linker.define("host", "add", External::Func(add))?;
    // This same linker can instantiate a compiled module importing host.add.
    assert_eq!(
        store.invoke(add, &[Value::I32(20), Value::I32(22)])?,
        RunOutcome::Returned(vec![Value::I32(42)])
    );
    println!("20 + 22 = 42");
    Ok(())
}
