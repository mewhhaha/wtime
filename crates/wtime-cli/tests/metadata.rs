use std::process::Command;
#[test]
fn t158_hints_do_not_change_execution() {
    // (module (func (export "f") (result i32) i32.const 1 if (result i32)
    // i32.const 7 else i32.const 9 end)). Hint offset 3 targets if.
    let plain = vec![
        0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 127, 3, 2, 1, 0, 7, 5, 1, 1, 102, 0, 0, 10,
        14, 1, 12, 0, 65, 1, 4, 127, 65, 7, 5, 65, 9, 11, 11,
    ];
    let name = b"metadata.code.branch_hint";
    let mut payload = vec![name.len() as u8];
    payload.extend(name);
    payload.extend([1, 0, 1, 3, 1, 1]);
    let mut hinted = plain[..26].to_vec();
    hinted.extend([0, payload.len() as u8]);
    hinted.extend(payload);
    hinted.extend(&plain[26..]);
    let directory = std::env::temp_dir().join(format!("wtime-hints-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let a = directory.join("a.wasm");
    let b = directory.join("b.wasm");
    std::fs::write(&a, plain).unwrap();
    std::fs::write(&b, hinted).unwrap();
    let run = |p: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_wtime-cli"))
            .args(["run", p.to_str().unwrap(), "f"])
            .output()
            .unwrap()
    };
    let first = run(&a);
    let second = run(&b);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(first.stdout, second.stdout);
    let output = Command::new(env!("CARGO_BIN_EXE_wtime-cli"))
        .args(["hints", b.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Likely"));
    std::fs::remove_dir_all(directory).unwrap();
}
