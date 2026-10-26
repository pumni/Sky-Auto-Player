#![forbid(unsafe_code)]
// Non-Windows production exposes only the fail-closed command stub; fixture
// internals stay available to Windows builds and cross-platform unit tests.
#![cfg_attr(all(not(windows), not(test)), allow(dead_code))]

use crate::Result;
use std::collections::BTreeMap;
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;

const OWNER_SENTINEL: &str = ".sky-auto-player-wave-c-owner";
const OWNER_SENTINEL_CONTENT: &[u8] = b"Sky Auto Player disposable installer harness v1\n";
#[cfg(windows)]
const PRIVATE_ENVIRONMENT_NAMES: &[&str] = &[
    "TAURI_SIGNING_PRIVATE_KEY",
    "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
    "TAURI_SIGNING_PRIVATE_KEY_PATH",
    "V4_UPDATER_PRIVATE_KEY_PATH",
    "V4_UPDATER_PASSWORD",
    "UPDATER_PRIVATE_KEY",
    "UPDATER_PASSWORD",
];

#[cfg(windows)]
fn private_environment_names() -> Vec<String> {
    PRIVATE_ENVIRONMENT_NAMES
        .iter()
        .map(|name| (*name).to_owned())
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RegistryMode {
    FreshInstall,
    PreserveExistingState,
}

impl RegistryMode {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "fresh-install" => Ok(Self::FreshInstall),
            "preserve-existing-state" => Ok(Self::PreserveExistingState),
            _ => Err("registry mode must be fresh-install or preserve-existing-state".into()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum RunnerAuthorization {
    #[default]
    None,
    HostedDisposable,
    ProtectedRelease,
}

fn parse_options(args: &[String]) -> Result<(BTreeMap<String, String>, RunnerAuthorization)> {
    let mut values = BTreeMap::new();
    let mut authorization = RunnerAuthorization::None;
    let mut index = 0;
    while index < args.len() {
        let next_authorization = match args[index].as_str() {
            "--allow-hosted-disposable-runner" => Some(RunnerAuthorization::HostedDisposable),
            "--allow-protected-release-runner" => Some(RunnerAuthorization::ProtectedRelease),
            _ => None,
        };
        if let Some(next_authorization) = next_authorization {
            if authorization != RunnerAuthorization::None {
                return Err("runner authorization option was repeated or conflicting".into());
            }
            authorization = next_authorization;
            index += 1;
            continue;
        }
        let name = args[index]
            .strip_prefix("--")
            .ok_or_else(|| format!("NSIS option must start with --: {}", args[index]))?;
        if ![
            "installer",
            "candidate-root",
            "source-sha",
            "fixture-root",
            "registry-mode",
            "evidence-root",
            "authenticode-mode",
        ]
        .contains(&name)
        {
            return Err(format!("unknown NSIS option: --{name}").into());
        }
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("missing value for --{name}"))?
            .clone();
        if values.insert(name.to_owned(), value).is_some() {
            return Err(format!("NSIS option was provided more than once: --{name}").into());
        }
        index += 1;
    }
    Ok((values, authorization))
}

#[cfg(windows)]
fn required<'a>(options: &'a BTreeMap<String, String>, name: &str) -> Result<&'a str> {
    options
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| format!("NSIS smoke requires --{name}").into())
}

#[cfg(not(windows))]
pub(crate) fn run(_repository_root: &Path, _args: &[String]) -> Result<()> {
    Err("NSIS install qualification is Windows-only and requires a disposable hosted runner".into())
}

#[cfg(windows)]
pub(crate) fn run(repository_root: &Path, args: &[String]) -> Result<()> {
    if args.first().map(String::as_str) != Some("smoke") {
        return Err("usage: cargo xtask ci nsis smoke --installer <path> --candidate-root <dir> --source-sha <sha> --fixture-root <new-path> --registry-mode <fresh-install|preserve-existing-state> [--evidence-root <dir>] (--allow-hosted-disposable-runner|--allow-protected-release-runner)".into());
    }
    let (options, authorization) = parse_options(&args[1..])?;
    require_runner_authorization(authorization)?;
    let source_sha = required(&options, "source-sha")?;
    let version = crate::repo::project_version(repository_root)?;
    let candidate = crate::ci_candidate::read_candidate(
        Path::new(required(&options, "candidate-root")?),
        source_sha,
        &version,
    )?;
    let installer = std::fs::canonicalize(required(&options, "installer")?)?;
    let contract_installer = std::fs::canonicalize(
        Path::new(required(&options, "candidate-root")?).join(&candidate.manifest.installer),
    )?;
    if installer != contract_installer {
        return Err(
            "NSIS installer path is not the exact installer in the validated candidate contract"
                .into(),
        );
    }
    let runner_temp = std::env::var_os("RUNNER_TEMP")
        .map(PathBuf::from)
        .ok_or("RUNNER_TEMP is required for the disposable install root")?;
    let workspace = std::env::var_os("GITHUB_WORKSPACE")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository_root.to_owned());
    let root = PathBuf::from(required(&options, "fixture-root")?);
    let evidence_root = options
        .get("evidence-root")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository_root.join("rust/target/dist"));
    let authenticode_mode = options
        .get("authenticode-mode")
        .map(String::as_str)
        .unwrap_or("unsigned-zero-budget");
    validate_authenticode_mode(authenticode_mode)?;
    std::fs::create_dir_all(&evidence_root)?;
    let mode = RegistryMode::parse(required(&options, "registry-mode")?)?;
    let mut scope = WindowsSmokeScope::enter(&root, &runner_temp, &workspace, mode)?;
    let operation = qualify_package(
        repository_root,
        &installer,
        &candidate,
        &evidence_root,
        authenticode_mode,
        &mut scope,
    );
    let cleanup = scope.cleanup();
    let result = combine_operation_and_cleanup(operation, cleanup);
    if result.is_ok() {
        println!(
            "[xtask] current-user NSIS qualification: PASS (version={}; installer_sha256={})",
            candidate.manifest.version, candidate.installer_sha256
        );
    }
    result
}

pub(crate) fn combine_operation_and_cleanup<T>(
    operation: Result<T>,
    cleanup: Result<()>,
) -> Result<T> {
    match (operation, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(original), Ok(())) => Err(original),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(original), Err(cleanup)) => Err(format!(
            "NSIS smoke operation failed: {original}; cleanup also failed: {cleanup}"
        )
        .into()),
    }
}

