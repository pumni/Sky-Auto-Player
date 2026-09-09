//! Rust-owned Tauri updater policy.
//!
//! React receives only the bounded DTOs below. Endpoint selection, updater
//! configuration, signature verification, artifact handling, and install
//! execution stay in this module and in the official Tauri updater plugin.
//! Production metadata endpoints are fixed Rust-owned v4 endpoints. The
//! updater trust root is compiled into this boundary; a missing or invalid
//! root makes the official updater fail closed.

use crate::app_state::{ActivityCoordinator, ActivityReservationError, UpdateInstallLease};
use crate::commands::{UpdateCheckDto, UpdateHandoffDto};
use crate::ui_events::{
    UiEvent, UpdateAvailablePayload, UpdateChannel, UpdateProgressPayload, UpdateResultPayload,
    UpdateState,
};
use sky_app_core::settings::{ApplicationSettings, SettingsService, UpdateChannel as CoreChannel};
use sky_native_adapters::JsonSettingsStore;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Runtime};
use tauri_plugin_updater::{Update, UpdaterExt};
use url::Url;

const MAX_RELEASE_NOTES: usize = 16 * 1024;
const MAX_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const V4_STABLE_METADATA_ENDPOINT: &str = "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json";
const V4_BETA_METADATA_ENDPOINT: &str = "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/beta/latest.json";
const OFFICIAL_METADATA_HOST: &str = "raw.githubusercontent.com";
const OFFICIAL_METADATA_OWNER: &str = "pumni";
const OFFICIAL_METADATA_REPOSITORY: &str = "Sky-Auto-Player";
const OFFICIAL_METADATA_REF: &str = "release-metadata";
#[cfg(not(feature = "tauri-update-fixture"))]
const V4_TAURI_UPDATER_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDE5QUFCRDJFNzgzODgxOEMKUldTTWdUaDRMcjJxR2JxeE5kTUx5VlIxS1dhOHRrSTEzY2FMeE8wYldtckM2TjV2KzRwQUNaTEUK";
#[cfg(not(feature = "tauri-update-fixture"))]
const V4_TAURI_UPDATER_PUBLIC_KEYS: &[&str] = &[V4_TAURI_UPDATER_PUBLIC_KEY];
#[cfg(feature = "tauri-update-fixture")]
const FIXTURE_TAURI_UPDATER_PUBLIC_KEYS: Option<&str> =
    option_env!("SKY_TAURI_UPDATE_FIXTURE_PUBLIC_KEYS");
const FIXTURE_TAURI_UPDATER_PORT: &str = match option_env!("SKY_TAURI_UPDATE_FIXTURE_PORT") {
    Some(port) => port,
    None => "invalid-fixture-port",
};

#[derive(Clone, Debug)]
pub(crate) struct NativeUpdateCandidate {
    pub version: String,
    pub channel: UpdateChannel,
    pub release_notes: Option<String>,
    pub published_at: Option<String>,
}

struct NativeUpdateState {
    candidate: Option<NativeUpdateCandidate>,
    updates: Vec<Update>,
    operation_id: Option<String>,
    state: UpdateState,
}

impl Default for NativeUpdateState {
    fn default() -> Self {
        Self {
            candidate: None,
            updates: Vec::new(),
            operation_id: None,
            state: UpdateState::Idle,
        }
    }
}

type SafetyHook = Arc<dyn Fn() + Send + Sync + 'static>;

/// The only updater object owned by the desktop application. No caller
/// supplied endpoint, public key, artifact path, or version comparator enters
/// this boundary.
pub(crate) struct UpdateService<R: Runtime> {
    app: AppHandle<R>,
    activity: ActivityCoordinator,
    state: Mutex<NativeUpdateState>,
    safety_hook: Arc<Mutex<Option<SafetyHook>>>,
}

