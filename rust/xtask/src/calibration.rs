#![forbid(unsafe_code)]

use crate::Result;
use crate::ci_artifacts::{
    is_reparse_point, pe_machine, replace_file_with_rollback, require_directory,
    require_regular_file, sha256_file, unique_sibling, validate_digest,
};
use serde::Serialize;
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const TARGET: &str = "x86_64-pc-windows-msvc";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct InstalledCalibrationEvidence {
    schema_version: u32,
    evidence_type: &'static str,
    verified: bool,
    path: &'static str,
    size: u64,
    sha256: String,
    architecture: &'static str,
    pe_machine: &'static str,
    help_exit_code: i32,
    help_usage_verified: bool,
}

struct StagedFile(PathBuf);

impl StagedFile {
    fn create_near(destination: &Path) -> Result<Self> {
        for _ in 0..8 {
            let path = unique_sibling(destination, "staging");
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err("could not allocate a unique file staging path".into())
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn checked_directory(path: &Path, description: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_dir() || is_reparse_point(path, &metadata) {
                return Err(format!(
                    "{description} must be a regular directory: {}",
                    path.display()
                )
                .into());
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path)?;
            let metadata = fs::symlink_metadata(path)?;
            if !metadata.is_dir() || is_reparse_point(path, &metadata) {
                return Err(format!(
                    "{description} must be a regular directory: {}",
                    path.display()
                )
                .into());
            }
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn copy_exact(source: &Path, destination: &Path) -> Result<()> {
    require_regular_file(source, "built calibration binary")?;
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(destination)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    Ok(())
}

fn cargo_target_directory(root: &Path, manifest: &Path) -> Result<PathBuf> {
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--locked",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(manifest)
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "Cargo target-directory discovery failed with exit code {}",
            output.status.code().unwrap_or(-1)
        )
        .into());
    }
    let metadata: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        format!("Cargo target-directory discovery returned invalid metadata: {error}")
    })?;
    let target = metadata
        .get("target_directory")
        .and_then(Value::as_str)
        .filter(|target| !target.trim().is_empty())
        .ok_or("Cargo metadata did not provide a target directory")?;
    Ok(PathBuf::from(target))
}

fn build_and_stage(root: &Path) -> Result<()> {
    let manifest = root.join("rust/Cargo.toml");
    require_regular_file(&manifest, "Rust workspace manifest")
        .map_err(|_| format!("Rust workspace manifest is missing: {}", manifest.display()))?;
    let status = Command::new("cargo")
        .args(["build", "--locked", "--manifest-path"])
        .arg(&manifest)
        .args([
            "--package",
            "sky_dispatch_win32",
            "--bin",
            "native_calibration",
            "--target",
            TARGET,
            "--profile",
            "dist",
        ])
        .current_dir(root)
        .status()?;
    if !status.success() {
        return Err(format!(
            "pinned x86_64 dist build failed with exit code {}",
            status.code().unwrap_or(-1)
        )
        .into());
    }
    let target_directory = cargo_target_directory(root, &manifest)?;
    let source = target_directory
        .join(TARGET)
        .join("dist")
        .join("native_calibration.exe");
    let source_metadata =
        require_regular_file(&source, "built calibration binary").map_err(|_| {
            format!(
                "expected x86_64 dist binary is missing: {}",
                source.display()
            )
        })?;
    if source_metadata.len() == 0 {
        return Err("built calibration binary must be a non-empty regular file".into());
    }
    if pe_machine(&source, "built calibration binary")? != 0x8664 {
        return Err("built calibration binary is not an x86_64 PE executable".into());
    }

    let resources = root.join("desktop/src-tauri/resources");
    checked_directory(&resources, "Tauri resource staging directory")?;
    let destination = resources.join("native_calibration.exe");
    match fs::symlink_metadata(&destination) {
        Ok(metadata) if metadata.is_file() && !is_reparse_point(&destination, &metadata) => {}
        Ok(_) => {
            return Err(
                "Tauri calibration resource must be a regular file before replacement".into(),
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "could not inspect Tauri calibration resource before replacement: {error}"
            )
            .into());
        }
    }
    let staging = StagedFile::create_near(&destination)?;
    copy_exact(&source, staging.path())?;
    if pe_machine(staging.path(), "staged calibration binary")? != 0x8664 {
        return Err("staged calibration resource is not an x86_64 PE executable".into());
    }
    let source_sha = sha256_file(&source)?;
    let staged_sha = sha256_file(staging.path())?;
    let staged_size = fs::metadata(staging.path())?.len();
    if staged_size != source_metadata.len() || staged_sha != source_sha {
        return Err("staged calibration resource differs from the validated x86_64 build".into());
    }
    replace_file_with_rollback(staging.path(), &destination)?;
    println!(
        "Native calibration package handoff: PASS (target={TARGET}; size={staged_size}; sha256={staged_sha}; path={})",
        destination.display()
    );
    Ok(())
}