fn validate_authenticode_mode(mode: &str) -> Result<()> {
    if ["unsigned-zero-budget", "test", "production"].contains(&mode) {
        Ok(())
    } else {
        Err("NSIS smoke Authenticode mode must be unsigned-zero-budget, test, or production".into())
    }
}

#[cfg(windows)]
pub(crate) fn require_disposable_runner() -> Result<()> {
    if !is_hosted_disposable_runner(
        std::env::var("GITHUB_ACTIONS").ok().as_deref(),
        std::env::var("RUNNER_ENVIRONMENT").ok().as_deref(),
    ) {
        return Err(
            "actual NSIS execution is restricted to a GitHub-hosted disposable Windows runner"
                .into(),
        );
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn require_disposable_runner() -> Result<()> {
    Err("actual NSIS execution is Windows-only".into())
}

pub(crate) fn require_runner_authorization(authorization: RunnerAuthorization) -> Result<()> {
    match authorization {
        RunnerAuthorization::HostedDisposable => require_disposable_runner(),
        RunnerAuthorization::ProtectedRelease => require_protected_release_runner(),
        RunnerAuthorization::None => Err(
            "real installer execution requires an explicit approved runner authorization".into(),
        ),
    }
}

fn is_protected_release_context(
    actions: Option<&str>,
    workflow: Option<&str>,
    job: Option<&str>,
    event: Option<&str>,
    reference: Option<&str>,
    repository: Option<&str>,
) -> bool {
    actions == Some("true")
        && event == Some("workflow_dispatch")
        && reference == Some("refs/heads/main")
        && repository == Some("pumni/Sky-Auto-Player")
        && matches!(
            (workflow, job),
            (Some("V4 Release Pipeline"), Some("release"))
                | (
                    Some("V4 Production Pre-Publication Qualification"),
                    Some("qualification")
                )
        )
}

#[cfg(windows)]
pub(crate) fn require_protected_release_runner() -> Result<()> {
    if !is_protected_release_context(
        std::env::var("GITHUB_ACTIONS").ok().as_deref(),
        std::env::var("GITHUB_WORKFLOW").ok().as_deref(),
        std::env::var("GITHUB_JOB").ok().as_deref(),
        std::env::var("GITHUB_EVENT_NAME").ok().as_deref(),
        std::env::var("GITHUB_REF").ok().as_deref(),
        std::env::var("GITHUB_REPOSITORY").ok().as_deref(),
    ) {
        return Err("protected release install scope is restricted to the exact canonical v4 workflow job on main".into());
    }
    let runner_temp =
        std::fs::canonicalize(std::env::var_os("RUNNER_TEMP").ok_or("RUNNER_TEMP is required")?)?;
    let workspace = std::fs::canonicalize(
        std::env::var_os("GITHUB_WORKSPACE").ok_or("GITHUB_WORKSPACE is required")?,
    )?;
    let state_root = std::fs::canonicalize(
        std::env::var_os("V4_RELEASE_STATE_ROOT").ok_or("V4_RELEASE_STATE_ROOT is required")?,
    )?;
    let run_id = std::env::var("GITHUB_RUN_ID").map_err(|_| "GITHUB_RUN_ID is required")?;
    if run_id.is_empty() || !run_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("GITHUB_RUN_ID must be a numeric workflow run identity".into());
    }
    let expected_state_root = runner_temp.join(format!("sky-v4-release-{run_id}"));
    if state_root != std::fs::canonicalize(expected_state_root)?
        || state_root.parent() != Some(runner_temp.as_path())
        || state_root.starts_with(&workspace)
    {
        return Err("protected release state root must be the exact run-scoped direct child of RUNNER_TEMP and outside the workspace".into());
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn require_protected_release_runner() -> Result<()> {
    Err("protected release NSIS execution is Windows-only".into())
}

fn is_hosted_disposable_runner(actions: Option<&str>, environment: Option<&str>) -> bool {
    actions == Some("true") && environment == Some("github-hosted")
}

#[cfg(windows)]
fn qualify_package(
    repository_root: &Path,
    installer: &Path,
    candidate: &crate::ci_candidate::CandidateContract,
    evidence_root: &Path,
    authenticode_mode: &str,
    scope: &mut WindowsSmokeScope,
) -> Result<()> {
    let install_root = scope.root.join("install");
    let app_data = scope.root.join("app-data");
    let app = install_root.join("sky_desktop_shell.exe");
    let uninstaller = install_root.join("uninstall.exe");
    std::fs::create_dir_all(&app_data)?;
    run_installer(
        installer,
        &install_root,
        &scope.child_environment(&app_data),
        "installer",
    )?;
    require_regular_file(&app, "installed desktop executable")?;
    require_regular_file(&uninstaller, "installed NSIS uninstaller")?;

    verify_calibration_artifact(
        repository_root,
        &install_root,
        candidate,
        Some(&evidence_root.join("INSTALLED_CALIBRATION_EVIDENCE.json")),
    )?;
    crate::builtin_catalog::run(
        repository_root,
        "verify-installed",
        &[(
            "root".to_owned(),
            install_root
                .join("builtin-songs")
                .to_string_lossy()
                .into_owned(),
        )],
    )?;

    let installed_pe = installed_project_pe_files(&install_root)?;
    if installed_pe.is_empty() {
        return Err("installed Tauri tree contains no project PE files".into());
    }
    let mut auth_args = vec![
        "verify".to_owned(),
        "--mode".to_owned(),
        authenticode_mode.to_owned(),
    ];
    for artifact in installed_pe {
        auth_args.push("--artifact".to_owned());
        auth_args.push(artifact.to_string_lossy().into_owned());
    }
    auth_args.push("--evidence".to_owned());
    auth_args.push(
        evidence_root
            .join("INSTALLED_AUTHENTICODE_EVIDENCE.json")
            .to_string_lossy()
            .into_owned(),
    );
    crate::authenticode::run(repository_root, &auth_args)?;

    let mut catalog_selftest_env = scope.child_environment(&app_data);
    catalog_selftest_env.push((
        "SKY_BUILTIN_CATALOG_FRESH_SELFTEST".to_owned(),
        "1".to_owned(),
    ));
    run_owned_app(
        scope,
        &app,
        &["--selftest-desktop-shell".to_owned()],
        &catalog_selftest_env,
        std::time::Duration::from_secs(120),
    )?;
    assert_user_songs_empty(&app_data.join("songs"))?;

    let shell_env = scope.child_environment(&app_data);
    launch_smoke(scope, &app, &shell_env, std::time::Duration::from_secs(5))?;
    run_owned_app(
        scope,
        &app,
        &["--selftest-update-active-playback".to_owned()],
        &shell_env,
        std::time::Duration::from_secs(120),
    )?;
    let mut uninstaller = run_uninstaller(&uninstaller, &install_root, &shell_env)?;
    finish_uninstall(&install_root, &mut uninstaller)?;
    assert_uninstall_registration_absent()?;
    scope.assert_shortcuts_unchanged()?;
    Ok(())
}

#[cfg(windows)]
fn verify_calibration_artifact(
    repository_root: &Path,
    install_root: &Path,
    candidate: &crate::ci_candidate::CandidateContract,
    evidence: Option<&Path>,
) -> Result<()> {
    let mut args = vec![
        "verify-installed".to_owned(),
        "--install-root".to_owned(),
        install_root.to_string_lossy().into_owned(),
        "--expected-size".to_owned(),
        candidate.manifest.calibration_binary_size.to_string(),
        "--expected-sha256".to_owned(),
        candidate.manifest.calibration_binary_sha256.clone(),
    ];
    if let Some(evidence) = evidence {
        args.push("--evidence".to_owned());
        args.push(evidence.to_string_lossy().into_owned());
    }
    crate::calibration::run(repository_root, &args)
}

#[cfg(windows)]
fn assert_user_songs_empty(root: &Path) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(root)?;
    if !metadata.file_type().is_dir() {
        return Err("fresh-install catalog self-test replaced the user songs directory".into());
    }
    let mut pending = vec![root.to_owned()];
    let mut entries_seen = 0usize;
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(
                    "fresh-install catalog self-test created a reparse point in user songs".into(),
                );
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                entries_seen += 1;
                if entries_seen > 1024 {
                    return Err("user songs self-test output exceeds the bounded file count".into());
                }
            }
        }
    }
    if entries_seen != 0 {
        return Err("fresh-install built-in catalog self-test populated user songs".into());
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn assert_uninstall_registration_absent() -> Result<()> {
    use sky_windows_security::registry::{KeySpec, assert_roots_absent};
    let scopes = [
        KeySpec::new(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Sky Auto Player",
            "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        )?,
        KeySpec::new(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\io.github.pumni.skyautoplayer",
            "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        )?,
    ];
    assert_roots_absent(&scopes)?;
    Ok(())
}

#[cfg(windows)]
fn require_regular_file(path: &Path, label: &str) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        format!(
            "{label} is missing or unreadable: {}: {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_file() {
        return Err(format!("{label} is not a regular file: {}", path.display()).into());
    }
    Ok(())
}

#[cfg(windows)]
fn installed_project_pe_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "installed Tauri tree contains a reparse point: {}",
                    path.display()
                )
                .into());
            }
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            let extension = path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or_default();
            if !metadata.is_file()
                || path.file_name().and_then(|name| name.to_str()) == Some("uninstall.exe")
                || !["exe", "dll"]
                    .iter()
                    .any(|allowed| extension.eq_ignore_ascii_case(allowed))
            {
                continue;
            }
            files.push(path);
            if files.len() > 256 {
                return Err("installed Tauri tree exceeds the Authenticode artifact bound".into());
            }
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(windows)]
pub(crate) fn run_installer(
    installer: &Path,
    install_root: &Path,
    env: &[(String, String)],
    label: &str,
) -> Result<()> {
    let args = vec![
        "/S".to_owned(),
        "/NS".to_owned(),
        format!("/D={}", install_root.display()),
    ];
    let output = crate::process::capture_owned_timeout_filtered(
        installer.as_os_str(),
        &args,
        installer
            .parent()
            .ok_or("installer has no parent directory")?,
        env,
        &private_environment_names(),
        std::time::Duration::from_secs(300),
    )?;
    if !output.status.success() {
        return Err(format!(
            "{label} exited with {}\nstdout: {}\nstderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn run_uninstaller(
    uninstaller: &Path,
    install_root: &Path,
    env: &[(String, String)],
) -> Result<crate::process::OwnedChild> {
    let cwd = uninstaller_working_directory(uninstaller, install_root)?;
    let args = vec!["/S".to_owned()];
    let mut child = crate::process::spawn_owned_silent_filtered(
        uninstaller.as_os_str(),
        &args,
        &cwd,
        env,
        &private_environment_names(),
    )?;
    let status = child.wait_timeout(std::time::Duration::from_secs(60))?;
    if !status.success() {
        return Err(format!("NSIS uninstaller exited with {status}").into());
    }
    Ok(child)
}

#[cfg(windows)]
fn uninstaller_working_directory(uninstaller: &Path, install_root: &Path) -> Result<PathBuf> {
    let install_root = std::fs::canonicalize(install_root)?;
    let uninstaller = std::fs::canonicalize(uninstaller)?;
    if uninstaller.parent() != Some(install_root.as_path()) {
        return Err("NSIS uninstaller must be a direct regular child of the install root".into());
    }
    let cwd = install_root
        .parent()
        .ok_or("NSIS install root has no parent working directory")?;
    if cwd.starts_with(&install_root) {
        return Err("NSIS uninstaller working directory is inside the install root".into());
    }
    Ok(cwd.to_owned())
}

#[cfg(windows)]
pub(crate) fn finish_uninstall(
    install_root: &Path,
    uninstaller: &mut crate::process::OwnedChild,
) -> Result<()> {
    if let Err(error) = uninstaller.wait_tree_timeout(std::time::Duration::from_secs(30)) {
        let inventory = bounded_install_root_inventory(install_root);
        let registry = uninstall_registration_diagnostic();
        return Err(format!(
            "{error}; install-root inventory: {inventory}; uninstall registration: {registry}"
        )
        .into());
    }
    if let Err(error) = wait_path_absent(install_root, std::time::Duration::from_secs(30)) {
        let status = uninstaller
            .try_wait()
            .map(|status| status.map_or_else(|| "still running".to_owned(), |s| s.to_string()))
            .unwrap_or_else(|error| format!("unavailable ({error})"));
        let process_count = uninstaller
            .active_processes()
            .map(|count| count.to_string())
            .unwrap_or_else(|error| format!("unavailable ({error})"));
        let registry = uninstall_registration_diagnostic();
        return Err(format!(
            "{error}; uninstaller exit status: {status}; owned process count: {process_count}; uninstall registration: {registry}"
        )
        .into());
    }
    Ok(())
}

#[cfg(windows)]
fn uninstall_registration_diagnostic() -> String {
    match assert_uninstall_registration_absent() {
        Ok(()) => "absent".to_owned(),
        Err(error) => format!("present or unreadable ({error})"),
    }
}

#[cfg(windows)]
fn bounded_install_root_inventory(path: &Path) -> String {
    const ENTRY_LIMIT: usize = 16;
    match std::fs::read_dir(path) {
        Ok(entries) => {
            let mut names = Vec::new();
            let mut truncated = false;
            for (index, entry) in entries.take(ENTRY_LIMIT + 1).enumerate() {
                if index == ENTRY_LIMIT {
                    truncated = true;
                    break;
                }
                match entry {
                    Ok(entry) => {
                        let name = format!("{:?}", entry.file_name().to_string_lossy());
                        let kind = match std::fs::symlink_metadata(entry.path()) {
                            Ok(metadata) if metadata.file_type().is_symlink() => "reparse-point",
                            Ok(metadata) if metadata.is_dir() => "directory",
                            Ok(metadata) if metadata.is_file() => "file",
                            Ok(_) => "other",
                            Err(error) => {
                                names.push(format!("{name} (metadata unavailable: {error})"));
                                continue;
                            }
                        };
                        names.push(format!("{name} ({kind})"));
                    }
                    Err(error) => names.push(format!("entry unavailable: {error}")),
                }
            }
            if truncated {
                names.push("additional entries omitted at the diagnostic bound".to_owned());
            }
            format!("{} direct entries [{}]", names.len(), names.join(", "))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "root absent".to_owned(),
        Err(error) => format!("entries unavailable ({error})"),
    }
}

#[cfg(windows)]
pub(crate) fn run_owned_app(
    scope: &mut WindowsSmokeScope,
    app: &Path,
    args: &[String],
    env: &[(String, String)],
    timeout: std::time::Duration,
) -> Result<()> {
    let child = crate::process::spawn_owned_silent_filtered(
        app.as_os_str(),
        args,
        app.parent()
            .ok_or("installed app has no parent directory")?,
        env,
        &private_environment_names(),
    )?;
    scope.children.push(child);
    let status = scope
        .children
        .last_mut()
        .expect("child was appended")
        .wait_timeout(timeout)?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("installed app exited with {status}").into())
    }
}

#[cfg(windows)]
fn launch_smoke(
    scope: &mut WindowsSmokeScope,
    app: &Path,
    env: &[(String, String)],
    duration: std::time::Duration,
) -> Result<()> {
    let child = crate::process::spawn_owned_silent_filtered(
        app.as_os_str(),
        &[],
        app.parent()
            .ok_or("installed app has no parent directory")?,
        env,
        &private_environment_names(),
    )?;
    scope.children.push(child);
    std::thread::sleep(duration);
    let owned = scope
        .children
        .last_mut()
        .expect("launch child was appended");
    if let Some(status) = owned.try_wait()? {
        return Err(format!("packaged Tauri app exited during launch smoke: {status}").into());
    }
    owned.terminate()?;
    Ok(())
}

#[cfg(windows)]
pub(crate) fn wait_path_absent(path: &Path, timeout: std::time::Duration) -> Result<()> {
    let started = std::time::Instant::now();
    loop {
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(format!(
                    "could not inspect NSIS install root {}: {error}",
                    path.display()
                )
                .into());
            }
            Ok(_) => {}
        }
        if started.elapsed() >= timeout {
            return Err(format!(
                "NSIS uninstaller left the install root in place: {}; remaining entries: {}",
                path.display(),
                bounded_install_root_inventory(path)
            )
            .into());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[cfg(windows)]
pub(crate) struct WindowsSmokeScope {
    pub(crate) root: PathBuf,
    owner_bytes: Vec<u8>,
    registry: sky_windows_security::registry::RegistrySnapshot,
    shortcuts: ShortcutSnapshot,
    pub(crate) children: Vec<crate::process::OwnedChild>,
    runner_temp: PathBuf,
    workspace: PathBuf,
    previous_app_data: Option<std::ffi::OsString>,
}

#[cfg(windows)]
impl WindowsSmokeScope {
    pub(crate) fn enter(
        root: &Path,
        runner_temp: &Path,
        workspace: &Path,
        mode: RegistryMode,
    ) -> Result<Self> {
        let runner_temp = std::fs::canonicalize(runner_temp)
            .map_err(|error| format!("RUNNER_TEMP is unavailable: {error}"))?;
        let workspace = std::fs::canonicalize(workspace)
            .map_err(|error| format!("GITHUB_WORKSPACE is unavailable: {error}"))?;
        let parent = root.parent().ok_or("fixture root must have a parent")?;
        let parent = std::fs::canonicalize(parent)?;
        if parent != runner_temp {
            return Err("fixture root must be a direct child of RUNNER_TEMP".into());
        }
        if root.exists() {
            return Err("fixture root must not exist before the harness creates it".into());
        }
        std::fs::create_dir(root)?;
        let canonical_root = match std::fs::canonicalize(root) {
            Ok(path) => path,
            Err(error) => {
                let cleanup = std::fs::remove_dir(root);
                return Err(format!(
                    "could not canonicalize new fixture root: {error}; empty-root rollback: {}",
                    display_result(cleanup)
                )
                .into());
            }
        };
        if canonical_root.parent() != Some(runner_temp.as_path())
            || canonical_root.starts_with(&workspace)
        {
            let _ = std::fs::remove_dir(root);
            return Err(
                "fixture root resolves inside the repository or outside RUNNER_TEMP".into(),
            );
        }
        let sentinel = canonical_root.join(OWNER_SENTINEL);
        let mut sentinel_created = false;
        let sentinel_setup = (|| -> std::io::Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&sentinel)?;
            sentinel_created = true;
            std::io::Write::write_all(&mut file, OWNER_SENTINEL_CONTENT)?;
            file.sync_all()
        })();
        if let Err(entry_error) = sentinel_setup {
            let rollback = remove_incomplete_owned_root(
                &canonical_root,
                &runner_temp,
                &workspace,
                sentinel_created,
            );
            return Err(format!(
                "fixture ownership sentinel setup failed: {entry_error}; root rollback: {}",
                display_result(rollback)
            )
            .into());
        }

        let registry = match sky_windows_security::registry::capture(
            &sky_windows_security::registry::product_scopes(),
        ) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let rollback = remove_owned_root(&canonical_root, &runner_temp, &workspace);
                return Err(format!(
                    "registry snapshot failed: {error}; owned root rollback: {}",
                    display_result(rollback)
                )
                .into());
            }
        };
        let shortcuts = match ShortcutSnapshot::capture() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let restore = registry.restore();
                let remove = remove_owned_root(&canonical_root, &runner_temp, &workspace);
                return Err(format!(
                    "shortcut snapshot failed: {error}; registry rollback: {}; root rollback: {}",
                    display_result(restore),
                    display_result(remove)
                )
                .into());
            }
        };
        let previous_app_data = std::env::var_os("SKY_APP_DATA_ROOT");
        if mode == RegistryMode::FreshInstall
            && let Err(entry_error) = registry.clear_monitored()
        {
            let rollback = registry.restore();
            let shortcuts_restore = shortcuts.restore();
            let root_cleanup = remove_owned_root(&canonical_root, &runner_temp, &workspace);
            return Err(format!("fresh-install registry entry failed: {entry_error}; rollback: {}; shortcut restore: {}; root cleanup: {}", display_result(rollback), display_result(shortcuts_restore), display_result(root_cleanup)).into());
        }
        if let Err(entry_error) = std::fs::create_dir(canonical_root.join("app-data")) {
            let rollback = registry.restore();
            let shortcuts_restore = shortcuts.restore();
            let root_cleanup = remove_owned_root(&canonical_root, &runner_temp, &workspace);
            return Err(format!("AppData scope setup failed: {entry_error}; registry rollback: {}; shortcut restore: {}; root cleanup: {}", display_result(rollback), display_result(shortcuts_restore), display_result(root_cleanup)).into());
        }
        Ok(Self {
            root: canonical_root,
            owner_bytes: OWNER_SENTINEL_CONTENT.to_vec(),
            registry,
            shortcuts,
            children: Vec::new(),
            runner_temp,
            workspace,
            previous_app_data,
        })
    }

    pub(crate) fn child_environment(&self, app_data_root: &Path) -> Vec<(String, String)> {
        vec![(
            "SKY_APP_DATA_ROOT".to_owned(),
            app_data_root.to_string_lossy().into_owned(),
        )]
    }

    fn assert_shortcuts_unchanged(&self) -> Result<()> {
        self.shortcuts.assert_equivalent()
    }

    pub(crate) fn cleanup(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        attempt_cleanup(&mut errors, "terminate owned process trees", || {
            terminate_children(&mut self.children, |child| child.terminate())
        });
        attempt_cleanup(&mut errors, "restore monitored HKCU state", || {
            self.registry.restore().map_err(Into::into)
        });
        attempt_cleanup(&mut errors, "restore product shortcuts", || {
            self.shortcuts.restore()
        });
        attempt_cleanup(
            &mut errors,
            "restore parent SKY_APP_DATA_ROOT state",
            || {
                if std::env::var_os("SKY_APP_DATA_ROOT") != self.previous_app_data {
                    return Err("harness changed the parent SKY_APP_DATA_ROOT environment".into());
                }
                Ok(())
            },
        );
        attempt_cleanup(&mut errors, "remove owned fixture root", || {
            let sentinel = self.root.join(OWNER_SENTINEL);
            if std::fs::read(&sentinel)? != self.owner_bytes {
                return Err(
                    "fixture ownership sentinel changed; refusing recursive cleanup".into(),
                );
            }
            remove_owned_root(&self.root, &self.runner_temp, &self.workspace)
        });
        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!("NSIS cleanup stages failed: {}", errors.join("; ")).into())
        }
    }
}