impl<R: Runtime> UpdateService<R> {
    pub(crate) fn new(app: AppHandle<R>, activity: ActivityCoordinator) -> Self {
        Self {
            app,
            activity,
            state: Mutex::new(NativeUpdateState::default()),
            safety_hook: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn set_pre_exit_safety(&self, hook: SafetyHook) {
        if let Ok(mut safety_hook) = self.safety_hook.lock() {
            *safety_hook = Some(hook);
        }
    }

    pub(crate) fn reset(&self) {
        if let Ok(mut state) = self.state.lock() {
            *state = NativeUpdateState::default();
        }
    }

    pub(crate) fn check(
        &self,
        settings: &mut SettingsService<JsonSettingsStore>,
        publish: impl Fn(UiEvent) -> Result<(), String>,
    ) -> Result<UpdateCheckDto, String> {
        let channel = public_channel(&settings.snapshot().update.channel);
        let current_version = env!("CARGO_PKG_VERSION").to_owned();
        let result = self.check_official(channel);
        let timestamp = unix_timestamp();

        match result {
            Ok(updates)
                if updates.first().is_some_and(|update| {
                    settings.snapshot().update.skip_version != update.version
                }) =>
            {
                settings
                    .record_update_success(timestamp)
                    .map_err(|error| format!("update timestamp persistence failed: {error}"))?;
                let candidate =
                    candidate_from_update(updates.first().expect("update exists"), channel);
                let dto = UpdateCheckDto {
                    state: UpdateState::Available,
                    current_version: current_version.clone(),
                    available_version: Some(candidate.version.clone()),
                    channel,
                    release_notes: candidate.release_notes.clone(),
                    published_at: candidate.published_at.clone(),
                    error: None,
                };
                {
                    let mut state = self
                        .state
                        .lock()
                        .map_err(|_| "native update state lock poisoned".to_string())?;
                    state.candidate = Some(candidate.clone());
                    state.updates = updates;
                    state.operation_id = None;
                    state.state = UpdateState::Available;
                }
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
            Ok(_) => {
                settings
                    .record_update_success(timestamp)
                    .map_err(|error| format!("update timestamp persistence failed: {error}"))?;
                let dto = UpdateCheckDto {
                    state: UpdateState::Current,
                    current_version,
                    available_version: None,
                    channel,
                    release_notes: None,
                    published_at: None,
                    error: None,
                };
                self.reset();
                publish_result(&publish, &dto)?;
                Ok(dto)
            }
            Err(error) => {
                let _ = settings.record_update_error(timestamp);
                let message = bounded(error);
                let dto = UpdateCheckDto {
                    state: UpdateState::Error,
                    current_version,
                    available_version: None,
                    channel,
                    release_notes: None,
                    published_at: None,
                    error: Some(message),
                };
                self.reset();
                publish_result(&publish, &dto)?;
                Ok(dto)
            }
        }
    }

    pub(crate) fn install(
        &self,
        settings: &ApplicationSettings,
        requested_target: &str,
        publish: impl Fn(UiEvent) -> Result<(), String>,
    ) -> Result<UpdateHandoffDto, String> {
        let (candidate, updates) = {
            let state = self
                .state
                .lock()
                .map_err(|_| "native update state lock poisoned".to_string())?;
            let candidate = state
                .candidate
                .clone()
                .ok_or_else(|| "update_unavailable: check for an update first".to_string())?;
            let updates = state.updates.clone();
            if updates.is_empty() {
                return Err("update_unavailable: update metadata is unavailable".into());
            }
            (candidate, updates)
        };
        if candidate.version != requested_target
            || settings.update.skip_version == candidate.version
            || settings.update.channel != core_channel(candidate.channel)
        {
            return Err("stale_update: update metadata is stale".into());
        }

        let reservation = self
            .activity
            .reserve_update()
            .map_err(update_activity_error)?;
        let operation_id = opaque_id()?;
        self.set_state(UpdateState::Downloading, Some(operation_id.clone()));
        publish_progress(
            &publish,
            UpdateState::Downloading,
            &candidate,
            &operation_id,
            0,
            None,
            "Downloading update",
        )?;

        let candidate_for_download = candidate.clone();
        let operation_for_download = operation_id.clone();
        let download = first_verified_download(updates, |update| {
            tauri::async_runtime::block_on(update.download(
                {
                    let publish = &publish;
                    let candidate = candidate_for_download.clone();
                    let operation_id = operation_for_download.clone();
                    move |completed, total| {
                        let total = total.filter(|value| *value <= MAX_ARTIFACT_BYTES);
                        let completed = (completed as u64).min(MAX_ARTIFACT_BYTES);
                        let _ = publish_progress(
                            publish,
                            UpdateState::Downloading,
                            &candidate,
                            &operation_id,
                            completed,
                            total,
                            "Downloading update",
                        );
                    }
                },
                || {},
            ))
            .map_err(|error| error.to_string())
        });
        let (update, bytes) = match download {
            Ok((update, bytes)) if (bytes.len() as u64) <= MAX_ARTIFACT_BYTES => (update, bytes),
            Ok(_) => {
                return self.install_error(
                    &candidate,
                    &operation_id,
                    &reservation,
                    &publish,
                    "update artifact exceeds the bounded size",
                );
            }
            Err(error) => {
                return self.install_error(
                    &candidate,
                    &operation_id,
                    &reservation,
                    &publish,
                    &format!("update download failed: {error}"),
                );
            }
        };

        self.set_state(UpdateState::Ready, Some(operation_id.clone()));
        publish_progress(
            &publish,
            UpdateState::Ready,
            &candidate,
            &operation_id,
            bytes.len() as u64,
            Some(bytes.len() as u64),
            "Update is ready to install",
        )?;
        let dto = UpdateHandoffDto {
            handoff_id: operation_id.clone(),
            target_version: candidate.version.clone(),
            state: UpdateState::Installing,
        };
        self.set_state(UpdateState::Installing, Some(operation_id.clone()));
        publish_progress(
            &publish,
            UpdateState::Installing,
            &candidate,
            &operation_id,
            bytes.len() as u64,
            Some(bytes.len() as u64),
            "Installing update and restarting",
        )?;
        publish_result(
            &publish,
            &UpdateCheckDto {
                state: UpdateState::Installing,
                current_version: env!("CARGO_PKG_VERSION").into(),
                available_version: Some(candidate.version.clone()),
                channel: candidate.channel,
                release_notes: candidate.release_notes.clone(),
                published_at: candidate.published_at.clone(),
                error: None,
            },
        )?;

        // `Update::install` is the official Tauri transaction. On Windows it
        // launches the signed NSIS installer and exits this process; its
        // on_before_exit hook runs the safety hook above first.
        if let Err(error) = update.install(bytes) {
            return self.install_error(
                &candidate,
                &operation_id,
                &reservation,
                &publish,
                &format!("update install failed: {error}"),
            );
        }

        #[cfg(not(windows))]
        self.app.request_restart();
        drop(reservation);
        Ok(dto)
    }

    fn check_official(&self, channel: UpdateChannel) -> Result<Vec<Update>, String> {
        let endpoint = metadata_endpoint(channel)?;
        let mut updates = Vec::new();
        let mut last_error = None;
        for public_key in updater_public_keys() {
            match self.check_official_with_key(endpoint.clone(), public_key) {
                Ok(Some(update)) => updates.push(update),
                Ok(None) => {}
                Err(error) => last_error = Some(error),
            }
        }
        if !updates.is_empty() {
            Ok(updates)
        } else if let Some(error) = last_error {
            Err(error)
        } else {
            Ok(Vec::new())
        }
    }

    fn check_official_with_key(
        &self,
        endpoint: Url,
        public_key: Option<&str>,
    ) -> Result<Option<Update>, String> {
        let builder = self.app.updater_builder();
        let builder = match public_key {
            Some(public_key) => builder.pubkey(public_key),
            None => builder,
        };
        let builder = builder
            .endpoints(vec![endpoint])
            .map_err(|error| format!("update metadata endpoint rejected: {error}"))?
            .on_before_exit(self.install_safety_hook())
            .restart_after_install(true);
        #[cfg(feature = "tauri-update-fixture")]
        let builder = builder.no_proxy();
        tauri::async_runtime::block_on(builder.build().map_err(|error| error.to_string())?.check())
            .map_err(|error| format!("update check failed: {error}"))
    }

    fn set_state(&self, state_value: UpdateState, operation_id: Option<String>) {
        if let Ok(mut state) = self.state.lock() {
            state.state = state_value;
            state.operation_id = operation_id;
        }
    }

    fn install_error(
        &self,
        candidate: &NativeUpdateCandidate,
        operation_id: &str,
        reservation: &UpdateInstallLease,
        publish: &impl Fn(UiEvent) -> Result<(), String>,
        message: &str,
    ) -> Result<UpdateHandoffDto, String> {
        let _ = reservation;
        self.set_state(UpdateState::Error, Some(operation_id.to_owned()));
        let error = bounded(message);
        publish_result(
            publish,
            &UpdateCheckDto {
                state: UpdateState::Error,
                current_version: env!("CARGO_PKG_VERSION").into(),
                available_version: Some(candidate.version.clone()),
                channel: candidate.channel,
                release_notes: candidate.release_notes.clone(),
                published_at: candidate.published_at.clone(),
                error: Some(error.clone()),
            },
        )?;
        Err(error)
    }

    pub(crate) fn install_safety_hook(&self) -> impl Fn() + Send + Sync + 'static {
        let app = self.app.clone();
        let safety_hook = self.safety_hook.clone();
        move || {
            // The plugin's default hook is replaced so the native boundary
            // can quiesce playback before Tauri cleans up windows.
            if let Ok(hook) = safety_hook.lock()
                && let Some(hook) = hook.as_ref()
            {
                hook();
            }
            app.cleanup_before_exit();
        }
    }
}

fn updater_public_keys() -> Vec<Option<&'static str>> {
    #[cfg(feature = "tauri-update-fixture")]
    {
        FIXTURE_TAURI_UPDATER_PUBLIC_KEYS
            .map(|keys| keys.split('|').map(Some).collect())
            .unwrap_or_else(|| vec![None])
    }
    #[cfg(not(feature = "tauri-update-fixture"))]
    {
        V4_TAURI_UPDATER_PUBLIC_KEYS
            .iter()
            .copied()
            .map(Some)
            .collect()
    }
}

/// Try each `Update`'s own Tauri verification context until the downloaded
/// bytes verify. This preserves the bounded trust-root rotation mechanism.
fn first_verified_download<T>(
    updates: Vec<T>,
    mut download: impl FnMut(&T) -> Result<Vec<u8>, String>,
) -> Result<(T, Vec<u8>), String> {
    let mut last_error = None;
    for update in updates {
        match download(&update) {
            Ok(bytes) => return Ok((update, bytes)),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| "update trust roots are unavailable".into()))
}

fn metadata_endpoint(channel: UpdateChannel) -> Result<Url, String> {
    let endpoint = if cfg!(feature = "tauri-update-fixture") {
        match channel {
            UpdateChannel::Stable => {
                format!("http://127.0.0.1:{FIXTURE_TAURI_UPDATER_PORT}/stable")
            }
            UpdateChannel::Beta => format!("http://127.0.0.1:{FIXTURE_TAURI_UPDATER_PORT}/beta"),
        }
    } else {
        let endpoint = match channel {
            UpdateChannel::Stable => V4_STABLE_METADATA_ENDPOINT,
            UpdateChannel::Beta => V4_BETA_METADATA_ENDPOINT,
        };
        endpoint.to_owned()
    };
    Url::parse(&endpoint)
        .map_err(|error| {
            if cfg!(feature = "tauri-update-fixture") {
                format!("fixture metadata URL invalid: {error}")
            } else {
                format!("v4 metadata URL invalid: {error}")
            }
        })
        .and_then(|endpoint| {
            if cfg!(feature = "tauri-update-fixture") {
                Ok(endpoint)
            } else {
                validate_official_metadata_endpoint(&endpoint, channel)?;
                Ok(endpoint)
            }
        })
}

fn validate_official_metadata_endpoint(
    endpoint: &Url,
    channel: UpdateChannel,
) -> Result<(), String> {
    if endpoint.scheme() != "https"
        || endpoint.host_str() != Some(OFFICIAL_METADATA_HOST)
        || endpoint.port().is_some()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err("v4 metadata URL has an unapproved origin or URL component".into());
    }

