use std::sync::{Arc, Mutex};
use wtime_runtime::*;
pub type Prints = Arc<Mutex<Vec<Vec<Value>>>>;
pub fn install(
    store: &mut Store,
    linker: &mut Linker,
    memory64: bool,
) -> Result<(Prints, Vec<(String, External)>)> {
    let mut exports = Vec::new();
    let prints: Prints = Arc::default();
    for (name, params) in [
        ("print", vec![]),
        ("print_i32", vec![ValType::I32]),
        ("print_i64", vec![ValType::I64]),
        ("print_f32", vec![ValType::F32]),
        ("print_f64", vec![ValType::F64]),
        ("print_i32_f32", vec![ValType::I32, ValType::F32]),
        ("print_f64_f64", vec![ValType::F64, ValType::F64]),
    ] {
        let sink = prints.clone();
        let f = store.create_host_func(FuncType::new(params, vec![]), move |values| {
            sink.lock()
                .map_err(|_| Error::new(ErrorKind::Host, "print sink poisoned"))?
                .push(values.to_vec());
            Ok(vec![])
        })?;
        linker.define("spectest", name, External::Func(f))?;
        exports.push((name.into(), External::Func(f)));
    }
    for (name, ty, value) in [
        ("global_i32", ValType::I32, Value::I32(666)),
        ("global_i64", ValType::I64, Value::I64(666)),
        ("global_f32", ValType::F32, Value::F32(666.6f32.to_bits())),
        ("global_f64", ValType::F64, Value::F64(666.6f64.to_bits())),
    ] {
        let g = store.create_global(
            GlobalType {
                value: ty,
                mutable: false,
            },
            value,
        )?;
        linker.define("spectest", name, External::Global(g))?;
        exports.push((name.into(), External::Global(g)));
    }
    let memory = store.create_memory(MemoryType {
        limits: Limits::new(1, Some(2)),
        ..MemoryType::default()
    })?;
    linker.define("spectest", "memory", External::Memory(memory))?;
    exports.push(("memory".into(), External::Memory(memory)));
    let table = store.create_table(TableType {
        limits: Limits::new(10, Some(20)),
        index_type: IndexType::I32,
        element: RefType {
            nullable: true,
            heap: HeapType::Func,
        },
    })?;
    linker.define("spectest", "table", External::Table(table))?;
    exports.push(("table".into(), External::Table(table)));
    if memory64 {
        let table64 = store.create_table(TableType {
            limits: Limits::new(10, Some(20)),
            index_type: IndexType::I64,
            element: RefType::new(true, HeapType::Func),
        })?;
        linker.define("spectest", "table64", External::Table(table64))?;
        exports.push(("table64".into(), External::Table(table64)));
    }
    Ok((prints, exports))
}
