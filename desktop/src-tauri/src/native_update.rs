//! Native update policy and handoff orchestration.
//!
//! The application owns selection, preference correlation, and the handoff
//! request.  Downloading, verification, transaction, rollback, and restart
//! remain in `sky_updater`; this module never accepts a caller supplied URL or
//! artifact path.

use crate::commands::{UpdateCheckDto, UpdateHandoffDto};
use crate::native_runtime::TestSeams;
use crate::ui_events::{
    UiEvent, UpdateAvailablePayload, UpdateChannel, UpdateHandoffReadyPayload, UpdateResultPayload,
    UpdateState,
};
use serde::Deserialize;
use sky_app_core::settings::{ApplicationSettings, SettingsService, UpdateChannel as CoreChannel};
use sky_native_adapters::JsonSettingsStore;
use sky_updater::archive::sha256_file;
use sky_updater::cli::Channel as UpdaterChannel;
use sky_updater::handoff::{Handoff, handoff_path};
use sky_updater::http::{HttpClient, WinHttpClient, validate_https_url};
use sky_updater::install::installed_manifest;
use sky_updater::signature::verify_project_files;
use sky_updater::{APP_NAME, MANIFEST_NAME, UPDATER_EXE};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

const MAX_RELEASE_NOTES: usize = 16 * 1024;
const MAX_HANDOFF_BYTES: u64 = 16 * 1024;
const HANDOFF_TIMEOUT: Duration = Duration::from_secs(5);
const HANDOFF_POLL: Duration = Duration::from_millis(50);

#[derive(Clone, Debug)]
pub(crate) struct NativeUpdateCandidate {
    pub version: String,
    pub channel: UpdateChannel,
    pub release_notes: Option<String>,
    pub published_at: Option<String>,
}

#[derive(Default)]
pub(crate) struct NativeUpdateState {
    pub candidate: Option<NativeUpdateCandidate>,
    pub handoff_id: Option<String>,
    pub handoff_starting: bool,
}

#[derive(Debug, Deserialize, Clone)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Deserialize, Clone)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
}

pub(crate) fn check(
    settings: &mut SettingsService<JsonSettingsStore>,
    state: &Mutex<NativeUpdateState>,
    test_seams: TestSeams,
    publish: impl Fn(UiEvent) -> Result<(), String>,
) -> Result<UpdateCheckDto, String> {
    let settings_snapshot = settings.snapshot().clone();
    let channel = to_public_channel(&settings_snapshot.update.channel);
    let current_version = env!("CARGO_PKG_VERSION").to_owned();
    let result = if test_seams == TestSeams::SafePackage {
        Ok(None)
    } else {
        fetch_release(&settings_snapshot, channel, &WinHttpClient)
    };
    let now = unix_timestamp();
    match result {
        Ok(Some(candidate)) => {
            settings
                .record_update_success(now)
                .map_err(|error| format!("update timestamp persistence failed: {error}"))?;
            let mut update = state
                .lock()
                .map_err(|_| "native update state lock poisoned".to_string())?;
            if update.handoff_starting {
                return Err("update_busy: update handoff is already starting".into());
            }
            update.candidate = Some(candidate.clone());
            update.handoff_id = None;
            update.handoff_starting = false;
            let dto = UpdateCheckDto {
                state: UpdateState::Available,
                current_version: current_version.clone(),
                available_version: Some(candidate.version.clone()),
                channel,
                release_notes: candidate.release_notes.clone(),
                published_at: candidate.published_at.clone(),
                error: None,
            };
            publish(UiEvent::UpdateAvailable {
                v: crate::DESKTOP_PROTOCOL_VERSION,
                payload: UpdateAvailablePayload {
                    current_version: current_version.clone(),
                    available_version: candidate.version,
                    channel,
                    release_notes: candidate.release_notes,
                    published_at: candidate.published_at,
                },
            })?;
            publish_result(&publish, &dto)?;
            Ok(dto)
        }
        Ok(None) => {
            settings
                .record_update_success(now)
                .map_err(|error| format!("update timestamp persistence failed: {error}"))?;
            let mut update = state
                .lock()
                .map_err(|_| "native update state lock poisoned".to_string())?;
            if update.handoff_starting {
                return Err("update_busy: update handoff is already starting".into());
            }
            update.candidate = None;
            update.handoff_id = None;
            update.handoff_starting = false;
            let dto = UpdateCheckDto {
                state: UpdateState::Current,
                current_version: current_version.clone(),
                available_version: None,
                channel,
                release_notes: None,
                published_at: None,
                error: None,
            };
            publish_result(&publish, &dto)?;
            Ok(dto)
        }
        Err(error) => {
            let _ = settings.record_update_error(now);
            let mut update = state
                .lock()
                .map_err(|_| "native update state lock poisoned".to_string())?;
            if update.handoff_starting {
                return Err("update_busy: update handoff is already starting".into());
            }
            update.candidate = None;
            update.handoff_id = None;
            update.handoff_starting = false;
            let message = bounded(error);
            let dto = UpdateCheckDto {
                state: UpdateState::Error,
                current_version: current_version.clone(),
                available_version: None,
                channel,
                release_notes: None,
                published_at: None,
                error: Some(message),
            };
            publish_result(&publish, &dto)?;
            Ok(dto)
        }
    }
}

