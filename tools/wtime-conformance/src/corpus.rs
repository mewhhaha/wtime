use std::{
    path::{Path, PathBuf},
    process::Command,
};
pub fn hash(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let data = std::fs::read(path)
        .map_err(|e| format!("missing/unreadable corpus file {}: {e}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(data)))
}
pub fn select(suite: &str, filter: &str) -> Result<Vec<PathBuf>, String> {
    let direct = Path::new(suite);
    if direct.is_file() {
        return if filter == "all"
            || direct
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .starts_with(filter)
        {
            Ok(vec![direct.into()])
        } else {
            Err("zero selected files".into())
        };
    }
    let output = Command::new("python3")
        .args(["tools/checks/fetch_corpus.py", "--select", suite, filter])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    let files: Vec<PathBuf> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(PathBuf::from)
        .collect();
    if files.is_empty() {
        return Err("zero selected corpus files".into());
    }

    Ok(files)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn t034_filtered_selection_fails() {
        assert!(select("src/main.rs", "no-such-prefix").is_err());
    }
    #[test]
    fn t011_case_01() {
        assert!(hash(Path::new("/nonexistent/wtime-corpus")).is_err());
    }
    #[test]
    fn t011_case_02() {
        let p = Path::new("Cargo.toml");
        assert_eq!(hash(p).unwrap(), hash(p).unwrap());
    }
    #[test]
    fn t011_case_03() {
        let p = std::env::temp_dir().join(format!("wtime-hash-{}", std::process::id()));
        std::fs::write(&p, "one").unwrap();
        let first = hash(&p).unwrap();
        std::fs::write(&p, "two").unwrap();
        assert_ne!(first, hash(&p).unwrap());
        std::fs::remove_file(p).unwrap();
    }
}
