use std::path::{Path, PathBuf};
fn collect(root: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(root).expect("source directory") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            collect(&path, files)
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path)
        }
    }
}
fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let mut files = Vec::new();
    for name in [
        "wtime-types",
        "wtime-decode",
        "wtime-validate",
        "wtime-runtime",
    ] {
        let dir = root.join("crates").join(name).join("src");
        println!("cargo:rerun-if-changed={}", dir.display());
        collect(&dir, &mut files);
    }
    files.sort();
    let mut entries = Vec::new();
    for path in files {
        println!("cargo:rerun-if-changed={}", path.display());
        use sha2::{Digest, Sha256};
        let data = std::fs::read(&path).expect("read source for hash");
        let hash = format!("{:x}", Sha256::digest(&data));
        let name = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        entries.push(format!("\"{name}\":\"{hash}\""));
    }
    println!(
        "cargo:rustc-env=WTIME_IMPLEMENTATION_SOURCES={{{}}}",
        entries.join(",")
    );
}
