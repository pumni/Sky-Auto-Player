//! Rust-owned Tauri updater policy.
//!
//! React receives only the bounded DTOs below. Endpoint selection, updater
//! configuration, signature verification, artifact handling, and install
//! execution stay in this module and in the official Tauri updater plugin.
//! Production metadata endpoints are fixed Rust-owned v4 endpoints. The
//! updater trust root is compiled into this boundary; a missing or invalid
//! root makes the official updater fail closed.

use crate::app_state::{ActivityCoordinator, ActivityReservationError};
use crate::commands::UpdateInstallAckDto;
use crate::ui_events::{
    UiEvent, UpdateChannel, UpdateCheckAckDto, UpdateCheckDisposition, UpdateCheckOrigin,
    UpdateCheckRequest, UpdateErrorCode, UpdateProgressDto, UpdateRetryAction,
    UpdateSnapshotPayload, UpdateState,
};
use sky_app_core::settings::{ApplicationSettings, SettingsService, UpdateChannel as CoreChannel};
use sky_native_adapters::JsonSettingsStore;
#[cfg(feature = "tauri-update-fixture")]
use std::fs;
#[cfg(feature = "tauri-update-fixture")]
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Runtime};
use tauri_plugin_updater::{Update, UpdaterExt};
use url::Url;

const MAX_RELEASE_NOTES: usize = 16 * 1024;
const MAX_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
#[cfg(not(feature = "tauri-update-fixture"))]
const V4_STABLE_METADATA_ENDPOINT: &str = "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json";
#[cfg(all(test, not(feature = "tauri-update-fixture")))]
const V4_BETA_METADATA_ENDPOINT: &str = "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/beta/latest.json";
#[cfg(not(feature = "tauri-update-fixture"))]
const OFFICIAL_METADATA_HOST: &str = "raw.githubusercontent.com";
#[cfg(not(feature = "tauri-update-fixture"))]
const OFFICIAL_METADATA_OWNER: &str = "pumni";
#[cfg(not(feature = "tauri-update-fixture"))]
const OFFICIAL_METADATA_REPOSITORY: &str = "Sky-Auto-Player";
#[cfg(not(feature = "tauri-update-fixture"))]
const OFFICIAL_METADATA_REF: &str = "release-metadata";
#[cfg(not(feature = "tauri-update-fixture"))]
const V4_TAURI_UPDATER_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDE5QUFCRDJFNzgzODgxOEMKUldTTWdUaDRMcjJxR2JxeE5kTUx5VlIxS1dhOHRrSTEzY2FMeE8wYldtckM2TjV2KzRwQUNaTEUK";
#[cfg(not(feature = "tauri-update-fixture"))]
const V4_TAURI_UPDATER_PUBLIC_KEYS: &[&str] = &[V4_TAURI_UPDATER_PUBLIC_KEY];

mod progress;
mod state;
use progress::{DownloadProgressAccumulator, verified_artifact_progress};
pub(crate) use state::{NativeUpdateCandidate, NativeUpdateState, StateTransition};
#[cfg(test)]
#[path = "native_update/tests.rs"]
mod tests;
#[cfg(feature = "tauri-update-fixture")]
const FIXTURE_NEW_ONLY_ARG: &str = "--selftest-update-fixture-new-only";
#[cfg(feature = "tauri-update-fixture")]
const FIXTURE_PORT_ARG: &str = "--selftest-update-fixture-port";
#[cfg(feature = "tauri-update-fixture")]
const FIXTURE_PUBLIC_KEY_ARG: &str = "--selftest-update-fixture-public-key";

type SafetyHook = Arc<dyn Fn() + Send + Sync + 'static>;

/// The only updater object owned by the desktop application. No caller
/// supplied endpoint, public key, artifact path, or version comparator enters
/// this boundary.
pub(crate) struct UpdateService<R: Runtime> {
    app: AppHandle<R>,
    activity: ActivityCoordinator,
    pub(crate) state: Mutex<NativeUpdateState>,
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

    pub(crate) fn reset_and_publish_idle(
        &self,
        channel: UpdateChannel,
        publish: impl Fn(UiEvent) -> Result<(), String>,
    ) -> Result<UpdateSnapshotPayload, String> {
        self.transition_and_publish(
            StateTransition {
                state: UpdateState::Idle,
                channel,
                ..Default::default()
            },
            &publish,
        )
    }