pub(crate) fn handoff(
    install_root: &Path,
    state: &Mutex<NativeUpdateState>,
    settings: &ApplicationSettings,
    requested_target: &str,
    publish: impl Fn(UiEvent) -> Result<(), String>,
) -> Result<UpdateHandoffDto, String> {
    let candidate = state
        .lock()
        .map_err(|_| "native update state lock poisoned".to_string())?
        .candidate
        .clone()
        .ok_or_else(|| "update_unavailable: check for an update first".to_string())?;
    let target = candidate.version.clone();
    if requested_target != target {
        return Err("stale_update: update metadata is stale".into());
    }
    if settings.update.channel != to_core_channel(candidate.channel)
        || settings.update.skip_version == target
    {
        return Err("stale_update: update metadata is stale".into());
    }

    // Reserve the application-level handoff before any filesystem or process
    // side effect.  A second caller observes `handoff_starting` and cannot
    // stage or spawn a competing updater.
    {
        let mut locked = state
            .lock()
            .map_err(|_| "native update state lock poisoned".to_string())?;
        if locked.handoff_starting {
            return Err("update_busy: update handoff is already starting".into());
        }
        if let Some(id) = locked.handoff_id.clone() {
            return Ok(UpdateHandoffDto {
                handoff_id: id,
                target_version: target,
                state: UpdateState::HandoffReady,
            });
        }
        if locked
            .candidate
            .as_ref()
            .map(|value| value.version.as_str())
            != Some(target.as_str())
        {
            return Err("stale_update: update metadata is stale".into());
        }
        locked.handoff_starting = true;
    }
    let current = env!("CARGO_PKG_VERSION");
    let channel = match settings.update.channel {
        CoreChannel::Stable => UpdaterChannel::Stable,
        CoreChannel::Beta => UpdaterChannel::Beta,
    };
    match sky_updater::active_state::active_update_for_install(install_root) {
        Ok(Some(_)) => {
            clear_handoff_starting(state);
            return Err("update_busy: another update is already active".into());
        }
        Ok(None) => {}
        Err(error) => {
            clear_handoff_starting(state);
            return Err(format!("update state check failed: {error}"));
        }
    }
    if let Err(error) = preflight_install_root_writable(install_root) {
        clear_handoff_starting(state);
        return Err(error);
    }
    let run_root = match new_run_root() {
        Ok(path) => path,
        Err(error) => {
            clear_handoff_starting(state);
            return Err(error);
        }
    };
    let staged = run_root.join(UPDATER_EXE);
    let child = match stage_and_spawn(install_root, &staged, current, &target, channel) {
        Ok(child) => child,
        Err(error) => {
            let _ = fs::remove_dir_all(&run_root);
            clear_handoff_starting(state);
            return Err(error);
        }
    };
    let ready = match wait_for_ready(child, &run_root, &target) {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_dir_all(&run_root);
            clear_handoff_starting(state);
            return Err(error);
        }
    };
    if ready.state != "ready" {
        let _ = fs::remove_dir_all(&run_root);
        clear_handoff_starting(state);
        return Err(handoff_rejection_error(&ready));
    }
    let handoff_id = match opaque_id() {
        Ok(id) => id,
        Err(error) => {
            let _ = fs::remove_dir_all(&run_root);
            clear_handoff_starting(state);
            return Err(error);
        }
    };
    {
        let mut locked = state
            .lock()
            .map_err(|_| "native update state lock poisoned".to_string())?;
        locked.handoff_id = Some(handoff_id.clone());
        locked.handoff_starting = false;
    }
    let dto = UpdateHandoffDto {
        handoff_id: handoff_id.clone(),
        target_version: target.clone(),
        state: UpdateState::HandoffReady,
    };
    publish(UiEvent::UpdateHandoffReady {
        v: crate::DESKTOP_PROTOCOL_VERSION,
        payload: UpdateHandoffReadyPayload {
            handoff_id,
            target_version: target,
        },
    })?;
    Ok(dto)
}

