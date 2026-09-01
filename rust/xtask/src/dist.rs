use crate::{Result, manifest, process, repo, version};
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

const APP: &str = "Sky-Auto-Player";
const PRIMARY: &str = "Sky-Auto-Player.exe";
const CALIBRATION: &str = "native_calibration.exe";
const UPDATER: &str = "Sky-Auto-Player-Updater.exe";

fn safe_output(root: &Path, output: &Path) -> Result<PathBuf> {
    if output.as_os_str().is_empty() {
        return Err("refusing to clean an empty output path".into());
    }
    let root = root.canonicalize()?;
    let absolute = if output.is_absolute() {
        output.to_path_buf()
    } else {
        std::env::current_dir()?.join(output)
    };
    if absolute.exists() && fs::symlink_metadata(&absolute)?.file_type().is_symlink() {
        return Err("refusing to clean a symlink/reparse output root".into());
    }
    let candidate = if absolute.exists() {
        absolute.canonicalize()?
    } else {
        let parent = absolute.parent().ok_or("output path has no parent")?;
        parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf())
            .join(absolute.file_name().ok_or("output path has no name")?)
    };
    let repository_parent = root.parent().ok_or("repository has no parent")?;
    if candidate.parent().is_none() || candidate == root || candidate == repository_parent {
        return Err("refusing to clean repository or its parent".into());
    }
    if candidate
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .is_empty()
    {
        return Err("refusing to clean an empty output path".into());
    }
    Ok(candidate)
}

fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    if !source.is_file() {
        return Err(format!("missing build input: {}", source.display()).into());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, destination)?;
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    if !source.is_dir() {
        return Err(format!("missing directory: {}", source.display()).into());
    }
    for entry in WalkDir::new(source).follow_links(false) {
        let entry = entry?;
        let relative = entry.path().strip_prefix(source)?;
        let target = destination.join(relative);
        if entry.file_type().is_symlink() {
            return Err(format!("symlink in package input: {}", entry.path().display()).into());
        }
        if entry.file_type().is_dir() {
            fs::create_dir_all(target)?;
        } else if entry.file_type().is_file() {
            copy_file(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn env_overrides(head: &str, fingerprint: &str, root: &Path) -> Result<Vec<(String, String)>> {
    Ok(vec![
        ("RUSTUP_TOOLCHAIN".into(), repo::pinned_toolchain(root)?),
        ("GITHUB_SHA".into(), head.into()),
        ("SKY_NATIVE_BUILD_COMMIT".into(), head.into()),
        ("SKY_NATIVE_SOURCE_FINGERPRINT".into(), fingerprint.into()),
        ("SKY_NATIVE_DIRTY_WORKTREE".into(), "false".into()),
    ])
}

fn cargo_build(
    root: &Path,
    package: &str,
    binary: &str,
    features: Option<&str>,
    env: &[(String, String)],
) -> Result<()> {
    let mut args = vec![
        "build".into(),
        "--manifest-path".into(),
        root.join("rust/Cargo.toml").display().to_string(),
        "-p".into(),
        package.into(),
        "--bin".into(),
        binary.into(),
        "--profile".into(),
        "dist".into(),
    ];
    if let Some(features) = features {
        args.extend(["--features".into(), features.into()]);
    }
    args.push("--locked".into());
    process::run_owned(&PathBuf::from("cargo"), &args, root, env)
}

fn observe(executable: &Path, args: &[&str], label: &str) -> Result<Value> {
    let args = args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    let output = process::capture_owned(
        executable.as_os_str(),
        &args,
        executable.parent().unwrap_or(Path::new(".")),
        &[],
    )?;
    if !output.status.success() {
        return Err(format!(
            "{label} metadata command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    if output.stdout.len() > 64 * 1024 {
        return Err(format!("{label} metadata exceeds bounded output").into());
    }
    let value: Value = serde_json::from_slice(&output.stdout)?;
    if !value.is_object() {
        return Err(format!("{label} metadata must be an object").into());
    }
    Ok(value)
}

fn validate_observed(
    root: &Path,
    version_value: &str,
    head: &str,
    fingerprint: &str,
    desktop: &Value,
    calibration: &Value,
) -> Result<String> {
    let commit = desktop
        .get("native_build_commit")
        .and_then(Value::as_str)
        .ok_or("desktop metadata missing native_build_commit")?;
    if commit != head
        || desktop.get("native_version").and_then(Value::as_str) != Some(version_value)
        || desktop.get("schema_version").and_then(Value::as_u64) != Some(1)
        || desktop
            .get("rustc_version")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        || desktop
            .get("win32_backend")
            .and_then(Value::as_bool)
            .is_none()
    {
        return Err("desktop observed metadata does not match exact build".into());
    }
    let source = calibration
        .get("source_git_sha")
        .and_then(Value::as_str)
        .ok_or("calibration metadata missing source_git_sha")?;
    let build = calibration
        .get("native_build_id")
        .and_then(Value::as_str)
        .ok_or("calibration metadata missing native_build_id")?;
    if source != head
        || build != head
        || calibration.get("dirty_worktree") != Some(&json!(false))
        || calibration
            .get("native_source_fingerprint")
            .and_then(Value::as_str)
            != Some(fingerprint)
    {
        return Err("calibration observed metadata does not match exact build".into());
    }
    if calibration
        .get("evidence_kind")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err("calibration metadata missing evidence_kind".into());
    }
    for key in [
        "calibration_schema_version",
        "measurement_protocol_version",
        "host_fingerprint_version",
    ] {
        if calibration
            .get(key)
            .and_then(Value::as_u64)
            .is_none_or(|value| value == 0)
        {
            return Err(format!("calibration metadata missing {key}").into());
        }
    }
    let _ = root;
    Ok(commit.to_owned())
}

fn default_config() -> Value {
    json!({
        "schema_version": 3, "theme": "aurora", "ui_background_mode": "transparent",
        "default_hold_frames": 1.0, "default_tempo_scale": 1.0, "game_fps": 60,
        "telemetry_enabled_by_default": false, "verbose_hud": false,
        "hotkeys": {"pause":"f8","skip":"f9","quit":"f10","refocus":"f6","panic":"ctrl+alt+backspace"},
        "safety": {"prompt_on_medium_risk": true, "prompt_on_high_risk": true},
        "songs_dir":"songs", "sky_process_names":["Sky.exe","Sky Children of the Light.exe"],
        "allow_title_fallback":false, "update":{"auto_check":true,"channel":"stable","skip_version":"","check_interval_s":86400,"last_check_ts":0,"last_error_ts":0,"last_notified_version":"","legacy_old_dir_sweep_pending":false}
    })
}

fn zip_tree(release_dir: &Path, destination: &Path) -> Result<String> {
    let file = fs::File::create(destination)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(9));
    let mut files = Vec::new();
    for entry in WalkDir::new(release_dir).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_symlink() {
            return Err("release ZIP cannot contain symlinks".into());
        }
        if entry.file_type().is_file() {
            files.push(entry.into_path());
        }
    }
    files.sort_by_key(|path| {
        path.strip_prefix(release_dir)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/")
    });
    for path in files {
        let relative = path
            .strip_prefix(release_dir)?
            .to_string_lossy()
            .replace('\\', "/");
        zip.start_file(relative, options)?;
        zip.write_all(&fs::read(path)?)?;
    }
    zip.finish()?;
    manifest::sha256(destination)
}

pub fn build(output: &Path) -> Result<()> {
    if !cfg!(windows) {
        return Err("cargo xtask dist requires Windows".into());
    }
    let root = repo::root();
    let output = safe_output(&root, output)?;
    if output.exists() {
        fs::remove_dir_all(&output)?;
    }
    fs::create_dir_all(&output)?;
    let head = repo::git_head(&root, true)?;
    let version_value = repo::project_version(&root)?;
    version::parse(&version_value)?;
    let fingerprint = repo::source_fingerprint(&root)?;
    let env = env_overrides(&head, &fingerprint, &root)?;
    process::run(
        "bun",
        &["install", "--frozen-lockfile"],
        &root.join("desktop"),
        &[],
    )?;
    process::run("bun", &["run", "build"], &root.join("desktop"), &[])?;
    cargo_build(&root, "sky_desktop_shell", "sky_desktop_shell", None, &env)?;
    cargo_build(
        &root,
        "sky_dispatch_win32",
        "native_calibration",
        None,
        &env,
    )?;
    cargo_build(
        &root,
        "sky_updater",
        "sky_updater_e2e",
        Some("e2e-local-source,e2e-fault-injection"),
        &env,
    )?;
    cargo_build(&root, "sky_updater", "sky_updater", None, &env)?;
    let target = root.join("rust/target/dist");
    let desktop = target.join("sky_desktop_shell.exe");
    let calibration = target.join(CALIBRATION);
    let updater = target.join("sky_updater.exe");
    let e2e = target.join("sky_updater_e2e.exe");
    let observed_desktop = observe(&desktop, &["--selftest-build-info"], "desktop")?;
    let observed_calibration = observe(&calibration, &["--metadata"], "calibration")?;
    let observed = validate_observed(
        &root,
        &version_value,
        &head,
        &fingerprint,
        &observed_desktop,
        &observed_calibration,
    )?;
    let release_dir = output.join(format!("{APP}-v{version_value}"));
    fs::create_dir_all(&release_dir)?;
    copy_file(&desktop, &release_dir.join(PRIMARY))?;
    copy_file(&calibration, &release_dir.join(CALIBRATION))?;
    copy_file(&updater, &release_dir.join(UPDATER))?;
    copy_tree(&root.join("songs"), &release_dir.join("songs"))?;
    fs::write(
        release_dir.join("config.json"),
        serde_json::to_vec_pretty(&default_config())?,
    )?;
    copy_file(&root.join("README.md"), &release_dir.join("README.md"))?;
    if !manifest::forbidden_paths(&release_dir)?.is_empty() {
        return Err("portable tree contains forbidden runtime files".into());
    }
    let copied_desktop = observe(
        &release_dir.join(PRIMARY),
        &["--selftest-build-info"],
        "copied desktop",
    )?;
    let copied_calibration = observe(
        &release_dir.join(CALIBRATION),
        &["--metadata"],
        "copied calibration",
    )?;
    let copied = validate_observed(
        &root,
        &version_value,
        &head,
        &fingerprint,
        &copied_desktop,
        &copied_calibration,
    )?;
    if copied != observed {
        return Err("copied native metadata changed during assembly".into());
    }
    manifest::write(&release_dir, &version_value, &head, &copied)?;
    manifest::verify_release(&release_dir)?;
    copy_file(
        &release_dir.join("MANIFEST.json"),
        &output.join("MANIFEST.json"),
    )?;
    run_packaged_selftests(&release_dir, false)?;
    run_packaged_selftests(&release_dir, true)?;
    let zip_path = output.join(format!("{APP}-v{version_value}.zip"));
    let zip_sha = zip_tree(&release_dir, &zip_path)?;
    fs::write(
        zip_path.with_extension("zip.sha256"),
        format!(
            "{zip_sha}  {}\n",
            zip_path.file_name().unwrap().to_string_lossy()
        ),
    )?;
    let manifest_sha = manifest::sha256(&release_dir.join("MANIFEST.json"))?;
    let provenance = json!({"schema_version":1,"repo_head":head,"version":version_value,"native_build_commit":copied,"native_source_fingerprint":fingerprint,"rust":{"compiler":process::capture_text("rustc", &["--version"], &root, &[])?},"bun":{"version":process::capture_text("bun", &["--version"], &root, &[])?},"runtime_python":{"required":false,"bundled":false},"artifact":{"filename":zip_path.file_name().unwrap().to_string_lossy(),"size":fs::metadata(&zip_path)?.len(),"sha256":zip_sha,"manifest_sha256":manifest_sha,"file_count":manifest::file_count(&release_dir)?}});
    fs::write(
        output.join("PROVENANCE.json"),
        serde_json::to_vec_pretty(&provenance)?,
    )?;
    let portable_file_count = manifest::file_count(&release_dir)?;
    let managed_entry_count = manifest::managed_count(&release_dir)?;
    fs::write(
        output.join("PORTABLE_ARTIFACT_SUMMARY.json"),
        serde_json::to_vec_pretty(
            &json!({"repo_head":head,"artifact_name":zip_path.file_name().unwrap().to_string_lossy(),"artifact_size":fs::metadata(&zip_path)?.len(),"artifact_sha256":zip_sha,"manifest_sha256":manifest_sha,"portable_file_count":portable_file_count,"managed_entry_count":managed_entry_count}),
        )?,
    )?;
    let updater_env = vec![
        (
            "SKY_PORTABLE_ARTIFACT_DIR".into(),
            output.display().to_string(),
        ),
        ("SKY_PORTABLE_E2E_UPDATER".into(), e2e.display().to_string()),
    ];
    process::run_owned(
        &PathBuf::from("cargo"),
        &[
            "test".into(),
            "--manifest-path".into(),
            root.join("rust/Cargo.toml").display().to_string(),
            "-p".into(),
            "sky_updater".into(),
            "--test".into(),
            "portable_exact_artifact".into(),
            "--all-features".into(),
            "--locked".into(),
            "--".into(),
            "--test-threads=1".into(),
        ],
        &root,
        &updater_env,
    )?;
    println!("[xtask] dist: PASS {}", release_dir.display());
    Ok(())
}

fn run_packaged_selftests(release_dir: &Path, python_unavailable: bool) -> Result<()> {
    let smoke_dir = std::env::temp_dir().join(format!(
        "sky-xtask-packaged-smoke-{}-{}",
        std::process::id(),
        if python_unavailable {
            "restricted"
        } else {
            "normal"
        }
    ));
    if smoke_dir.exists() {
        fs::remove_dir_all(&smoke_dir)?;
    }
    copy_tree(release_dir, &smoke_dir)?;
    let phase_log = smoke_dir.join("gui-smoke-phases.log");
    let _ = fs::remove_file(&phase_log);
    let path_value = if python_unavailable {
        let system_root = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        [
            smoke_dir.to_path_buf(),
            system_root.join("System32"),
            system_root.join("System32/Wbem"),
            system_root,
        ]
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(";")
    } else {
        std::env::var("PATH").unwrap_or_default()
    };
    let env = vec![
        ("PATH".into(), path_value),
        (
            "SKY_GUI_SMOKE_PHASE_LOG".into(),
            phase_log.display().to_string(),
        ),
    ];
    let result = (|| -> Result<()> {
        process::run_owned(
            &smoke_dir.join(PRIMARY),
            &["--selftest-desktop-shell".into()],
            &smoke_dir,
            &env,
        )?;
        process::run_owned(
            &smoke_dir.join(PRIMARY),
            &["--selftest-desktop-gui".into()],
            &smoke_dir,
            &env,
        )?;
        Ok(())
    })();
    let result_error = result.err();
    if let Some(error) = &result_error {
        eprintln!(
            "[xtask] packaged smoke phases ({}): {}",
            if python_unavailable {
                "python-unavailable"
            } else {
                "normal"
            },
            fs::read_to_string(&phase_log)
                .unwrap_or_else(|read_error| format!("<unavailable: {read_error}>"))
        );
        eprintln!("[xtask] packaged smoke failure: {error}");
    }
    let cleanup = fs::remove_dir_all(&smoke_dir);
    cleanup?;
    if let Some(error) = result_error {
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(commit: &str) -> (Value, Value) {
        (
            json!({
                "schema_version": 1,
                "native_build_commit": commit,
                "native_version": "3.5.0",
                "rustc_version": "rustc 1.98.0",
                "win32_backend": true
            }),
            json!({
                "source_git_sha": commit,
                "native_build_id": commit,
                "dirty_worktree": false,
                "native_source_fingerprint": "f".repeat(64),
                "evidence_kind": "paired_sender_timing",
                "calibration_schema_version": 4,
                "measurement_protocol_version": 4,
                "host_fingerprint_version": 2
            }),
        )
    }

    #[test]
    fn observed_metadata_requires_exact_commit_and_fingerprint() {
        let commit = "a".repeat(40);
        let (desktop, calibration) = metadata(&commit);
        assert!(
            validate_observed(
                &repo::root(),
                "3.5.0",
                &commit,
                &"f".repeat(64),
                &desktop,
                &calibration
            )
            .is_ok()
        );
        let (wrong_desktop, _) = metadata(&"b".repeat(40));
        assert!(
            validate_observed(
                &repo::root(),
                "3.5.0",
                &commit,
                &"f".repeat(64),
                &wrong_desktop,
                &calibration
            )
            .is_err()
        );
        let (_, wrong_calibration) = metadata(&"b".repeat(40));
        assert!(
            validate_observed(
                &repo::root(),
                "3.5.0",
                &commit,
                &"f".repeat(64),
                &desktop,
                &wrong_calibration
            )
            .is_err()
        );
        let malformed = json!({"native_build_commit": commit});
        assert!(
            validate_observed(
                &repo::root(),
                "3.5.0",
                &commit,
                &"f".repeat(64),
                &malformed,
                &calibration
            )
            .is_err()
        );
    }

    #[test]
    fn deterministic_zip_uses_stable_order_and_metadata() {
        let root = std::env::temp_dir().join(format!("sky-xtask-zip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let release = root.join("release");
        fs::create_dir_all(release.join("nested")).unwrap();
        fs::write(release.join("z.txt"), b"z").unwrap();
        fs::write(release.join("nested/a.txt"), b"a").unwrap();
        let first = root.join("first.zip");
        let second = root.join("second.zip");
        zip_tree(&release, &first).unwrap();
        zip_tree(&release, &second).unwrap();
        assert_eq!(fs::read(first).unwrap(), fs::read(second).unwrap());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn output_safety_rejects_repository_and_empty_targets() {
        let root = repo::root();
        assert!(safe_output(&root, &root).is_err());
        assert!(safe_output(&root, &root.join(".")).is_err());
        assert!(safe_output(&root, Path::new("")).is_err());
    }
}