fn drain_limited<R: Read + Send + 'static>(mut reader: R) -> JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        const MAX_CAPTURE: usize = 64 * 1024;
        let mut captured = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    let remaining = MAX_CAPTURE.saturating_sub(captured.len());
                    captured.extend_from_slice(&buffer[..count.min(remaining)]);
                }
            }
        }
        captured
    })
}

fn join_reader(reader: JoinHandle<Vec<u8>>) -> Result<Vec<u8>> {
    reader
        .join()
        .map_err(|_| "calibration help output reader thread panicked".into())
}

fn usage_line_matches(line: &str) -> bool {
    let trimmed = line.trim_start();
    let folded = trimmed.to_ascii_lowercase();
    let Some(rest) = folded.strip_prefix("usage:") else {
        return false;
    };
    let executable = rest.trim_start();
    executable.starts_with("native_calibration")
        && executable
            .as_bytes()
            .get("native_calibration".len())
            .is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
}

fn run_help(path: &Path) -> Result<i32> {
    let mut child = Command::new(path)
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("installed calibration --help did not start: {error}"))?;
    let stdout = drain_limited(
        child
            .stdout
            .take()
            .ok_or("calibration help stdout was not captured")?,
    );
    let stderr = drain_limited(
        child
            .stderr
            .take()
            .ok_or("calibration help stderr was not captured")?,
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_reader(stdout);
            let _ = join_reader(stderr);
            return Err("installed calibration --help exceeded the 5-second smoke bound".into());
        }
        thread::sleep(Duration::from_millis(25));
    };
    let stdout = join_reader(stdout)?;
    let stderr = join_reader(stderr)?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
    let exit_code = status.code().unwrap_or(-1);
    if exit_code != 0 {
        return Err(format!("installed calibration --help exited with {exit_code}").into());
    }
    if !text.lines().any(usage_line_matches) {
        return Err("installed calibration --help did not print its usage".into());
    }
    Ok(exit_code)
}

fn write_evidence(path: &Path, evidence: &InstalledCalibrationEvidence) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    checked_directory(parent, "installed calibration evidence directory")?;
    let staging = StagedFile::create_near(path)?;
    let bytes = serde_json::to_vec_pretty(evidence)?;
    {
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(staging.path())?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    replace_file_with_rollback(staging.path(), path)
}

fn verify_installed(
    root: &Path,
    expected_size: u64,
    expected_sha: &str,
    evidence: Option<&Path>,
) -> Result<()> {
    if expected_size == 0 {
        return Err("expected calibration binary size must be positive".into());
    }
    let expected_sha = validate_digest(expected_sha, "expected calibration SHA-256")?;
    require_directory(root, "install root is not a directory")?;
    let root =
        fs::canonicalize(root).map_err(|error| format!("install root is missing: {error}"))?;
    let binary = root.join("native_calibration.exe");
    let metadata =
        require_regular_file(&binary, "installed native_calibration.exe").map_err(|_| {
            "installed native_calibration.exe is missing or not a regular file".to_owned()
        })?;
    if metadata.len() != expected_size {
        return Err(format!(
            "installed calibration size mismatch: expected={expected_size} observed={}",
            metadata.len()
        )
        .into());
    }
    let sha256 = sha256_file(&binary)?;
    if sha256 != expected_sha {
        return Err(format!(
            "installed calibration SHA-256 mismatch: expected={expected_sha} observed={sha256}"
        )
        .into());
    }
    let machine = pe_machine(&binary, "installed calibration executable")?;
    if machine != 0x8664 {
        return Err(format!(
            "installed calibration executable is not x86_64 (PE machine 0x{machine:04x})"
        )
        .into());
    }
    let help_exit_code = run_help(&binary)?;
    let evidence_record = InstalledCalibrationEvidence {
        schema_version: 1,
        evidence_type: "installed-native-calibration",
        verified: true,
        path: "native_calibration.exe",
        size: metadata.len(),
        sha256: sha256.clone(),
        architecture: "x86_64",
        pe_machine: "0x8664",
        help_exit_code,
        help_usage_verified: true,
    };
    if let Some(path) = evidence {
        write_evidence(path, &evidence_record)?;
    }
    println!(
        "Installed calibration verification: PASS (size={}; sha256={sha256}; architecture=x86_64; help=PASS)",
        metadata.len()
    );
    Ok(())
}

fn parse_options(
    args: &[String],
    allowed: &[&str],
) -> Result<std::collections::BTreeMap<String, String>> {
    let mut options = std::collections::BTreeMap::new();
    let mut index = 0;
    while index < args.len() {
        let name = args[index]
            .strip_prefix("--")
            .ok_or_else(|| format!("calibration option must start with --: {}", args[index]))?;
        if !allowed.contains(&name) {
            return Err(format!("unknown calibration option: --{name}").into());
        }
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("missing value for --{name}"))?
            .clone();
        if options.insert(name.to_owned(), value).is_some() {
            return Err(format!("calibration option was provided more than once: --{name}").into());
        }
        index += 1;
    }
    Ok(options)
}