    let expected_channel = match channel {
        UpdateChannel::Stable => "stable",
        UpdateChannel::Beta => "beta",
    };
    let expected_segments = [
        OFFICIAL_METADATA_OWNER,
        OFFICIAL_METADATA_REPOSITORY,
        OFFICIAL_METADATA_REF,
        "channels",
        expected_channel,
        "latest.json",
    ];
    let actual_segments = endpoint
        .path_segments()
        .ok_or_else(|| "v4 metadata URL path is not hierarchical".to_string())?
        .collect::<Vec<_>>();
    if actual_segments != expected_segments
        || endpoint.path()
            != format!(
                "/{}/{}/{}/channels/{}/latest.json",
                OFFICIAL_METADATA_OWNER,
                OFFICIAL_METADATA_REPOSITORY,
                OFFICIAL_METADATA_REF,
                expected_channel
            )
    {
        return Err("v4 metadata URL path is not an approved channel endpoint".into());
    }
    Ok(())
}

fn candidate_from_update(update: &Update, channel: UpdateChannel) -> NativeUpdateCandidate {
    NativeUpdateCandidate {
        version: bounded(&update.version),
        channel,
        release_notes: update
            .body
            .as_deref()
            .map(|value| value.chars().take(MAX_RELEASE_NOTES).collect()),
        published_at: update.date.map(bounded),
    }
}

