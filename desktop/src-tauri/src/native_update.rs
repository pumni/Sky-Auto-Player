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
#[cfg(not(feature = "tauri-update-fixture"))]
#[allow(dead_code)]
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
#[cfg(feature = "tauri-update-fixture")]
const FIXTURE_NEW_ONLY_ARG: &str = "--selftest-update-fixture-new-only";
#[cfg(feature = "tauri-update-fixture")]
const FIXTURE_PORT_ARG: &str = "--selftest-update-fixture-port";
#[cfg(feature = "tauri-update-fixture")]
const FIXTURE_PUBLIC_KEY_ARG: &str = "--selftest-update-fixture-public-key";

#[cfg(windows)]
fn updater_install_root_arg() -> Result<String, String> {
    let executable = std::env::current_exe().map_err(|error| {
        format!("could not resolve the current executable for updater install root: {error}")
    })?;
    let root = executable
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| "current executable has no install root".to_owned())?;
    let root = root.to_string_lossy();
    if root.contains('"') {
        return Err("current updater install root contains an unsupported quote".to_owned());
    }
    Ok(if root.contains(' ') || root.contains('\t') {
        format!(r#"/D="{root}""#)
    } else {
        format!("/D={root}")
    })
}

#[derive(Clone, Debug)]
pub(crate) struct NativeUpdateCandidate {
    pub version: String,
    pub channel: UpdateChannel,
    pub release_notes: Option<String>,
    pub published_at: Option<String>,
}

pub(crate) struct NativeUpdateState {
    pub(crate) revision: u64,
    pub(crate) channel: UpdateChannel,
    pub(crate) candidate: Option<NativeUpdateCandidate>,
    pub(crate) updates: Vec<Update>,
    pub(crate) operation_id: Option<String>,
    pub(crate) state: UpdateState,
    pub(crate) error_code: Option<UpdateErrorCode>,
    pub(crate) error_detail: Option<String>,
    pub(crate) retry_action: UpdateRetryAction,
    pub(crate) progress: Option<UpdateProgressDto>,
}

impl Default for NativeUpdateState {
    fn default() -> Self {
        Self {
            revision: 0,
            channel: UpdateChannel::Stable,
            candidate: None,
            updates: Vec::new(),
            operation_id: None,
            state: UpdateState::Idle,
            error_code: None,
            error_detail: None,
            retry_action: UpdateRetryAction::None,
            progress: None,
        }
    }
}

impl NativeUpdateState {
    pub(crate) fn apply_transition(
        &mut self,
        transition: StateTransition,
    ) -> (u64, UpdateSnapshotPayload) {
        self.revision += 1;

        let error_detail = transition.error_detail.as_deref().map(bounded);

        self.state = transition.state;
        self.channel = transition.channel;
        self.candidate = transition.candidate.clone();
        self.updates = transition.updates;
        self.operation_id = transition.operation_id.clone();
        self.progress = transition.progress.clone();
        self.error_code = transition.error_code;
        self.error_detail = error_detail.clone();
        self.retry_action = transition.retry_action;

        let (available_version, release_notes, published_at) = match &transition.candidate {
            Some(c) => (
                Some(c.version.clone()),
                c.release_notes.clone(),
                c.published_at.clone(),
            ),
            None => (None, None, None),
        };

        let snapshot = UpdateSnapshotPayload {
            revision: self.revision,
            state: transition.state,
            current_version: env!("CARGO_PKG_VERSION").to_owned(),
            available_version,
            channel: transition.channel,
            release_notes,
            published_at,
            error_code: transition.error_code,
            error_detail,
            retry_action: transition.retry_action,
            operation_id: transition.operation_id,
            progress: transition.progress,
        };

        (self.revision, snapshot)
    }

    pub(crate) fn snapshot(&self) -> UpdateSnapshotPayload {
        let (available_version, release_notes, published_at) = match &self.candidate {
            Some(c) => (
                Some(c.version.clone()),
                c.release_notes.clone(),
                c.published_at.clone(),
            ),
            None => (None, None, None),
        };

        UpdateSnapshotPayload {
            revision: self.revision,
            state: self.state,
            current_version: env!("CARGO_PKG_VERSION").to_owned(),
            available_version,
            channel: self.channel,
            release_notes,
            published_at,
            error_code: self.error_code,
            error_detail: self.error_detail.clone(),
            retry_action: self.retry_action,
            operation_id: self.operation_id.clone(),
            progress: self.progress.clone(),
        }
    }
}