fn fetch_release(
    settings: &ApplicationSettings,
    channel: UpdateChannel,
    client: &impl HttpClient,
) -> Result<Option<NativeUpdateCandidate>, String> {
    let path = match channel {
        UpdateChannel::Stable => "/releases/latest",
        UpdateChannel::Beta => "/releases?per_page=10",
    };
    let url = format!("https://api.github.com/repos/pumni/{APP_NAME}{path}");
    validate_https_url(&url).map_err(|error| error.to_string())?;
    let bytes = client
        .get(&url, sky_updater::API_MAX_BYTES)
        .map_err(|error| error.to_string())?;
    let release = if channel == UpdateChannel::Beta {
        let releases: Vec<Release> = serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid release metadata: {error}"))?;
        releases
            .into_iter()
            .filter(|release| !release.draft)
            .filter_map(|release| {
                let version = normalize_tag(&release.tag_name)?;
                let parsed = sky_updater::version::Pep440Version::parse(&version).ok()?;
                Some((parsed, release))
            })
            .max_by(|left, right| left.0.cmp(&right.0))
            .map(|(_, release)| release)
            .ok_or_else(|| "no valid releases found".to_string())?
    } else {
        serde_json::from_slice::<Release>(&bytes)
            .map_err(|error| format!("invalid release metadata: {error}"))?
    };
    if release.draft || (channel == UpdateChannel::Stable && release.prerelease) {
        return Ok(None);
    }
    let Some(version) = normalize_tag(&release.tag_name) else {
        return Err("missing or malformed release tag".into());
    };
    let current = sky_updater::version::Pep440Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|error| error.to_string())?;
    let parsed =
        sky_updater::version::Pep440Version::parse(&version).map_err(|error| error.to_string())?;
    if parsed <= current || settings.update.skip_version == version {
        return Ok(None);
    }
    let zip_name = format!("{APP_NAME}-v{version}.zip");
    let required = [
        zip_name.as_str(),
        &format!("{zip_name}.sha256"),
        MANIFEST_NAME,
    ];
    for name in required {
        let matches = release
            .assets
            .iter()
            .filter(|asset| asset.name == name)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err("release is missing canonical verification assets".into());
        }
        validate_https_url(&matches[0].browser_download_url).map_err(|error| error.to_string())?;
    }
    Ok(Some(NativeUpdateCandidate {
        version,
        channel,
        release_notes: release
            .body
            .map(|value| value.chars().take(MAX_RELEASE_NOTES).collect()),
        published_at: release.published_at,
    }))
}