    pub(crate) fn current_snapshot(&self) -> UpdateSnapshotPayload {
        let state_guard = self.state.lock().expect("native update state lock");
        state_guard.snapshot()
    }

    fn transition_and_publish(
        &self,
        transition: StateTransition,
        publish: &impl Fn(UiEvent) -> Result<(), String>,
    ) -> Result<UpdateSnapshotPayload, String> {
        let (_, snapshot) = {
            let mut state_guard = self
                .state
                .lock()
                .map_err(|_| "native update state lock poisoned".to_string())?;
            state_guard.apply_transition(transition)
        };

        publish(UiEvent::UpdateChanged {
            v: crate::DESKTOP_PROTOCOL_VERSION,
            payload: snapshot.clone(),
        })?;

        Ok(snapshot)
    }

    pub(crate) fn handle_check_persistence_failure(
        &self,
        channel: UpdateChannel,
        error: impl std::fmt::Display,
        publish: &impl Fn(UiEvent) -> Result<(), String>,
    ) -> Result<UpdateCheckAckDto, String> {
        let detail = bounded(format!("update timestamp persistence failed: {error}"));
        self.transition_and_publish(
            StateTransition {
                state: UpdateState::Error,
                channel,
                error_code: Some(UpdateErrorCode::StatePersistenceFailed),
                error_detail: Some(detail),
                retry_action: UpdateRetryAction::Check,
                ..Default::default()
            },
            publish,
        )?;
        Ok(UpdateCheckAckDto {
            disposition: UpdateCheckDisposition::Performed,
        })
    }

    pub(crate) fn check(
        &self,
        request: &UpdateCheckRequest,
        settings: &mut SettingsService<JsonSettingsStore>,
        publish: impl Fn(UiEvent) -> Result<(), String>,
    ) -> Result<UpdateCheckAckDto, String> {
        let channel = public_channel(&settings.snapshot().update.channel);
        let now = unix_timestamp();
        let preferences = &settings.snapshot().update;

        let disposition = check_disposition(request.origin, preferences, now);
        if disposition != UpdateCheckDisposition::Performed {
            return Ok(UpdateCheckAckDto { disposition });
        }

        self.transition_and_publish(
            StateTransition {
                state: UpdateState::Checking,
                channel,
                ..Default::default()
            },
            &publish,
        )?;

        let result = self.check_official(channel);
        let timestamp = unix_timestamp();

        match result {
            Ok(updates)
                if updates.first().is_some_and(|update| {
                    settings.snapshot().update.skip_version != update.version
                }) =>
            {
                if let Err(error) = settings.record_update_success(timestamp) {
                    return self.handle_check_persistence_failure(channel, error, &publish);
                }
                let candidate =
                    candidate_from_update(updates.first().expect("update exists"), channel);
                self.transition_and_publish(
                    StateTransition {
                        state: UpdateState::Available,
                        channel,
                        candidate: Some(candidate),
                        updates,
                        ..Default::default()
                    },
                    &publish,
                )?;
                Ok(UpdateCheckAckDto {
                    disposition: UpdateCheckDisposition::Performed,
                })
            }
            Ok(_) => {
                if let Err(error) = settings.record_update_success(timestamp) {
                    return self.handle_check_persistence_failure(channel, error, &publish);
                }
                self.transition_and_publish(
                    StateTransition {
                        state: UpdateState::Current,
                        channel,
                        ..Default::default()
                    },
                    &publish,
                )?;
                Ok(UpdateCheckAckDto {
                    disposition: UpdateCheckDisposition::Performed,
                })
            }
            Err(error) => {
                if let Err(persist_error) = settings.record_update_error(timestamp) {
                    return self.handle_check_persistence_failure(channel, persist_error, &publish);
                }
                let error_code = classify_check_error(&error);
                let message = bounded(error);
                self.transition_and_publish(
                    StateTransition {
                        state: UpdateState::Error,
                        channel,
                        error_code: Some(error_code),
                        error_detail: Some(message),
                        retry_action: UpdateRetryAction::Check,
                        ..Default::default()
                    },
                    &publish,
                )?;
                Ok(UpdateCheckAckDto {
                    disposition: UpdateCheckDisposition::Performed,
                })
            }
        }
    }

