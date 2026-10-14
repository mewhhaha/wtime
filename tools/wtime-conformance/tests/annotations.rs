use std::{path::PathBuf, process::Command};
use wtime_runtime::{Config, Engine, External, Linker, RunOutcome, Store, Value};
fn encode(id: &str, source: &str) -> (std::process::Output, PathBuf) {
    let dir = std::env::temp_dir().join(format!("wtime-annotations-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wat = dir.join("input.wat");
    let wasm = dir.join("output.wasm");
    std::fs::write(&wat, source).unwrap();
    (
        Command::new(env!("CARGO_BIN_EXE_wtime-conformance"))
            .args([
                "encode",
                wat.to_str().unwrap(),
                "-o",
                wasm.to_str().unwrap(),
            ])
            .output()
            .unwrap(),
        wasm,
    )
}
#[test]
fn t159_case_01() {
    let (out, path) = encode(
        "known",
        r#"(module (@name "demo") (@custom "hello" "\01\02") (func (export "f") (result i32) i32.const 7))"#,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.windows(5).any(|w| w == b"hello"));
    let e = Engine::new(Config::default()).unwrap();
    let m = e.compile(&bytes).unwrap();
    let mut s = Store::new(&e);
    let i = Linker::new(&e).instantiate(&mut s, &m).unwrap();
    let External::Func(f) = i.export(&s, "f").unwrap() else {
        panic!()
    };
    assert_eq!(
        s.invoke(f, &[]).unwrap(),
        RunOutcome::Returned(vec![Value::I32(7)])
    );
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
#[test]
fn t159_case_02() {
    let (out, path) = encode(
        "unknown",
        r#"(module (@future.nested (balanced "payload")) (func))"#,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let e = Engine::new(Config::default()).unwrap();
    e.compile(&std::fs::read(&path).unwrap()).unwrap();
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
#[test]
fn t159_case_03() {
    let (out, path) = encode(
        "broken",
        r#"(module (@future.nested (balanced "payload") (func))"#,
    );
    assert!(!out.status.success());
    assert!(!path.exists());
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