fn publish_result(
    publish: &impl Fn(UiEvent) -> Result<(), String>,
    dto: &UpdateCheckDto,
) -> Result<(), String> {
    publish(UiEvent::UpdateResult {
        v: crate::DESKTOP_PROTOCOL_VERSION,
        payload: UpdateResultPayload {
            state: dto.state,
            current_version: dto.current_version.clone(),
            available_version: dto.available_version.clone(),
            channel: dto.channel,
            error: dto.error.clone(),
        },
    })
}

fn normalize_tag(tag: &str) -> Option<String> {
    let value = tag.trim();
    let value = value.strip_prefix(['v', 'V']).unwrap_or(value);
    (!value.is_empty()).then(|| value.to_owned())
}

fn to_public_channel(channel: &CoreChannel) -> UpdateChannel {
    match channel {
        CoreChannel::Stable => UpdateChannel::Stable,
        CoreChannel::Beta => UpdateChannel::Beta,
    }
}

fn to_core_channel(channel: UpdateChannel) -> CoreChannel {
    match channel {
        UpdateChannel::Stable => CoreChannel::Stable,
        UpdateChannel::Beta => CoreChannel::Beta,
    }
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or_default()
}

fn bounded(value: impl ToString) -> String {
    value.to_string().chars().take(4096).collect()
}

fn clear_handoff_starting(state: &Mutex<NativeUpdateState>) {
    if let Ok(mut locked) = state.lock() {
        locked.handoff_starting = false;
    }
}

fn handoff_rejection_error(ready: &Handoff) -> String {
    if ready.error_code == "UPDATE_ALREADY_RUNNING" {
        return "update_busy: another update is already active".into();
    }
    format!(
        "update_handoff_rejected: [{}] {}",
        bounded(&ready.error_code),
        bounded(&ready.message)
    )
}

fn opaque_id() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| format!("secure update identifier failed: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn new_run_root() -> Result<PathBuf, String> {
    let local = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?;
    allocate_run_root(&PathBuf::from(local).join(APP_NAME))
}

fn allocate_run_root(state_root: &Path) -> Result<PathBuf, String> {
    ensure_secure_directory(state_root, "update state root")?;
    let runs = state_root.join("update-runs");
    ensure_secure_directory(&runs, "update run root")?;
    let canonical = runs
        .canonicalize()
        .map_err(|error| format!("could not resolve update state: {error}"))?;
    for _ in 0..16 {
        let id = opaque_id()?;
        let path = runs.join(format!("run-{id}"));
        match fs::create_dir(&path) {
            Ok(()) => {
                if path
                    .canonicalize()
                    .ok()
                    .and_then(|value| value.parent().map(Path::to_owned))
                    != Some(canonical.clone())
                {
                    let _ = fs::remove_dir_all(&path);
                    return Err("update run escaped allow-listed state root".into());
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("could not allocate update run: {error}")),
        }
    }
    Err("could not allocate unique update run".into())
}

fn ensure_secure_directory(path: &Path, label: &str) -> Result<(), String> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata_is_redirected(&metadata) {
            return Err(format!("{label} must not be a symlink or reparse point"));
        }
        if !metadata.is_dir() {
            return Err(format!("{label} is not a directory"));
        }
        return Ok(());
    }
    fs::create_dir_all(path).map_err(|error| format!("could not create {label}: {error}"))?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect {label}: {error}"))?;
    if metadata_is_redirected(&metadata) || !metadata.is_dir() {
        return Err(format!("{label} failed secure directory admission"));
    }
    Ok(())
}