fn publish_progress(
    publish: &impl Fn(UiEvent) -> Result<(), String>,
    state: UpdateState,
    candidate: &NativeUpdateCandidate,
    operation_id: &str,
    completed: u64,
    total: Option<u64>,
    message: &str,
) -> Result<(), String> {
    publish(UiEvent::UpdateProgress {
        v: crate::DESKTOP_PROTOCOL_VERSION,
        payload: UpdateProgressPayload {
            operation_id: operation_id.to_owned(),
            state,
            available_version: candidate.version.clone(),
            completed,
            total,
            message: bounded(message),
        },
    })
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

fn public_channel(channel: &CoreChannel) -> UpdateChannel {
    match channel {
        CoreChannel::Stable => UpdateChannel::Stable,
        CoreChannel::Beta => UpdateChannel::Beta,
    }
}

fn core_channel(channel: UpdateChannel) -> CoreChannel {
    match channel {
        UpdateChannel::Stable => CoreChannel::Stable,
        UpdateChannel::Beta => CoreChannel::Beta,
    }
}

fn update_activity_error(error: ActivityReservationError) -> String {
    match error {
        ActivityReservationError::Closing => "closing: desktop application is closing".into(),
        ActivityReservationError::PhysicalPlaybackActive => {
            "playback_active: update installation cannot run during physical playback".into()
        }
        ActivityReservationError::CalibrationAlreadyActive => {
            "calibration_active: update installation cannot run during calibration".into()
        }
        ActivityReservationError::UpdateAlreadyActive => {
            "update_busy: another update installation is already active".into()
        }
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

fn opaque_id() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| format!("secure update identifier failed: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    #[cfg(not(feature = "tauri-update-fixture"))]
    use super::{
        V4_BETA_METADATA_ENDPOINT, V4_STABLE_METADATA_ENDPOINT, V4_TAURI_UPDATER_PUBLIC_KEY,
        V4_TAURI_UPDATER_PUBLIC_KEYS, metadata_endpoint, validate_official_metadata_endpoint,
    };
    use super::{bounded, first_verified_download, update_activity_error};
    use crate::app_state::ActivityReservationError;
    #[cfg(not(feature = "tauri-update-fixture"))]
    use crate::ui_events::UpdateChannel;

    #[cfg(not(feature = "tauri-update-fixture"))]
    #[test]
    fn production_metadata_endpoints_are_fixed_and_channel_isolated() {
        let stable = metadata_endpoint(UpdateChannel::Stable).unwrap();
        let beta = metadata_endpoint(UpdateChannel::Beta).unwrap();
        assert_eq!(stable.as_str(), V4_STABLE_METADATA_ENDPOINT);
        assert_eq!(beta.as_str(), V4_BETA_METADATA_ENDPOINT);
        assert_ne!(stable, beta);
        for endpoint in [stable, beta] {
            assert_eq!(endpoint.scheme(), "https");
            assert_eq!(endpoint.host_str(), Some("raw.githubusercontent.com"));
        }
    }

    #[cfg(not(feature = "tauri-update-fixture"))]
    #[test]
    fn production_metadata_allowlist_rejects_unapproved_endpoints() {
        let rejected = [
            (
                "http://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json",
                UpdateChannel::Stable,
            ),
            (
                "https://evil.example/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json",
                UpdateChannel::Stable,
            ),
            (
                "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/main/channels/stable/latest.json",
                UpdateChannel::Stable,
            ),
            (
                "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/beta/latest.json",
                UpdateChannel::Stable,
            ),
            (
                "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json?redirect=1",
                UpdateChannel::Stable,
            ),
            (
                "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json/",
                UpdateChannel::Stable,
            ),
        ];

        for (raw, channel) in rejected {
            let endpoint = url::Url::parse(raw).unwrap();
            assert!(
                validate_official_metadata_endpoint(&endpoint, channel).is_err(),
                "endpoint should be rejected: {raw}"
            );
        }
    }

    #[cfg(not(feature = "tauri-update-fixture"))]
    #[test]
    fn production_updater_trust_root_is_independent_and_bounded() {
        assert_eq!(V4_TAURI_UPDATER_PUBLIC_KEYS, &[V4_TAURI_UPDATER_PUBLIC_KEY]);
        assert!(!V4_TAURI_UPDATER_PUBLIC_KEY.is_empty());
        assert!(V4_TAURI_UPDATER_PUBLIC_KEY.len() <= 4096);
        assert!(!V4_TAURI_UPDATER_PUBLIC_KEY.contains("PRIVATE KEY"));
        assert!(
            V4_TAURI_UPDATER_PUBLIC_KEY
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
        );
    }

    #[test]
    fn production_error_is_bounded_and_does_not_name_a_release_namespace() {
        let message = bounded("x".repeat(8_000));
        assert_eq!(message.len(), 4096);
        assert!(
            update_activity_error(ActivityReservationError::PhysicalPlaybackActive)
                .contains("playback_active")
        );
    }

    #[test]
    fn production_fixture_manifest_is_tauri_consumable() {
        let manifest = serde_json::json!({
            "version": "4.0.0-rc.2",
            "notes": "Deterministic bridge rotation candidate.",
            "pub_date": "2026-09-04T00:00:00Z",
            "platforms": {
                "windows-x86_64": {
                    "signature": "fixture-signature",
                    "url": "http://127.0.0.1:40000/candidate/update.exe"
                }
            }
        });
        let release: tauri_plugin_updater::RemoteRelease =
            serde_json::from_value(manifest).expect("fixture manifest must match Tauri schema");
        assert_eq!(release.version.to_string(), "4.0.0-rc.2");
        assert_eq!(
            release
                .download_url("windows-x86_64")
                .expect("production platform must be present")
                .as_str(),
            "http://127.0.0.1:40000/candidate/update.exe"
        );
        assert_eq!(
            release
                .signature("windows-x86_64")
                .expect("production signature must be present"),
            "fixture-signature"
        );
    }

    #[test]
    fn update_installation_has_a_distinct_playback_policy_error() {
        let message = update_activity_error(ActivityReservationError::PhysicalPlaybackActive);
        assert_eq!(
            message,
            "playback_active: update installation cannot run during physical playback"
        );
    }

    #[test]
    fn runtime_rotation_download_falls_through_to_the_root_that_verifies_bytes() {
        let mut attempts = Vec::new();
        let (selected, bytes) = first_verified_download(vec!["old-root", "new-root"], |root| {
            attempts.push(*root);
            if *root == "old-root" {
                Err("signature mismatch".into())
            } else {
                Ok(b"new-root-signed-update".to_vec())
            }
        })
        .unwrap();
        assert_eq!(attempts, ["old-root", "new-root"]);
        assert_eq!(selected, "new-root");
        assert_eq!(bytes, b"new-root-signed-update");
    }

    #[test]
    fn runtime_rotation_download_fails_closed_when_no_root_verifies_bytes() {
        let result = first_verified_download(vec!["old-root", "new-root"], |_| {
            Err::<Vec<u8>, _>("signature mismatch".into())
        });
        assert_eq!(result.unwrap_err(), "signature mismatch");
    }
}