type SafetyHook = Arc<dyn Fn() + Send + Sync + 'static>;

pub(crate) struct StateTransition {
    state: UpdateState,
    channel: UpdateChannel,
    candidate: Option<NativeUpdateCandidate>,
    updates: Vec<Update>,
    operation_id: Option<String>,
    progress: Option<UpdateProgressDto>,
    error_code: Option<UpdateErrorCode>,
    error_detail: Option<String>,
    retry_action: UpdateRetryAction,
}

impl Default for StateTransition {
    fn default() -> Self {
        Self {
            state: UpdateState::Idle,
            channel: UpdateChannel::Stable,
            candidate: None,
            updates: Vec::new(),
            operation_id: None,
            progress: None,
            error_code: None,
            error_detail: None,
            retry_action: UpdateRetryAction::None,
        }
    }
}

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

    #[allow(dead_code)]
    pub(crate) fn reset(&self) {
        if let Ok(mut state) = self.state.lock() {
            let revision = state.revision;
            *state = NativeUpdateState::default();
            state.revision = revision;
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
                    ActivityReservationError::CalibrationAlreadyActive => (
                        UpdateErrorCode::CalibrationActive,
                        UpdateRetryAction::Install,
                    ),
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
        #[cfg(windows)]
        let builder = builder.installer_arg(updater_install_root_arg()?);
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

#[derive(Debug, Default)]
struct DownloadProgressAccumulator {
    downloaded: u64,
}

impl DownloadProgressAccumulator {
    fn record_chunk(
        &mut self,
        chunk_length: usize,
        content_length: Option<u64>,
    ) -> Result<UpdateProgressDto, String> {
        let chunk_length = u64::try_from(chunk_length)
            .map_err(|_| "update download chunk length is not representable".to_string())?;
        let downloaded = self
            .downloaded
            .checked_add(chunk_length)
            .ok_or_else(|| "update download progress overflowed".to_string())?;
        if downloaded > MAX_ARTIFACT_BYTES {
            return Err("update download exceeds the bounded artifact size".to_string());
        }

        let total = content_length.filter(|value| (1..=MAX_ARTIFACT_BYTES).contains(value));
        if total.is_some_and(|total| downloaded > total) {
            return Err("update download progress exceeds its content length".to_string());
        }

        self.downloaded = downloaded;
        Ok(UpdateProgressDto {
            completed: downloaded,
            total,
            message: "Downloading update".to_string(),
        })
    }
}

fn verified_artifact_progress(bytes: &[u8], message: &str) -> UpdateProgressDto {
    let bytes_len = bytes.len() as u64;
    debug_assert!(bytes_len <= MAX_ARTIFACT_BYTES);
    UpdateProgressDto {
        completed: bytes_len,
        total: Some(bytes_len),
        message: message.to_string(),
    }
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
    now: i64,
) -> UpdateCheckDisposition {
    if origin == UpdateCheckOrigin::Background {
        if !preferences.auto_check {
            return UpdateCheckDisposition::Disabled;
        }
        if !sky_app_core::update::should_auto_check(preferences, now) {
            return UpdateCheckDisposition::Throttled;
        }
    }
    UpdateCheckDisposition::Performed
}

#[cfg(test)]
mod tests {
    use super::{
        DownloadProgressAccumulator, MAX_ARTIFACT_BYTES, bounded, first_verified_download,
        update_activity_error, verified_artifact_progress,
    };
    #[cfg(not(feature = "tauri-update-fixture"))]
    use super::{
        V4_BETA_METADATA_ENDPOINT, V4_STABLE_METADATA_ENDPOINT, V4_TAURI_UPDATER_PUBLIC_KEY,
        V4_TAURI_UPDATER_PUBLIC_KEYS, metadata_endpoint, validate_official_metadata_endpoint,
    };
    use crate::app_state::ActivityReservationError;
    #[cfg(not(feature = "tauri-update-fixture"))]
    use crate::ui_events::UpdateChannel;

    #[cfg(feature = "tauri-update-fixture")]
    #[test]
    fn fixture_new_only_mode_selects_only_the_last_supplied_root() {
        let keys = vec!["old-root".to_owned(), "new-root".to_owned()];
        assert_eq!(
            super::fixture_public_keys(&keys, true),
            vec![Some("new-root".to_owned())]
        );
        assert_eq!(
            super::fixture_public_keys(&keys, false),
            vec![Some("old-root".to_owned()), Some("new-root".to_owned())]
        );
    }

    #[cfg(feature = "tauri-update-fixture")]
    #[test]
    fn fixture_runtime_configuration_requires_a_loopback_port_and_public_keys() {
        let missing_port = super::fixture_runtime_config_from_args([
            super::FIXTURE_PUBLIC_KEY_ARG.to_owned(),
            "missing.pub".to_owned(),
        ]);
        assert!(missing_port.is_err());

        let missing_key = super::fixture_runtime_config_from_args([
            super::FIXTURE_PORT_ARG.to_owned(),
            "40000".to_owned(),
        ]);
        assert!(missing_key.is_err());
    }

    #[cfg(feature = "tauri-update-fixture")]
    #[test]
    fn fixture_runtime_configuration_rejects_invalid_port_and_private_key() {
        let root = std::env::temp_dir().join(format!(
            "sky-auto-player-fixture-key-{}",
            std::process::id()
        ));
        std::fs::write(&root, "PRIVATE KEY").expect("write fixture key");
        let invalid_port = super::fixture_runtime_config_from_args([
            super::FIXTURE_PORT_ARG.to_owned(),
            "0".to_owned(),
            super::FIXTURE_PUBLIC_KEY_ARG.to_owned(),
            root.to_string_lossy().into_owned(),
        ]);
        assert!(invalid_port.is_err());

        let private_key = super::fixture_runtime_config_from_args([
            super::FIXTURE_PORT_ARG.to_owned(),
            "40000".to_owned(),
            super::FIXTURE_PUBLIC_KEY_ARG.to_owned(),
            root.to_string_lossy().into_owned(),
        ]);
        assert!(private_key.is_err());
        let _ = std::fs::remove_file(root);
    }

    #[cfg(feature = "tauri-update-fixture")]
    #[test]
    fn fixture_runtime_configuration_reads_bounded_keys_and_selects_new_only() {
        let root = std::env::temp_dir().join(format!(
            "sky-auto-player-fixture-public-{}",
            std::process::id()
        ));
        std::fs::write(&root, "fixture-public-root").expect("write fixture key");
        let config = super::fixture_runtime_config_from_args([
            "--selftest-desktop-update".to_owned(),
            super::FIXTURE_PORT_ARG.to_owned(),
            "40000".to_owned(),
            super::FIXTURE_PUBLIC_KEY_ARG.to_owned(),
            root.to_string_lossy().into_owned(),
            super::FIXTURE_NEW_ONLY_ARG.to_owned(),
        ])
        .expect("valid fixture runtime configuration");
        assert_eq!(config.port, 40000);
        assert_eq!(config.public_keys, vec!["fixture-public-root"]);
        assert!(config.new_only);
        let _ = std::fs::remove_file(root);
    }

    #[cfg(not(feature = "tauri-update-fixture"))]
    #[test]
    fn production_metadata_endpoints_are_fixed_and_channel_isolated() {
        let stable = metadata_endpoint(UpdateChannel::Stable).unwrap();
        assert_eq!(stable.as_str(), V4_STABLE_METADATA_ENDPOINT);
        assert_eq!(stable.scheme(), "https");
        assert_eq!(stable.host_str(), Some("raw.githubusercontent.com"));

        let beta_err = metadata_endpoint(UpdateChannel::Beta).unwrap_err();
        assert!(beta_err.contains("channel_unavailable"));
        assert_eq!(
            V4_BETA_METADATA_ENDPOINT,
            "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/beta/latest.json"
        );
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
    fn download_progress_accumulates_chunk_lengths() {
        let mut accumulator = DownloadProgressAccumulator::default();
        let completed = [16_000_usize, 16_000, 8_000]
            .into_iter()
            .map(|chunk_length| {
                accumulator
                    .record_chunk(chunk_length, Some(40_000))
                    .expect("valid chunk should produce progress")
                    .completed
            })
            .collect::<Vec<_>>();
        assert_eq!(completed, [16_000, 32_000, 40_000]);
    }

    #[test]
    fn download_progress_never_decreases_within_an_attempt() {
        let mut accumulator = DownloadProgressAccumulator::default();
        let mut previous = 0;
        for chunk_length in [16_000_usize, 16_000, 8_000, 1_000] {
            let progress = accumulator
                .record_chunk(chunk_length, Some(41_000))
                .expect("valid chunk should produce progress");
            assert!(progress.completed >= previous);
            previous = progress.completed;
        }
    }

    #[test]
    fn download_progress_stays_within_a_valid_total() {
        let mut accumulator = DownloadProgressAccumulator::default();
        for chunk_length in [16_000_usize, 16_000, 8_000] {
            let progress = accumulator
                .record_chunk(chunk_length, Some(40_000))
                .expect("valid chunk should produce progress");
            assert!(progress.completed <= progress.total.expect("total is valid"));
        }
    }

    #[test]
    fn download_progress_accumulates_without_a_content_length() {
        let mut accumulator = DownloadProgressAccumulator::default();
        let progress = [16_000_usize, 16_000, 8_000]
            .into_iter()
            .map(|chunk_length| {
                accumulator
                    .record_chunk(chunk_length, None)
                    .expect("chunk without total should produce progress")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            progress
                .iter()
                .map(|value| value.completed)
                .collect::<Vec<_>>(),
            [16_000, 32_000, 40_000]
        );
        assert!(progress.iter().all(|value| value.total.is_none()));
    }

    #[test]
    fn download_progress_fails_closed_at_the_artifact_bound() {
        let mut accumulator = DownloadProgressAccumulator::default();
        assert!(
            accumulator
                .record_chunk(MAX_ARTIFACT_BYTES as usize, None)
                .is_ok()
        );
        assert!(accumulator.record_chunk(1, None).is_err());

        let mut invalid_total = DownloadProgressAccumulator::default();
        let progress = invalid_total
            .record_chunk(1, Some(MAX_ARTIFACT_BYTES + 1))
            .expect("oversized content length is treated as unknown");
        assert_eq!(progress.total, None);

        let mut overflowed = DownloadProgressAccumulator {
            downloaded: u64::MAX,
        };
        assert!(overflowed.record_chunk(1, None).is_err());
    }

    #[test]
    fn final_ready_and_installing_progress_use_exact_verified_artifact_length() {
        let bytes = vec![0_u8; 40_000];
        let ready = verified_artifact_progress(&bytes, "Update is ready to install");
        let installing = verified_artifact_progress(&bytes, "Installing update and restarting");
        assert_eq!(ready.completed, bytes.len() as u64);
        assert_eq!(ready.total, Some(bytes.len() as u64));
        assert_eq!(installing.completed, bytes.len() as u64);
        assert_eq!(installing.total, Some(bytes.len() as u64));
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

    #[test]
    fn classify_check_error_maps_known_classes() {
        use crate::ui_events::UpdateErrorCode;
        assert_eq!(
            super::classify_check_error(
                "channel_unavailable: beta update channel is not supported"
            ),
            UpdateErrorCode::ChannelUnavailable
        );
        assert_eq!(
            super::classify_check_error("connection refused while fetching"),
            UpdateErrorCode::CheckFailed
        );
    }

    #[test]
    fn native_update_state_default_has_zero_revision_and_idle() {
        use crate::ui_events::{UpdateRetryAction, UpdateState};
        let state = super::NativeUpdateState::default();
        assert_eq!(state.revision, 0);
        assert_eq!(state.state, UpdateState::Idle);
        assert_eq!(state.retry_action, UpdateRetryAction::None);
        assert!(state.candidate.is_none());
        assert!(state.progress.is_none());
    }

    #[test]
    fn check_disposition_matrix_enforces_background_throttling_and_manual_bypass() {
        use crate::ui_events::{UpdateCheckDisposition, UpdateCheckOrigin};
        use sky_app_core::settings::UpdatePreferences;

        let mut prefs = UpdatePreferences::default();
        let now = 1_700_000_000;

        // Fresh default: background check is performed
        assert_eq!(
            super::check_disposition(UpdateCheckOrigin::Background, &prefs, now),
            UpdateCheckDisposition::Performed
        );

        // Auto check disabled: background check is disabled
        prefs.auto_check = false;
        assert_eq!(
            super::check_disposition(UpdateCheckOrigin::Background, &prefs, now),
            UpdateCheckDisposition::Disabled
        );
        // Manual check ignores auto_check = false
        assert_eq!(
            super::check_disposition(UpdateCheckOrigin::Manual, &prefs, now),
            UpdateCheckDisposition::Performed
        );

        // Re-enable auto_check, recent success check (< 24h)
        prefs.auto_check = true;
        prefs.last_check_ts = now - 1000;
        assert_eq!(
            super::check_disposition(UpdateCheckOrigin::Background, &prefs, now),
            UpdateCheckDisposition::Throttled
        );
        // Manual check bypasses throttling
        assert_eq!(
            super::check_disposition(UpdateCheckOrigin::Manual, &prefs, now),
            UpdateCheckDisposition::Performed
        );

        // >= 24h (86_400s) elapsed: background check is performed
        prefs.last_check_ts = now - 86_400;
        assert_eq!(
            super::check_disposition(UpdateCheckOrigin::Background, &prefs, now),
            UpdateCheckDisposition::Performed
        );

        // Recent failure (< 300s)
        prefs.last_check_ts = 0;
        prefs.last_error_ts = now - 100;
        assert_eq!(
            super::check_disposition(UpdateCheckOrigin::Background, &prefs, now),
            UpdateCheckDisposition::Throttled
        );
        assert_eq!(
            super::check_disposition(UpdateCheckOrigin::Manual, &prefs, now),
            UpdateCheckDisposition::Performed
        );

        // >= 300s elapsed after error: background check is performed
        prefs.last_error_ts = now - 300;
        assert_eq!(
            super::check_disposition(UpdateCheckOrigin::Background, &prefs, now),
            UpdateCheckDisposition::Performed
        );
    }

    #[test]
    fn native_update_state_transitions_maintain_monotonic_revisions_and_invariants() {
        use super::{NativeUpdateCandidate, NativeUpdateState, StateTransition};
        use crate::ui_events::{UpdateChannel, UpdateErrorCode, UpdateRetryAction, UpdateState};

        let mut state = NativeUpdateState::default();
        assert_eq!(state.revision, 0);
        assert_eq!(state.snapshot().revision, 0);

        // 1. Idle -> Checking
        let (rev1, snap1) = state.apply_transition(StateTransition {
            state: UpdateState::Checking,
            channel: UpdateChannel::Stable,
            ..Default::default()
        });
        assert_eq!(rev1, 1);
        assert_eq!(snap1.revision, 1);
        assert_eq!(snap1.state, UpdateState::Checking);
        assert_eq!(state.snapshot(), snap1);

        // 2. Checking -> Available
        let candidate = NativeUpdateCandidate {
            version: "4.1.0".to_string(),
            channel: UpdateChannel::Stable,
            release_notes: Some("Notes".to_string()),
            published_at: Some("2026-09-01T00:00:00Z".to_string()),
        };
        let (rev2, snap2) = state.apply_transition(StateTransition {
            state: UpdateState::Available,
            channel: UpdateChannel::Stable,
            candidate: Some(candidate.clone()),
            ..Default::default()
        });
        assert_eq!(rev2, 2);
        assert_eq!(snap2.revision, 2);
        assert_eq!(snap2.state, UpdateState::Available);
        assert_eq!(snap2.available_version.as_deref(), Some("4.1.0"));
        assert_eq!(snap2.release_notes.as_deref(), Some("Notes"));
        assert_eq!(state.snapshot(), snap2);

        // 3. Available -> Current (candidate cleared)
        let (rev3, snap3) = state.apply_transition(StateTransition {
            state: UpdateState::Current,
            channel: UpdateChannel::Stable,
            ..Default::default()
        });
        assert_eq!(rev3, 3);
        assert_eq!(snap3.revision, 3);
        assert_eq!(snap3.state, UpdateState::Current);
        assert!(snap3.available_version.is_none());
        assert_eq!(state.snapshot(), snap3);

        // 4. Checking error -> error fields set, candidate None, retry Check
        let (rev4, snap4) = state.apply_transition(StateTransition {
            state: UpdateState::Error,
            channel: UpdateChannel::Stable,
            error_code: Some(UpdateErrorCode::CheckFailed),
            error_detail: Some("network error".to_string()),
            retry_action: UpdateRetryAction::Check,
            ..Default::default()
        });
        assert_eq!(rev4, 4);
        assert_eq!(snap4.revision, 4);
        assert_eq!(snap4.state, UpdateState::Error);
        assert_eq!(snap4.error_code, Some(UpdateErrorCode::CheckFailed));
        assert_eq!(snap4.retry_action, UpdateRetryAction::Check);
        assert!(snap4.available_version.is_none());
        assert_eq!(state.snapshot(), snap4);

        // 5. Available -> Install error (e.g. download failure): candidate preserved, retry Install
        let _ = state.apply_transition(StateTransition {
            state: UpdateState::Available,
            channel: UpdateChannel::Stable,
            candidate: Some(candidate.clone()),
            ..Default::default()
        });
        let (rev6, snap6) = state.apply_transition(StateTransition {
            state: UpdateState::Error,
            channel: UpdateChannel::Stable,
            candidate: Some(candidate.clone()),
            error_code: Some(UpdateErrorCode::DownloadFailed),
            error_detail: Some("download failed".to_string()),
            retry_action: UpdateRetryAction::Install,
            ..Default::default()
        });
        assert_eq!(rev6, 6);
        assert_eq!(snap6.revision, 6);
        assert_eq!(snap6.state, UpdateState::Error);
        assert_eq!(snap6.error_code, Some(UpdateErrorCode::DownloadFailed));
        assert_eq!(snap6.retry_action, UpdateRetryAction::Install);
        assert_eq!(snap6.available_version.as_deref(), Some("4.1.0"));
        assert_eq!(state.snapshot(), snap6);

        // 6. Stale update error: candidate cleared, retry Check
        let (rev7, snap7) = state.apply_transition(StateTransition {
            state: UpdateState::Error,
            channel: UpdateChannel::Stable,
            error_code: Some(UpdateErrorCode::StaleUpdate),
            error_detail: Some("stale update".to_string()),
            retry_action: UpdateRetryAction::Check,
            ..Default::default()
        });
        assert_eq!(rev7, 7);
        assert_eq!(snap7.revision, 7);
        assert_eq!(snap7.state, UpdateState::Error);
        assert_eq!(snap7.retry_action, UpdateRetryAction::Check);
        assert!(snap7.available_version.is_none());
        assert_eq!(state.snapshot(), snap7);

        // 7. Reset to Idle: candidate, error, progress cleared
        let (rev8, snap8) = state.apply_transition(StateTransition {
            state: UpdateState::Idle,
            channel: UpdateChannel::Stable,
            ..Default::default()
        });
        assert_eq!(rev8, 8);
        assert_eq!(snap8.revision, 8);
        assert_eq!(snap8.state, UpdateState::Idle);
        assert!(snap8.available_version.is_none());
        assert!(snap8.error_code.is_none());
        assert!(snap8.error_detail.is_none());
        assert!(snap8.progress.is_none());
        assert_eq!(snap8.retry_action, UpdateRetryAction::None);
        assert_eq!(state.snapshot(), snap8);
    }

    #[test]
    fn check_persistence_failure_produces_state_persistence_failed_and_performed_ack() {
        use super::UpdateService;
        use crate::app_state::ActivityCoordinator;
        use crate::ui_events::{
            UiEvent, UpdateChannel, UpdateCheckDisposition, UpdateErrorCode, UpdateRetryAction,
            UpdateState,
        };
        use std::sync::atomic::{AtomicUsize, Ordering};

        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        let activity = ActivityCoordinator::default();
        let service = UpdateService::new(app.handle().clone(), activity);

        let event_count = AtomicUsize::new(0);
        let oversized_error = "disk write error (simulated): ".to_string() + &"x".repeat(8192);
        let ack = service
            .handle_check_persistence_failure(UpdateChannel::Stable, &oversized_error, &|event| {
                event_count.fetch_add(1, Ordering::SeqCst);
                if let UiEvent::UpdateChanged { v, payload } = event {
                    assert_eq!(v, 1);
                    assert_eq!(payload.state, UpdateState::Error);
                    assert_eq!(
                        payload.error_code,
                        Some(UpdateErrorCode::StatePersistenceFailed)
                    );
                    assert_eq!(payload.retry_action, UpdateRetryAction::Check);
                    let detail = payload.error_detail.unwrap();
                    assert!(detail.contains("disk write error"));
                    assert!(detail.len() <= 4096);
                } else {
                    panic!("unexpected event: {event:?}");
                }
                Ok(())
            })
            .expect("handled persistence failure");

        assert_eq!(ack.disposition, UpdateCheckDisposition::Performed);
        assert_eq!(event_count.load(Ordering::SeqCst), 1);

        let snapshot = service.current_snapshot();
        assert_eq!(snapshot.state, UpdateState::Error);
        assert_eq!(
            snapshot.error_code,
            Some(UpdateErrorCode::StatePersistenceFailed)
        );
        assert_eq!(snapshot.retry_action, UpdateRetryAction::Check);
        assert!(snapshot.error_detail.as_ref().unwrap().len() <= 4096);
    }

    #[test]
    fn native_update_state_bounds_oversized_error_detail_at_transition_boundary() {
        use super::{NativeUpdateState, StateTransition};
        use crate::ui_events::{UpdateChannel, UpdateErrorCode, UpdateRetryAction, UpdateState};

        let oversized = "x".repeat(8_192);
        let mut state = NativeUpdateState::default();

        let (_, snapshot) = state.apply_transition(StateTransition {
            state: UpdateState::Error,
            channel: UpdateChannel::Stable,
            error_code: Some(UpdateErrorCode::DownloadFailed),
            error_detail: Some(oversized),
            retry_action: UpdateRetryAction::Install,
            ..Default::default()
        });

        let detail = snapshot
            .error_detail
            .as_deref()
            .expect("error detail must be present");

        assert_eq!(detail.chars().count(), 4096);
        assert_eq!(detail, "x".repeat(4096));

        assert_eq!(snapshot.error_code, Some(UpdateErrorCode::DownloadFailed));
        assert_eq!(snapshot.retry_action, UpdateRetryAction::Install);

        let stored = state.snapshot();

        assert_eq!(stored.error_detail, snapshot.error_detail);
        assert_eq!(
            stored
                .error_detail
                .as_deref()
                .expect("stored detail")
                .chars()
                .count(),
            4096
        );
    }

    #[test]
    fn native_update_state_preserves_short_error_detail_exactly() {
        use super::{NativeUpdateState, StateTransition};
        use crate::ui_events::{UpdateChannel, UpdateErrorCode, UpdateRetryAction, UpdateState};

        let detail = "update install failed: signature verification failed";
        let mut state = NativeUpdateState::default();

        let (_, snapshot) = state.apply_transition(StateTransition {
            state: UpdateState::Error,
            channel: UpdateChannel::Stable,
            error_code: Some(UpdateErrorCode::InstallFailed),
            error_detail: Some(detail.to_string()),
            retry_action: UpdateRetryAction::Install,
            ..Default::default()
        });

        assert_eq!(snapshot.error_detail.as_deref(), Some(detail));
        assert_eq!(state.snapshot().error_detail.as_deref(), Some(detail));
        assert_eq!(snapshot.error_code, Some(UpdateErrorCode::InstallFailed));
        assert_eq!(snapshot.retry_action, UpdateRetryAction::Install);
    }

    #[test]
    fn native_update_state_bounds_error_detail_by_characters() {
        use super::{NativeUpdateState, StateTransition};
        use crate::ui_events::{UpdateChannel, UpdateErrorCode, UpdateRetryAction, UpdateState};

        let oversized = "é".repeat(5_000);
        let mut state = NativeUpdateState::default();

        let (_, snapshot) = state.apply_transition(StateTransition {
            state: UpdateState::Error,
            channel: UpdateChannel::Stable,
            error_code: Some(UpdateErrorCode::DownloadFailed),
            error_detail: Some(oversized),
            retry_action: UpdateRetryAction::Install,
            ..Default::default()
        });

        let detail = snapshot.error_detail.as_deref().unwrap();

        assert_eq!(detail.chars().count(), 4096);
        assert_eq!(detail, "é".repeat(4096));

        let stored = state.snapshot();
        assert_eq!(stored.error_detail, snapshot.error_detail);
        assert_eq!(
            stored
                .error_detail
                .as_deref()
                .expect("stored detail")
                .chars()
                .count(),
            4096
        );
    }
}