fn metadata_is_redirected(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn preflight_install_root_writable(install_root: &Path) -> Result<(), String> {
    let sentinel = format!(
        ".sky-auto-player-update-write-{}",
        opaque_id()
            .map_err(|error| format!("could not allocate update preflight name: {error}"))?
    );
    preflight_install_root_writable_with_cleanup(install_root, &sentinel, |path| {
        fs::remove_file(path).map_err(|error| error.to_string())
    })
}

fn preflight_install_root_writable_with_cleanup<F>(
    install_root: &Path,
    sentinel_name: &str,
    cleanup: F,
) -> Result<(), String>
where
    F: FnOnce(&Path) -> Result<(), String>,
{
    let sentinel = install_root.join(sentinel_name);
    let mut created = false;
    let write_result = (|| -> Result<(), String> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&sentinel)
            .map_err(|error| format!("install root is not writable: {error}"))?;
        created = true;
        file.write_all(b"Sky Auto Player update preflight\n")
            .map_err(|error| format!("install root is not writable: {error}"))?;
        file.flush()
            .map_err(|error| format!("install root is not writable: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("install root is not durable: {error}"))?;
        Ok(())
    })();
    if created && let Err(error) = cleanup(&sentinel) {
        return Err(format!(
            "install root write preflight cleanup failed: {error}"
        ));
    }
    write_result
}

fn stage_and_spawn(
    install_root: &Path,
    staged: &Path,
    current: &str,
    target: &str,
    channel: UpdaterChannel,
) -> Result<Child, String> {
    let manifest = installed_manifest(install_root).map_err(|error| error.to_string())?;
    manifest
        .validate(Some(current))
        .map_err(|error| error.to_string())?;
    verify_project_files(install_root, &manifest).map_err(|error| error.to_string())?;
    let updater = install_root.join(UPDATER_EXE);
    let entry = manifest
        .files
        .iter()
        .find(|file| file.path == UPDATER_EXE)
        .ok_or("installed manifest has no updater entry")?;
    let metadata = fs::symlink_metadata(&updater).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != entry.size {
        return Err("installed updater failed manifest preflight".into());
    }
    if sha256_file(&updater).map_err(|error| error.to_string())?
        != entry.sha256.to_ascii_lowercase()
    {
        return Err("installed updater hash failed manifest preflight".into());
    }
    fs::copy(&updater, staged).map_err(|error| format!("could not stage updater: {error}"))?;
    let staged_meta = fs::metadata(staged).map_err(|error| error.to_string())?;
    if staged_meta.len() != entry.size
        || sha256_file(staged).map_err(|error| error.to_string())?
            != entry.sha256.to_ascii_lowercase()
    {
        return Err("staged updater hash mismatch".into());
    }
    let channel = match channel {
        UpdaterChannel::Stable => "stable",
        UpdaterChannel::Beta => "beta",
    };
    Command::new(staged)
        .args([
            "--install-root",
            &install_root.to_string_lossy(),
            "--parent-pid",
            &std::process::id().to_string(),
            "--current-version",
            current,
            "--target-version",
            target,
            "--channel",
            channel,
            "--restart",
        ])
        .current_dir(install_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not launch verified updater: {error}"))
}

fn wait_for_ready(mut child: Child, run_root: &Path, target: &str) -> Result<Handoff, String> {
    let deadline = std::time::Instant::now() + HANDOFF_TIMEOUT;
    let path = handoff_path(run_root);
    while std::time::Instant::now() < deadline {
        if let Ok(metadata) = fs::metadata(&path)
            && metadata.len() <= MAX_HANDOFF_BYTES
            && let Ok(bytes) = fs::read(&path)
            && let Ok(value) = serde_json::from_slice::<Handoff>(&bytes)
            && value.schema_version == 1
            && value.run_id
                == run_root
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
            && value.updater_pid == child.id()
            && value.target_version == target
        {
            return Ok(value);
        }
        if child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Err("updater exited before ready handshake".into());
        }
        thread::sleep(HANDOFF_POLL);
    }
    let _ = child.kill();
    let _ = child.wait();
    Err("updater did not complete ready handshake".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn tag_normalization_and_channel_mapping_are_bounded() {
        assert_eq!(normalize_tag(" v3.5.1 "), Some("3.5.1".into()));
        assert_eq!(normalize_tag("V3.5.1"), Some("3.5.1".into()));
        assert!(normalize_tag("v").is_none());
        assert_eq!(to_core_channel(UpdateChannel::Beta), CoreChannel::Beta);
    }

    #[test]
    fn update_run_names_are_opaque_32_hex_suffixes() {
        let id = opaque_id().expect("id");
        assert_eq!(id.len(), 32);
        assert!(
            id.bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
    }

    fn temp_path(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("sky-native-update-{label}-{suffix}"))
    }

    #[test]
    fn handoff_starting_reservation_blocks_a_competing_operation() {
        let state = Mutex::new(NativeUpdateState {
            candidate: Some(NativeUpdateCandidate {
                version: "9.9.9".into(),
                channel: UpdateChannel::Stable,
                release_notes: None,
                published_at: None,
            }),
            handoff_id: None,
            handoff_starting: true,
        });
        let settings = ApplicationSettings::default();
        let error = handoff(
            Path::new("missing-install-root"),
            &state,
            &settings,
            "9.9.9",
            |_| Ok(()),
        )
        .expect_err("reserved handoff must not spawn a second updater");
        assert!(error.starts_with("update_busy: update handoff is already starting"));
    }

    #[test]
    fn only_the_established_already_running_handshake_maps_to_busy() {
        let rejected = |error_code: &str| Handoff {
            schema_version: 1,
            state: "rejected".into(),
            run_id: "run-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            updater_pid: 1,
            target_version: "9.9.9".into(),
            error_code: error_code.into(),
            message: "bounded reason".into(),
        };
        assert_eq!(
            handoff_rejection_error(&rejected("UPDATE_ALREADY_RUNNING")),
            "update_busy: another update is already active"
        );
        let error = handoff_rejection_error(&rejected("UI_INITIALIZATION_FAILED"));
        assert!(error.contains("update_handoff_rejected"));
        assert!(error.contains("UI_INITIALIZATION_FAILED"));
        assert!(!error.starts_with("update_busy"));
    }

    #[test]
    fn secure_run_root_rejects_redirected_state_directories() {
        let state_root = temp_path("state");
        let outside = temp_path("outside");
        fs::create_dir_all(&outside).expect("outside");

        #[cfg(windows)]
        {
            use std::os::windows::fs::symlink_dir;
            if symlink_dir(&outside, &state_root).is_err() {
                let _ = fs::remove_dir_all(&outside);
                return;
            }
            let error = allocate_run_root(&state_root).expect_err("state symlink");
            assert!(error.contains("must not be a symlink or reparse point"));
            let _ = fs::remove_dir(&state_root);
        }

        fs::create_dir_all(&state_root).expect("state root");
        let runs_target = temp_path("runs-target");
        fs::create_dir_all(&runs_target).expect("runs target");
        #[cfg(windows)]
        {
            use std::os::windows::fs::symlink_dir;
            let runs = state_root.join("update-runs");
            if symlink_dir(&runs_target, &runs).is_err() {
                let _ = fs::remove_dir_all(&state_root);
                let _ = fs::remove_dir_all(&outside);
                let _ = fs::remove_dir_all(&runs_target);
                return;
            }
            let error = allocate_run_root(&state_root).expect_err("runs symlink");
            assert!(error.contains("must not be a symlink or reparse point"));
            let _ = fs::remove_dir(&runs);
        }
        let _ = fs::remove_dir_all(state_root);
        let _ = fs::remove_dir_all(outside);
        let _ = fs::remove_dir_all(runs_target);
    }

    #[test]
    fn install_preflight_rejects_unwritable_root_and_reports_cleanup_failure() {
        let missing = temp_path("missing");
        let error = preflight_install_root_writable(&missing).expect_err("missing root");
        assert!(error.contains("install root is not writable"));

        let root = temp_path("cleanup");
        fs::create_dir_all(&root).expect("root");
        let error = preflight_install_root_writable_with_cleanup(&root, ".sentinel", |_| {
            Err("injected cleanup failure".into())
        })
        .expect_err("cleanup failure");
        assert!(error.contains("preflight cleanup failed"));
        let _ = fs::remove_dir_all(root);
    }
}