#[cfg(windows)]
fn display_result<E: std::fmt::Display>(result: std::result::Result<(), E>) -> String {
    result.map_or_else(|error| error.to_string(), |_| "ok".to_owned())
}

fn attempt_cleanup(errors: &mut Vec<String>, stage: &str, action: impl FnOnce() -> Result<()>) {
    if let Err(error) = action() {
        errors.push(format!("{stage}: {error}"));
    }
}

fn terminate_children<T>(
    children: &mut [T],
    mut terminate: impl FnMut(&mut T) -> Result<()>,
) -> Result<()> {
    let mut errors = Vec::new();
    for (index, child) in children.iter_mut().enumerate() {
        if let Err(error) = terminate(child) {
            errors.push(format!("child {}: {error}", index + 1));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("owned child teardown failures: {}", errors.join("; ")).into())
    }
}

#[cfg(windows)]
fn remove_owned_root(root: &Path, runner_temp: &Path, workspace: &Path) -> Result<()> {
    use std::time::Duration;
    let runner_temp = std::fs::canonicalize(runner_temp)?;
    let workspace = std::fs::canonicalize(workspace)?;
    let metadata = std::fs::symlink_metadata(root)?;
    if !metadata.is_dir() || crate::ci_artifacts::is_reparse_point(root, &metadata) {
        return Err(
            "fixture root is not a regular owned directory; refusing recursive cleanup".into(),
        );
    }
    let canonical_root = std::fs::canonicalize(root)?;
    if canonical_root.parent() != Some(runner_temp.as_path())
        || canonical_root.starts_with(&workspace)
    {
        return Err("fixture root ownership boundary changed; refusing recursive cleanup".into());
    }
    let sentinel = canonical_root.join(OWNER_SENTINEL);
    let sentinel_metadata = std::fs::symlink_metadata(&sentinel)?;
    if !sentinel_metadata.is_file()
        || crate::ci_artifacts::is_reparse_point(&sentinel, &sentinel_metadata)
    {
        return Err(
            "fixture ownership sentinel is not a regular file; refusing recursive cleanup".into(),
        );
    }
    if std::fs::read(&sentinel)? != OWNER_SENTINEL_CONTENT {
        return Err("fixture ownership sentinel mismatch; refusing recursive cleanup".into());
    }
    let mut last_error = None;
    for _ in 0..10 {
        match std::fs::remove_dir_all(&canonical_root) {
            Ok(()) => return Ok(()),
            Err(_error) if !canonical_root.exists() => return Ok(()),
            Err(error) => last_error = Some(error.to_string()),
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(format!(
        "fixture root remains after bounded cleanup retries: {}; last error: {}",
        canonical_root.display(),
        last_error.unwrap_or_else(|| "unknown cleanup error".to_owned())
    )
    .into())
}

#[cfg(windows)]
fn remove_incomplete_owned_root(
    root: &Path,
    runner_temp: &Path,
    workspace: &Path,
    sentinel_created: bool,
) -> Result<()> {
    let runner_temp = std::fs::canonicalize(runner_temp)?;
    let workspace = std::fs::canonicalize(workspace)?;
    let root_metadata = std::fs::symlink_metadata(root)?;
    if !root_metadata.is_dir() || crate::ci_artifacts::is_reparse_point(root, &root_metadata) {
        return Err("new fixture root is not a regular directory".into());
    }
    let canonical_root = std::fs::canonicalize(root)?;
    if canonical_root.parent() != Some(runner_temp.as_path())
        || canonical_root.starts_with(&workspace)
    {
        return Err("new fixture root escaped the direct RUNNER_TEMP boundary".into());
    }
    let sentinel = canonical_root.join(OWNER_SENTINEL);
    match std::fs::symlink_metadata(&sentinel) {
        Ok(metadata)
            if metadata.is_file()
                && !crate::ci_artifacts::is_reparse_point(&sentinel, &metadata)
                && sentinel_created =>
        {
            let partial = std::fs::read(&sentinel)?;
            if !OWNER_SENTINEL_CONTENT.starts_with(&partial) {
                return Err(
                    "partial fixture sentinel changed unexpectedly; refusing root rollback".into(),
                );
            }
            std::fs::remove_file(&sentinel)?;
        }
        Ok(_) => {
            return Err(
                "unexpected fixture sentinel state; refusing incomplete root rollback".into(),
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !sentinel_created => {}
        Err(error) => return Err(error.into()),
    }
    std::fs::remove_dir(canonical_root)?;
    Ok(())
}

#[cfg(windows)]
struct ShortcutSnapshot {
    files: BTreeMap<PathBuf, Option<Vec<u8>>>,
}

#[cfg(windows)]
impl ShortcutSnapshot {
    fn capture() -> Result<Self> {
        let app_data = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or("APPDATA is unavailable on the hosted runner")?;
        let profile = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .ok_or("USERPROFILE is unavailable on the hosted runner")?;
        let candidates = [
            profile.join("Desktop/Sky Auto Player.lnk"),
            app_data.join("Microsoft/Windows/Start Menu/Programs/Sky Auto Player.lnk"),
            app_data
                .join("Microsoft/Windows/Start Menu/Programs/Sky Auto Player/Sky Auto Player.lnk"),
        ];
        let mut files = BTreeMap::new();
        for path in candidates {
            let value = match std::fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_file() => Some(std::fs::read(&path)?),
                Ok(_) => {
                    return Err(format!(
                        "product shortcut path is not a regular file: {}",
                        path.display()
                    )
                    .into());
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(
                        format!("could not snapshot shortcut {}: {error}", path.display()).into(),
                    );
                }
            };
            files.insert(path, value);
        }
        Ok(Self { files })
    }

    fn restore(&self) -> Result<()> {
        let mut errors = Vec::new();
        for (path, expected) in &self.files {
            let result = match expected {
                Some(bytes) => (|| -> std::io::Result<()> {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(path, bytes)
                })(),
                None => match std::fs::symlink_metadata(path) {
                    Ok(metadata) if metadata.file_type().is_file() => std::fs::remove_file(path),
                    Ok(_) => Err(std::io::Error::other("shortcut path changed type")),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error),
                },
            };
            if let Err(error) = result {
                errors.push(format!("{}: {error}", path.display()));
            }
        }
        if errors.is_empty() {
            self.assert_equivalent()
        } else {
            Err(format!("shortcut restore failed: {}", errors.join("; ")).into())
        }
    }

    fn assert_equivalent(&self) -> Result<()> {
        for (path, expected) in &self.files {
            let actual = match std::fs::symlink_metadata(path) {
                Ok(metadata) if metadata.file_type().is_file() => Some(std::fs::read(path)?),
                Ok(_) => {
                    return Err(
                        format!("product shortcut path changed type: {}", path.display()).into(),
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(
                        format!("could not verify shortcut {}: {error}", path.display()).into(),
                    );
                }
            };
            if &actual != expected {
                return Err(format!(
                    "product shortcut state differs after smoke: {}",
                    path.display()
                )
                .into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn registry_mode_names_are_explicit_and_distinct() {
        assert_eq!(
            RegistryMode::parse("fresh-install").unwrap(),
            RegistryMode::FreshInstall
        );
        assert_eq!(
            RegistryMode::parse("preserve-existing-state").unwrap(),
            RegistryMode::PreserveExistingState
        );
        assert!(RegistryMode::parse("fresh").is_err());
    }

    #[test]
    fn options_reject_unknown_duplicate_and_missing_values() {
        assert!(parse_options(&["--unknown".to_owned(), "x".to_owned()]).is_err());
        assert!(parse_options(&["--installer".to_owned()]).is_err());
        assert!(
            parse_options(&[
                "--installer".to_owned(),
                "one.exe".to_owned(),
                "--installer".to_owned(),
                "two.exe".to_owned(),
            ])
            .is_err()
        );
    }

    #[test]
    fn authenticode_mode_rejects_unknown_policy() {
        assert!(validate_authenticode_mode("unsigned-zero-budget").is_ok());
        assert!(validate_authenticode_mode("best-effort").is_err());
    }

    #[test]
    fn real_installs_are_allowed_only_on_github_hosted_runners() {
        assert!(is_hosted_disposable_runner(
            Some("true"),
            Some("github-hosted")
        ));
        assert!(!is_hosted_disposable_runner(
            Some("true"),
            Some("self-hosted")
        ));
        assert!(!is_hosted_disposable_runner(
            Some("false"),
            Some("github-hosted")
        ));
        assert!(!is_hosted_disposable_runner(None, None));
        assert!(require_runner_authorization(RunnerAuthorization::None).is_err());
    }

    #[test]
    fn protected_release_authorization_is_bound_to_exact_main_workflow_jobs() {
        assert!(is_protected_release_context(
            Some("true"),
            Some("V4 Release Pipeline"),
            Some("release"),
            Some("workflow_dispatch"),
            Some("refs/heads/main"),
            Some("pumni/Sky-Auto-Player"),
        ));
        assert!(is_protected_release_context(
            Some("true"),
            Some("V4 Production Pre-Publication Qualification"),
            Some("qualification"),
            Some("workflow_dispatch"),
            Some("refs/heads/main"),
            Some("pumni/Sky-Auto-Player"),
        ));
        assert!(!is_protected_release_context(
            Some("true"),
            Some("V4 Release Pipeline"),
            Some("release"),
            Some("push"),
            Some("refs/heads/main"),
            Some("pumni/Sky-Auto-Player"),
        ));
        assert!(!is_protected_release_context(
            Some("true"),
            Some("V4 Release Pipeline"),
            Some("release"),
            Some("workflow_dispatch"),
            Some("refs/heads/codex/tooling"),
            Some("pumni/Sky-Auto-Player"),
        ));
        assert!(!is_protected_release_context(
            Some("true"),
            Some("CI"),
            Some("packaged"),
            Some("workflow_dispatch"),
            Some("refs/heads/main"),
            Some("pumni/Sky-Auto-Player"),
        ));
    }

    #[cfg(not(windows))]
    #[test]
    fn installer_command_fails_closed_off_windows() {
        let result = run(Path::new("."), &["smoke".to_owned()]);
        assert!(result.unwrap_err().to_string().contains("Windows-only"));
    }

    #[test]
    fn cleanup_attempts_every_stage_and_preserves_original_failure() {
        let mut attempted = Vec::new();
        let mut cleanup_errors = Vec::new();
        for stage in [
            "processes",
            "registry",
            "shortcuts",
            "environment",
            "filesystem",
        ] {
            let stage_name = stage.to_owned();
            attempt_cleanup(&mut cleanup_errors, stage, || {
                attempted.push(stage_name.clone());
                Err(format!("injected {stage_name} failure").into())
            });
        }
        let cleanup = Err::<(), _>(cleanup_errors.join("; ").into());
        let result = combine_operation_and_cleanup::<()>(
            Err("injected original operation failure".into()),
            cleanup,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            attempted,
            [
                "processes",
                "registry",
                "shortcuts",
                "environment",
                "filesystem"
            ]
        );
        assert!(result.contains("injected original operation failure"));
        assert!(result.contains("injected filesystem failure"));
    }

    #[test]
    fn child_cleanup_attempts_later_children_and_aggregates_failures() {
        let mut children = ["first", "second", "third"];
        let mut attempted = Vec::new();
        let error = terminate_children(&mut children, |child| {
            attempted.push(*child);
            if *child == "first" {
                Err("injected first-child teardown failure".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err()
        .to_string();

        assert_eq!(attempted, ["first", "second", "third"]);
        assert!(error.contains("child 1: injected first-child teardown failure"));
    }

    #[test]
    fn owner_sentinel_and_reserved_name_are_bounded() {
        assert_eq!(OWNER_SENTINEL, ".sky-auto-player-wave-c-owner");
        assert!(OWNER_SENTINEL_CONTENT.len() < 128);
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        assert!(unique > 0);
    }

    #[cfg(windows)]
    fn synthetic_roots() -> (PathBuf, PathBuf, PathBuf) {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "sky-nsis-scope-test-{}-{nonce:x}",
            std::process::id()
        ));
        let runner_temp = base.join("runner-temp");
        let workspace = base.join("workspace");
        std::fs::create_dir_all(&runner_temp).expect("create synthetic runner temp");
        std::fs::create_dir_all(&workspace).expect("create synthetic workspace");
        (base, runner_temp, workspace)
    }

    #[cfg(windows)]
    #[test]
    fn installed_calibration_uses_candidate_bytes_without_producer_staging() {
        let (base, runner_temp, workspace) = synthetic_roots();
        let repository = workspace.join("consumer-checkout");
        let install = runner_temp.join("consumer-install");
        std::fs::create_dir(&repository).expect("create synthetic consumer checkout");
        std::fs::create_dir(&install).expect("create synthetic install root");
        let staged_resources = repository.join("desktop/src-tauri/resources");
        assert!(!staged_resources.exists());

        let expected_bytes = synthetic_x64_pe();
        let expected_path = runner_temp.join("expected-native-calibration.exe");
        std::fs::write(&expected_path, &expected_bytes).expect("write expected fixture bytes");
        let expected_sha256 =
            crate::ci_artifacts::sha256_file(&expected_path).expect("hash expected fixture bytes");
        let candidate = crate::ci_candidate::CandidateContract {
            manifest: crate::ci_candidate::CandidateManifest {
                schema_version: 2,
                source_sha: "a".repeat(40),
                version: "4.2.0".to_owned(),
                calibration_binary: "native_calibration.exe".to_owned(),
                calibration_binary_size: expected_bytes.len() as u64,
                calibration_binary_sha256: expected_sha256,
                calibration_binary_architecture: "x86_64".to_owned(),
                installer: "installer.exe".to_owned(),
                installer_sha256: String::new(),
                updater_signature: "installer.exe.sig".to_owned(),
                updater_signature_sha256: String::new(),
                updater_public_key: "updater-public-key.pub".to_owned(),
                updater_public_key_sha256: String::new(),
            },
            installer_sha256: String::new(),
            signature_sha256: String::new(),
            public_key_sha256: String::new(),
        };

        let missing = verify_calibration_artifact(&repository, &install, &candidate, None)
            .expect_err("missing installed calibration must fail closed")
            .to_string();
        assert!(missing.contains("installed native_calibration.exe is missing"));

        let mut wrong_bytes = expected_bytes;
        wrong_bytes[0x20] ^= 1;
        std::fs::write(install.join("native_calibration.exe"), wrong_bytes)
            .expect("write wrong installed fixture bytes");
        let mismatch = verify_calibration_artifact(&repository, &install, &candidate, None)
            .expect_err("installed calibration must match the frozen candidate digest")
            .to_string();
        assert!(mismatch.contains("installed calibration SHA-256 mismatch"));
        assert!(!staged_resources.exists());

        std::fs::remove_dir_all(base).expect("remove synthetic consumer fixture");
    }

    #[cfg(windows)]
    fn synthetic_x64_pe() -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x86];
        bytes[0..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80_i32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&0x8664_u16.to_le_bytes());
        bytes
    }

    #[cfg(windows)]
    #[test]
    fn owned_root_cleanup_requires_exact_sentinel_and_direct_runner_temp_scope() {
        let (base, runner_temp, workspace) = synthetic_roots();
        let root = runner_temp.join("owned-smoke-root");
        std::fs::create_dir(&root).expect("create isolated owned root");
        std::fs::write(root.join(OWNER_SENTINEL), OWNER_SENTINEL_CONTENT)
            .expect("write owner sentinel");
        std::fs::create_dir(root.join("synthetic-nested-residue"))
            .expect("create synthetic residue");
        std::fs::write(
            root.join("synthetic-nested-residue").join("safe-test.txt"),
            b"fixture-owned",
        )
        .expect("write synthetic residue");

        remove_owned_root(&root, &runner_temp, &workspace).expect("remove sentinel-owned root");
        assert!(
            !root.exists(),
            "owned root must be removed after verified cleanup"
        );

        let tampered = runner_temp.join("tampered-sentinel-root");
        std::fs::create_dir(&tampered).expect("create tampered sentinel root");
        std::fs::write(tampered.join(OWNER_SENTINEL), b"not the owner sentinel")
            .expect("write tampered sentinel");
        assert!(remove_owned_root(&tampered, &runner_temp, &workspace).is_err());
        assert!(
            tampered.exists(),
            "cleanup must retain a root with changed ownership proof"
        );

        let unscoped = workspace.join("unscoped-root");
        std::fs::create_dir(&unscoped).expect("create unscoped path");
        std::fs::write(unscoped.join(OWNER_SENTINEL), OWNER_SENTINEL_CONTENT)
            .expect("write sentinel outside runner temp");
        assert!(remove_owned_root(&unscoped, &runner_temp, &workspace).is_err());
        assert!(
            unscoped.exists(),
            "cleanup must reject a root outside RUNNER_TEMP"
        );

        std::fs::remove_dir_all(base).expect("remove only synthetic test roots");
    }

    #[cfg(windows)]
    #[test]
    fn incomplete_root_rollback_accepts_only_a_prefix_of_the_created_sentinel() {
        let (base, runner_temp, workspace) = synthetic_roots();
        let root = runner_temp.join("incomplete-smoke-root");
        std::fs::create_dir(&root).expect("create isolated incomplete root");
        let partial = &OWNER_SENTINEL_CONTENT[..8];
        std::fs::write(root.join(OWNER_SENTINEL), partial).expect("write partial sentinel");
        remove_incomplete_owned_root(&root, &runner_temp, &workspace, true)
            .expect("rollback partial owner marker");
        assert!(!root.exists());

        let foreign = runner_temp.join("foreign-incomplete-root");
        std::fs::create_dir(&foreign).expect("create foreign incomplete root");
        std::fs::write(foreign.join(OWNER_SENTINEL), b"foreign").expect("write foreign sentinel");
        assert!(remove_incomplete_owned_root(&foreign, &runner_temp, &workspace, true).is_err());
        assert!(
            foreign.exists(),
            "rollback must fail closed for a foreign marker"
        );
        std::fs::remove_dir_all(base).expect("remove only synthetic test roots");
    }

    #[cfg(windows)]
    #[test]
    fn install_root_absence_polling_has_a_deadline() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sky-nsis-wait-bound-test-{}-{nonce:x}",
            std::process::id()
        ));
        std::fs::create_dir(&root).expect("create synthetic install root");
        for index in 0..20 {
            std::fs::write(root.join(format!("item-{index:02}.bin")), b"synthetic")
                .expect("write synthetic leftover entry");
        }
        let started = std::time::Instant::now();
        let result = wait_path_absent(&root, std::time::Duration::from_millis(30));
        let error = result.expect_err("present synthetic root must hit its deadline");
        assert!(error.to_string().contains("remaining entries"));
        assert!(error.to_string().contains("item-00.bin"));
        assert!(error.to_string().contains("additional entries omitted"));
        assert!(
            error.to_string().len() < 3000,
            "inventory must remain bounded"
        );
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
        std::fs::remove_dir_all(&root).expect("remove synthetic install root");
    }

    #[cfg(windows)]
    #[test]
    fn uninstaller_working_directory_is_outside_the_exact_install_root() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sky-nsis-uninstaller-cwd-test-{}-{unique:x}",
            std::process::id()
        ));
        let install_root = root.join("install");
        std::fs::create_dir_all(&install_root).expect("create synthetic install root");
        let uninstaller = install_root.join("uninstall.exe");
        std::fs::write(&uninstaller, b"synthetic test file").expect("write fake uninstaller");

        let cwd = uninstaller_working_directory(&uninstaller, &install_root)
            .expect("direct uninstaller must run from the owned root's parent");
        let canonical_install = std::fs::canonicalize(&install_root).unwrap();
        assert!(!cwd.starts_with(&canonical_install));
        assert_eq!(cwd, std::fs::canonicalize(&root).unwrap());

        let foreign = root.join("foreign-uninstaller.exe");
        std::fs::write(&foreign, b"synthetic foreign file").expect("write foreign file");
        assert!(uninstaller_working_directory(&foreign, &install_root).is_err());
        std::fs::remove_dir_all(root).expect("remove synthetic uninstaller tree");
    }

    #[cfg(windows)]
    #[test]
    fn fresh_catalog_guard_accepts_empty_songs_and_rejects_nested_user_files() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sky-auto-player-empty-songs-test-{}-{unique:x}",
            std::process::id()
        ));
        let songs = root.join("songs");
        std::fs::create_dir_all(&songs).unwrap();
        assert_user_songs_empty(&songs).unwrap();
        let nested = songs.join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("user-song.json"), b"synthetic test song").unwrap();
        assert!(assert_user_songs_empty(&songs).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
