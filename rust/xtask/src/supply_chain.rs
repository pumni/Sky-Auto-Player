use crate::{Result, manifest, process, repo};
use serde::Serialize;
use std::env;
use std::path::Path;

const CARGO_VET_VERSION: &str = "0.10.2";
const POLICY_FILES: &[&str] = &[
    "rust/supply-chain/audits.toml",
    "rust/supply-chain/config.toml",
    "rust/supply-chain/imports.lock",
];

#[derive(Debug, Serialize)]
struct FileEvidence {
    path: String,
    size: u64,
    sha256: String,
}

#[derive(Debug, Serialize)]
struct DependencyAttestation {
    schema_version: u32,
    tool: &'static str,
    tool_version: &'static str,
    result: &'static str,
    git_head: String,
    dirty_worktree: bool,
    cargo_lock: FileEvidence,
    policy_files: Vec<FileEvidence>,
}

pub fn run(attestation_path: Option<&Path>) -> Result<()> {
    let root = repo::root();
    let version = process::capture_text("cargo", &["vet", "--version"], &root, &[])?;
    if version != format!("cargo-vet {CARGO_VET_VERSION}") {
        return Err(format!(
            "cargo-vet version mismatch: expected cargo-vet {CARGO_VET_VERSION}, got {version}"
        )
        .into());
    }
    let mut args = vec![
        "vet".to_owned(),
        "--manifest-path".to_owned(),
        "rust/Cargo.toml".to_owned(),
        "--locked".to_owned(),
    ];
    if let Some(cache_dir) = env::var_os("SKY_CARGO_VET_CACHE_DIR") {
        args.push("--cache-dir".to_owned());
        args.push(cache_dir.to_string_lossy().into_owned());
    }
    process::run_owned(Path::new("cargo"), &args, &root, &[])?;
    if let Some(attestation_path) = attestation_path {
        write_attestation(&root, attestation_path)?;
    }
    println!("[xtask] cargo-vet supply-chain: PASS");
    Ok(())
}

fn evidence(root: &Path, relative: &str) -> Result<FileEvidence> {
    let path = root.join(relative);
    if !path.is_file() {
        return Err(format!("supply-chain evidence file is missing: {relative}").into());
    }
    Ok(FileEvidence {
        path: relative.to_owned(),
        size: std::fs::metadata(&path)?.len(),
        sha256: manifest::sha256(&path)?,
    })
}

fn write_attestation(root: &Path, output: &Path) -> Result<()> {
    let head = repo::git_head(root, true)?;
    let cargo_lock = evidence(root, "rust/Cargo.lock")?;
    let policy_files = POLICY_FILES
        .iter()
        .map(|relative| evidence(root, relative))
        .collect::<Result<Vec<_>>>()?;
    let attestation = DependencyAttestation {
        schema_version: 1,
        tool: "cargo-vet",
        tool_version: CARGO_VET_VERSION,
        result: "passed",
        git_head: head,
        dirty_worktree: false,
        cargo_lock,
        policy_files,
    };
    let mut bytes = serde_json::to_vec_pretty(&attestation)?;
    bytes.push(b'\n');
    std::fs::write(output, bytes)?;
    println!(
        "[xtask] wrote signed-attestation subject {}",
        output.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_manifest_is_explicit_and_stable() {
        assert_eq!(POLICY_FILES.len(), 3);
        assert_eq!(POLICY_FILES[0], "rust/supply-chain/audits.toml");
        assert_eq!(POLICY_FILES[1], "rust/supply-chain/config.toml");
        assert_eq!(POLICY_FILES[2], "rust/supply-chain/imports.lock");
    }
}
