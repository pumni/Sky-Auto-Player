use crate::{Result, process};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use toml::Value;
use walkdir::WalkDir;

pub const RUST_DISPATCH_SCHEMA_VERSION: u32 = 4;
pub const NATIVE_PATHS: &[&str] = &[
    "rust/Cargo.lock",
    "rust/crates/sky_app_core",
    "rust/crates/sky_dispatch_core",
    "rust/crates/sky_dispatch_win32",
    "rust/crates/sky_native_adapters",
    "rust/crates/sky_player",
    "rust/crates/sky_updater",
    "desktop/src-tauri",
];

pub fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask must live below repository rust/")
        .to_path_buf()
}

pub fn git_head(root: &Path, require_clean: bool) -> Result<String> {
    let head = process::capture_text("git", &["rev-parse", "--verify", "HEAD"], root, &[])?;
    if !head.chars().all(|c| c.is_ascii_hexdigit()) || head.len() != 40 {
        return Err(format!("git HEAD is not a full commit SHA: {head:?}").into());
    }
    let status = process::capture_text("git", &["status", "--porcelain"], root, &[])?;
    if require_clean && !status.is_empty() {
        return Err(format!("release command requires a clean worktree: {status}").into());
    }
    Ok(head)
}

pub fn project_version(root: &Path) -> Result<String> {
    let path = root.join("desktop/src-tauri/Cargo.toml");
    let value: Value = toml::from_str(&std::fs::read_to_string(&path)?)?;
    value
        .get("package")
        .and_then(Value::as_table)
        .and_then(|table| table.get("version"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("missing package.version in {}", path.display()).into())
}

pub fn pinned_toolchain(root: &Path) -> Result<String> {
    let path = root.join("rust/rust-toolchain.toml");
    let value: Value = toml::from_str(&std::fs::read_to_string(&path)?)?;
    let channel = value
        .get("toolchain")
        .and_then(Value::as_table)
        .and_then(|table| table.get("channel"))
        .and_then(Value::as_str)
        .ok_or_else(|| "rust-toolchain.toml has no exact channel".to_string())?;
    if channel.split('.').count() != 3
        || channel.split('.').any(|part| part.parse::<u32>().is_err())
    {
        return Err(format!("Rust toolchain is not an exact x.y.z version: {channel}").into());
    }
    Ok(channel.to_owned())
}

pub fn source_fingerprint(root: &Path) -> Result<String> {
    source_fingerprint_for_paths(root, NATIVE_PATHS)
}

fn source_fingerprint_for_paths(root: &Path, input_paths: &[&str]) -> Result<String> {
    let mut digest = Sha256::new();
    digest.update(format!("schema:{RUST_DISPATCH_SCHEMA_VERSION}\n").as_bytes());
    let mut files = Vec::new();
    for relative in input_paths {
        let path = root.join(relative);
        if path.is_file() {
            files.push(path);
        } else if path.is_dir() {
            for entry in WalkDir::new(path).follow_links(false) {
                let entry = entry?;
                if entry.file_type().is_file() {
                    files.push(entry.into_path());
                }
            }
        } else {
            return Err(format!("native fingerprint input is missing: {relative}").into());
        }
    }
    files.sort_by_key(|path| path.strip_prefix(root).unwrap().to_owned());
    for path in files {
        let relative = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        if relative == "desktop/src-tauri/gen" || relative.starts_with("desktop/src-tauri/gen/") {
            continue;
        }
        digest.update(relative.as_bytes());
        digest.update([0]);
        digest.update(std::fs::read(path)?);
        digest.update([0]);
    }
    Ok(hex_digest(digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_repository_root() {
        assert!(root().join("rust/Cargo.toml").is_file());
    }

    #[test]
    fn generated_bindings_are_excluded_but_native_source_changes_fingerprint() {
        let root =
            std::env::temp_dir().join(format!("sky-xtask-fingerprint-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("rust")).unwrap();
        std::fs::create_dir_all(root.join("desktop/src-tauri/gen")).unwrap();
        std::fs::write(root.join("rust/native.rs"), b"one").unwrap();
        std::fs::write(root.join("desktop/src-tauri/gen/generated.ts"), b"one").unwrap();
        let paths = ["rust", "desktop/src-tauri"];
        let first = source_fingerprint_for_paths(&root, &paths).unwrap();
        std::fs::write(root.join("desktop/src-tauri/gen/generated.ts"), b"two").unwrap();
        assert_eq!(first, source_fingerprint_for_paths(&root, &paths).unwrap());
        std::fs::write(root.join("rust/native.rs"), b"two").unwrap();
        assert_ne!(first, source_fingerprint_for_paths(&root, &paths).unwrap());
        let _ = std::fs::remove_dir_all(root);
    }
}
