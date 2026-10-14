use wtime_runtime::{Config, Engine, External, Linker, RefValue, RunOutcome, Store, Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = Engine::new(Config::default())?;
    let module = engine.compile(include_bytes!("suspension.wasm"))?;
    let mut store = Store::new(&engine);
    let instance = Linker::new(&engine).instantiate(&mut store, &module)?;
    let External::Func(run) = instance.export(&store, "run")? else {
        unreachable!()
    };
    let RunOutcome::Suspended {
        payload,
        continuation,
        ..
    } = store.invoke(run, &[])?
    else {
        panic!("expected suspension")
    };
    assert_eq!(payload, vec![Value::I32(7)]);
    let root = store.root(continuation)?;
    let value = Value::Ref(RefValue::Gc(continuation));
    store.release_published(&[value]);
    store.collect()?; // The explicit root preserves the detached frames.
    let External::Func(resume) = instance.export(&store, "resume")? else {
        unreachable!()
    };
    assert_eq!(
        store.invoke(resume, &[value])?,
        RunOutcome::Returned(vec![Value::I32(9)])
    );
    drop(root);
    store.collect()?;
    println!("suspended with 7; resumed with 9");
    Ok(())
}