    pub(crate) fn install(
        &self,
        settings: &ApplicationSettings,
        requested_target: &str,
        publish: impl Fn(UiEvent) -> Result<(), String>,
    ) -> Result<UpdateInstallAckDto, String> {
        let (candidate, updates) = {
            let state = self
                .state
                .lock()
                .map_err(|_| "native update state lock poisoned".to_string())?;
            let candidate = match &state.candidate {
                Some(candidate) => candidate.clone(),
                None => {
                    drop(state);
                    self.transition_and_publish(
                        StateTransition {
                            state: UpdateState::Error,
                            channel: public_channel(&settings.update.channel),
                            error_code: Some(UpdateErrorCode::UpdateUnavailable),
                            error_detail: Some(
                                "update_unavailable: check for an update first".to_string(),
                            ),
                            retry_action: UpdateRetryAction::Check,
                            ..Default::default()
                        },
                        &publish,
                    )?;
                    return Ok(UpdateInstallAckDto { accepted: false });
                }
            };
            let updates = state.updates.clone();
            if updates.is_empty() {
                drop(state);
                self.transition_and_publish(
                    StateTransition {
                        state: UpdateState::Error,
                        channel: public_channel(&settings.update.channel),
                        error_code: Some(UpdateErrorCode::UpdateUnavailable),
                        error_detail: Some(
                            "update_unavailable: update metadata is unavailable".to_string(),
                        ),
                        retry_action: UpdateRetryAction::Check,
                        ..Default::default()
                    },
                    &publish,
                )?;
                return Ok(UpdateInstallAckDto { accepted: false });
            }
            (candidate, updates)
        };

        if candidate.version != requested_target
            || settings.update.skip_version == candidate.version
            || settings.update.channel != core_channel(candidate.channel)
        {
            self.transition_and_publish(
                StateTransition {
                    state: UpdateState::Error,
                    channel: candidate.channel,
                    error_code: Some(UpdateErrorCode::StaleUpdate),
                    error_detail: Some("stale_update: update metadata is stale".to_string()),
                    retry_action: UpdateRetryAction::Check,
                    ..Default::default()
                },
                &publish,
            )?;
            return Ok(UpdateInstallAckDto { accepted: false });
        }

        let reservation = match self.activity.reserve_update() {
            Ok(lease) => lease,
            Err(error) => {
                let (code, action) = match error {
                    ActivityReservationError::Closing => {
                        (UpdateErrorCode::Closing, UpdateRetryAction::None)
                    }
                    ActivityReservationError::PhysicalPlaybackActive => {
                        (UpdateErrorCode::PlaybackActive, UpdateRetryAction::Install)
                    }
                    ActivityReservationError::PhysicalReleaseUnverified => {
                        (UpdateErrorCode::PlaybackActive, UpdateRetryAction::None)
                    }
                    ActivityReservationError::UpdateAlreadyActive => {
                        (UpdateErrorCode::UpdateBusy, UpdateRetryAction::Install)
                    }
                };
                let message = update_activity_error(error);
                self.transition_and_publish(
                    StateTransition {
                        state: UpdateState::Error,
                        channel: candidate.channel,
                        candidate: Some(candidate.clone()),
                        updates,
                        error_code: Some(code),
                        error_detail: Some(message),
                        retry_action: action,
                        ..Default::default()
                    },
                    &publish,
                )?;
                return Ok(UpdateInstallAckDto { accepted: false });
            }
        };

        let operation_id = opaque_id()?;
        self.transition_and_publish(
            StateTransition {
                state: UpdateState::Downloading,
                channel: candidate.channel,
                candidate: Some(candidate.clone()),
                updates: updates.clone(),
                operation_id: Some(operation_id.clone()),
                progress: Some(UpdateProgressDto {
                    completed: 0,
                    total: None,
                    message: "Downloading update".to_string(),
                }),
                ..Default::default()
            },
            &publish,
        )?;

        let candidate_for_download = candidate.clone();
        let operation_for_download = operation_id.clone();
        let updates_for_download = updates.clone();
        let download = first_verified_download(updates.clone(), |update| {
            let progress_accumulator = Arc::new(Mutex::new(DownloadProgressAccumulator::default()));
            let progress_error = Arc::new(Mutex::new(None::<String>));
            let callback_accumulator = Arc::clone(&progress_accumulator);
            let callback_error = Arc::clone(&progress_error);
            tauri::async_runtime::block_on(update.download(
                {
                    let publish = &publish;
                    let candidate = candidate_for_download.clone();
                    let operation_id = operation_for_download.clone();
                    let updates = updates_for_download.clone();
                    move |chunk_length, total| {
                        let already_failed = callback_error
                            .lock()
                            .map(|error| error.is_some())
                            .unwrap_or(true);
                        if already_failed {
                            return;
                        }
                        let progress = callback_accumulator
                            .lock()
                            .map_err(|_| "native update progress state lock poisoned".to_string())
                            .and_then(|mut accumulator| {
                                accumulator.record_chunk(chunk_length, total)
                            });
                        match progress {
                            Ok(progress) => {
                                let _ = self.transition_and_publish(
                                    StateTransition {
                                        state: UpdateState::Downloading,
                                        channel: candidate.channel,
                                        candidate: Some(candidate.clone()),
                                        updates: updates.clone(),
                                        operation_id: Some(operation_id.clone()),
                                        progress: Some(progress),
                                        ..Default::default()
                                    },
                                    publish,
                                );
                            }
                            Err(error) => {
                                if let Ok(mut progress_error) = callback_error.lock() {
                                    *progress_error = Some(error);
                                }
                            }
                        }
                    }
                },
                || {},
            ))
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                let progress_error = progress_error
                    .lock()
                    .map_err(|_| "native update progress state lock poisoned".to_string())?
                    .clone();
                progress_error.map_or(Ok(bytes), Err)
            })
        });

        let (update, bytes) = match download {
            Ok((update, bytes)) if (bytes.len() as u64) <= MAX_ARTIFACT_BYTES => (update, bytes),
            Ok(_) => {
                let detail = "update artifact exceeds the bounded size".to_string();
                self.transition_and_publish(
                    StateTransition {
                        state: UpdateState::Error,
                        channel: candidate.channel,
                        candidate: Some(candidate.clone()),
                        updates,
                        operation_id: Some(operation_id),
                        error_code: Some(UpdateErrorCode::DownloadFailed),
                        error_detail: Some(detail),
                        retry_action: UpdateRetryAction::Install,
                        ..Default::default()
                    },
                    &publish,
                )?;
                return Ok(UpdateInstallAckDto { accepted: false });
            }
            Err(error) => {
                let detail = format!("update download failed: {error}");
                self.transition_and_publish(
                    StateTransition {
                        state: UpdateState::Error,
                        channel: candidate.channel,
                        candidate: Some(candidate.clone()),
                        updates,
                        operation_id: Some(operation_id),
                        error_code: Some(UpdateErrorCode::DownloadFailed),
                        error_detail: Some(detail),
                        retry_action: UpdateRetryAction::Install,
                        ..Default::default()
                    },
                    &publish,
                )?;
                return Ok(UpdateInstallAckDto { accepted: false });
            }
        };

        let ready_progress = verified_artifact_progress(&bytes, "Update is ready to install");
        self.transition_and_publish(
            StateTransition {
                state: UpdateState::Ready,
                channel: candidate.channel,
                candidate: Some(candidate.clone()),
                updates: updates.clone(),
                operation_id: Some(operation_id.clone()),
                progress: Some(ready_progress),
                ..Default::default()
            },
            &publish,
        )?;

        let installing_progress =
            verified_artifact_progress(&bytes, "Installing update and restarting");
        self.transition_and_publish(
            StateTransition {
                state: UpdateState::Installing,
                channel: candidate.channel,
                candidate: Some(candidate.clone()),
                updates: updates.clone(),
                operation_id: Some(operation_id.clone()),
                progress: Some(installing_progress),
                ..Default::default()
            },
            &publish,
        )?;

        // `Update::install` is the official Tauri transaction. On Windows it
        // launches the signed NSIS installer and exits this process; its
        // on_before_exit hook runs the safety hook above first.
        if let Err(error) = update.install(bytes) {
            let detail = format!("update install failed: {error}");
            self.transition_and_publish(
                StateTransition {
                    state: UpdateState::Error,
                    channel: candidate.channel,
                    candidate: Some(candidate.clone()),
                    updates,
                    operation_id: Some(operation_id),
                    error_code: Some(UpdateErrorCode::InstallFailed),
                    error_detail: Some(detail),
                    retry_action: UpdateRetryAction::Install,
                    ..Default::default()
                },
                &publish,
            )?;
            return Ok(UpdateInstallAckDto { accepted: false });
        }

        #[cfg(not(windows))]
        self.app.request_restart();
        drop(reservation);
        Ok(UpdateInstallAckDto { accepted: true })
    }

    fn check_official(&self, channel: UpdateChannel) -> Result<Vec<Update>, String> {
        let endpoint = metadata_endpoint(channel)?;
        let mut updates = Vec::new();
        let mut last_error = None;
        for public_key in updater_public_keys()? {
            match self.check_official_with_key(endpoint.clone(), public_key.as_deref()) {
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
        let builder = {
            // The fixture qualifies the official installer transaction itself
            // and launches the installed candidate from the harness. Disabling
            // the plugin's automatic restart keeps the qualification bounded
            // on headless Windows runners while production retains restart.
            builder.restart_after_install(false)
        };
        #[cfg(feature = "tauri-update-fixture")]
        let builder = builder.installer_args(["/S", "/NS"]);
        #[cfg(feature = "tauri-update-fixture")]
        let builder = builder.no_proxy();
        tauri::async_runtime::block_on(builder.build().map_err(|error| error.to_string())?.check())
            .map_err(|error| format!("update check failed: {error}"))
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

fn updater_public_keys() -> Result<Vec<Option<String>>, String> {
    #[cfg(feature = "tauri-update-fixture")]
    {
        let runtime = fixture_runtime_config()?;
        Ok(fixture_public_keys(&runtime.public_keys, runtime.new_only))
    }
    #[cfg(not(feature = "tauri-update-fixture"))]
    {
        Ok(V4_TAURI_UPDATER_PUBLIC_KEYS
            .iter()
            .map(|key| Some((*key).to_owned()))
            .collect())
    }
}

#[cfg(feature = "tauri-update-fixture")]
#[derive(Debug, PartialEq, Eq)]
struct FixtureRuntimeConfig {
    port: u16,
    public_keys: Vec<String>,
    new_only: bool,
}

#[cfg(feature = "tauri-update-fixture")]
fn fixture_runtime_config() -> Result<FixtureRuntimeConfig, String> {
    fixture_runtime_config_from_args(std::env::args().skip(1))
}

#[cfg(feature = "tauri-update-fixture")]
fn fixture_runtime_config_from_args(
    args: impl IntoIterator<Item = String>,
) -> Result<FixtureRuntimeConfig, String> {
    let mut port = None;
    let mut public_key_paths = Vec::new();
    let mut new_only = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            FIXTURE_NEW_ONLY_ARG => new_only = true,
            FIXTURE_PORT_ARG => {
                if port.is_some() {
                    return Err("fixture runtime port was supplied more than once".into());
                }
                let value = args.next().ok_or("fixture runtime port value is missing")?;
                let parsed = value
                    .parse::<u16>()
                    .map_err(|_| "fixture runtime port must be numeric".to_string())?;
                if parsed == 0 {
                    return Err("fixture runtime port must be between 1 and 65535".into());
                }
                port = Some(parsed);
            }
            FIXTURE_PUBLIC_KEY_ARG => {
                let value = args
                    .next()
                    .ok_or("fixture runtime public-key path is missing")?;
                public_key_paths.push(PathBuf::from(value));
                if public_key_paths.len() > 4 {
                    return Err("fixture runtime supplied too many public-key paths".into());
                }
            }
            _ => {}
        }
    }
    let port = port.ok_or("fixture runtime loopback port is missing")?;
    if public_key_paths.is_empty() {
        return Err("fixture runtime public-key paths are missing".into());
    }
    let public_keys = public_key_paths
        .iter()
        .map(|path| read_fixture_public_key(path))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(FixtureRuntimeConfig {
        port,
        public_keys,
        new_only,
    })
}

#[cfg(feature = "tauri-update-fixture")]
fn read_fixture_public_key(path: &Path) -> Result<String, String> {
    if path.as_os_str().len() > 260 {
        return Err("fixture public-key path is unbounded".into());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("fixture public-key path is unavailable: {error}"))?;
    if !metadata.file_type().is_file() || metadata.len() == 0 || metadata.len() > 4096 {
        return Err("fixture public-key path must be a bounded regular file".into());
    }
    let value = fs::read_to_string(path)
        .map_err(|error| format!("fixture public-key file is not valid UTF-8: {error}"))?;
    let value = value.trim().to_owned();
    let upper = value.to_ascii_uppercase();
    if value.is_empty()
        || value.contains('\0')
        || upper.contains("PRIVATE KEY")
        || upper.contains("SECRET KEY")
    {
        return Err("fixture public-key file contains missing or private-key material".into());
    }
    Ok(value)
}

#[cfg(feature = "tauri-update-fixture")]
fn fixture_public_keys(keys: &[String], new_only: bool) -> Vec<Option<String>> {
    if new_only {
        return keys
            .last()
            .cloned()
            .map(|key| vec![Some(key)])
            .unwrap_or_default();
    }
    keys.iter().cloned().map(Some).collect()
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
    #[cfg(feature = "tauri-update-fixture")]
    {
        let runtime = fixture_runtime_config()?;
        let path = match channel {
            UpdateChannel::Stable => "stable",
            UpdateChannel::Beta => "beta",
        };
        Url::parse(&format!("http://127.0.0.1:{}/{}", runtime.port, path))
            .map_err(|error| format!("fixture metadata URL invalid: {error}"))
    }

    #[cfg(not(feature = "tauri-update-fixture"))]
    {
        match channel {
            UpdateChannel::Stable => {
                let endpoint = Url::parse(V4_STABLE_METADATA_ENDPOINT)
                    .map_err(|error| format!("v4 metadata URL invalid: {error}"))?;
                validate_official_metadata_endpoint(&endpoint, channel)?;
                Ok(endpoint)
            }
            UpdateChannel::Beta => Err(
                "channel_unavailable: beta update channel is not supported in production".into(),
            ),
        }
    }
}

#[cfg(not(feature = "tauri-update-fixture"))]
fn validate_official_metadata_endpoint(
    endpoint: &Url,
    channel: UpdateChannel,
) -> Result<(), String> {
    if channel != UpdateChannel::Stable {
        return Err(
            "channel_unavailable: beta update channel is not supported in production".into(),
        );
    }
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

    let expected_segments = [
        OFFICIAL_METADATA_OWNER,
        OFFICIAL_METADATA_REPOSITORY,
        OFFICIAL_METADATA_REF,
        "channels",
        "stable",
        "latest.json",
    ];
    let actual_segments = endpoint
        .path_segments()
        .ok_or_else(|| "v4 metadata URL path is not hierarchical".to_string())?
        .collect::<Vec<_>>();
    if actual_segments != expected_segments
        || endpoint.path()
            != format!(
                "/{}/{}/{}/channels/stable/latest.json",
                OFFICIAL_METADATA_OWNER, OFFICIAL_METADATA_REPOSITORY, OFFICIAL_METADATA_REF
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

fn classify_check_error(error: &str) -> UpdateErrorCode {
    let lower = error.to_ascii_lowercase();
    if lower.contains("channel_unavailable") {
        UpdateErrorCode::ChannelUnavailable
    } else {
        UpdateErrorCode::CheckFailed
    }
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
        ActivityReservationError::PhysicalReleaseUnverified => {
            "playback_release_unverified: update installation is blocked because the previous physical key release could not be verified; restart the application to restore physical input".into()
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

pub(crate) fn check_disposition(
    origin: UpdateCheckOrigin,
    preferences: &sky_app_core::settings::UpdatePreferences,
    _now: i64,
) -> UpdateCheckDisposition {
    // The desktop sends one background check after bootstrap on each app
    // launch. Never suppress it using a timestamp from a previous launch.
    // Explicit manual checks remain available even with Auto Check disabled.
    if origin == UpdateCheckOrigin::Background
        && !sky_app_core::update::should_check_on_launch(preferences)
    {
        return UpdateCheckDisposition::Disabled;
    }
    UpdateCheckDisposition::Performed
}