fn required<'a>(
    options: &'a std::collections::BTreeMap<String, String>,
    name: &str,
) -> Result<&'a str> {
    options
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| format!("calibration requires --{name}").into())
}

pub(crate) fn run(root: &Path, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("build") => {
            if args.len() != 1 {
                return Err("calibration build does not accept options".into());
            }
            build_and_stage(root)
        }
        Some("verify-installed") => {
            let options = parse_options(
                &args[1..],
                &[
                    "install-root",
                    "expected-size",
                    "expected-sha256",
                    "evidence",
                ],
            )?;
            let expected_size = required(&options, "expected-size")?
                .parse::<u64>()
                .map_err(|_| "expected calibration binary size is invalid")?;
            let evidence = options.get("evidence").map(Path::new);
            verify_installed(
                Path::new(required(&options, "install-root")?),
                expected_size,
                required(&options, "expected-sha256")?,
                evidence,
            )
        }
        Some(operation) => Err(format!("unknown calibration operation: {operation}").into()),
        None => Err("calibration requires build or verify-installed".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ci_artifacts;

    fn temp_dir(name: &str) -> PathBuf {
        let path = ci_artifacts::unique_sibling(&std::env::temp_dir().join(name), "test");
        fs::create_dir(&path).unwrap();
        path
    }

    fn pe(machine: u16) -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x86];
        bytes[0..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80_i32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        bytes
    }

    #[test]
    fn installed_verification_fails_closed_for_missing_binary_invalid_size_digest_and_pe() {
        let root = temp_dir("sky-xtask-installed-calibration-negative");
        let digest = "0".repeat(64);
        assert!(
            verify_installed(&root, 1, &digest, None)
                .unwrap_err()
                .to_string()
                .contains("missing")
        );
        let binary = root.join("native_calibration.exe");
        fs::write(&binary, pe(0x8664)).unwrap();
        assert!(
            verify_installed(&root, 0, &digest, None)
                .unwrap_err()
                .to_string()
                .contains("size must be positive")
        );
        assert!(
            verify_installed(&root, 1, "bad", None)
                .unwrap_err()
                .to_string()
                .contains("SHA-256")
        );
        assert!(
            verify_installed(&root, 1, &digest, None)
                .unwrap_err()
                .to_string()
                .contains("size mismatch")
        );
        assert!(
            verify_installed(&root, 0x86, &digest, None)
                .unwrap_err()
                .to_string()
                .contains("SHA-256 mismatch")
        );
        fs::write(&binary, pe(0x014c)).unwrap();
        let sha = sha256_file(&binary).unwrap();
        assert!(
            verify_installed(&root, 0x86, &sha, None)
                .unwrap_err()
                .to_string()
                .contains("not x86_64")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn installed_calibration_usage_probe_requires_native_calibration_usage_line() {
        assert!(usage_line_matches("Usage: native_calibration --help"));
        assert!(usage_line_matches(
            "    usage: native_calibration.exe [OPTIONS]"
        ));
        assert!(!usage_line_matches("Usage: another_program --help"));
        assert!(!usage_line_matches("native_calibration command"));
    }

    #[test]
    fn successful_evidence_keeps_the_installed_calibration_schema() {
        let evidence = InstalledCalibrationEvidence {
            schema_version: 1,
            evidence_type: "installed-native-calibration",
            verified: true,
            path: "native_calibration.exe",
            size: 134,
            sha256: "a".repeat(64),
            architecture: "x86_64",
            pe_machine: "0x8664",
            help_exit_code: 0,
            help_usage_verified: true,
        };
        let value: Value = serde_json::to_value(evidence).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 10);
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["evidence_type"], "installed-native-calibration");
        assert_eq!(value["pe_machine"], "0x8664");
        assert_eq!(value["help_usage_verified"], true);
    }

    #[test]
    fn installed_calibration_cli_rejects_unknown_and_duplicate_options() {
        let unknown = vec![
            "--install-root".to_owned(),
            "root".to_owned(),
            "--skip-hash".to_owned(),
            "true".to_owned(),
        ];
        assert!(
            parse_options(&unknown, &["install-root"])
                .unwrap_err()
                .to_string()
                .contains("unknown calibration option")
        );
        let duplicate = vec![
            "--install-root".to_owned(),
            "first".to_owned(),
            "--install-root".to_owned(),
            "second".to_owned(),
        ];
        assert!(
            parse_options(&duplicate, &["install-root"])
                .unwrap_err()
                .to_string()
                .contains("more than once")
        );
    }
}
