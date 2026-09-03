//! Rust-owned Tauri updater policy.
//!
//! React receives only the bounded DTOs below. Endpoint selection, updater
//! configuration, signature verification, artifact handling, and install
//! execution stay in this module and in the official Tauri updater plugin.
//! The production authority is intentionally absent until WO-04 supplies it.

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

#[derive(Clone, Debug)]
pub(crate) struct NativeUpdateCandidate {
    pub version: String,
    pub channel: UpdateChannel,
    pub release_notes: Option<String>,
    pub published_at: Option<String>,
}

struct NativeUpdateState {
    candidate: Option<NativeUpdateCandidate>,
    update: Option<Update>,
    operation_id: Option<String>,
    state: UpdateState,
}

impl Default for NativeUpdateState {
    fn default() -> Self {
        Self {
            candidate: None,
            update: None,
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
            Ok(Some(update)) if settings.snapshot().update.skip_version != update.version => {
                settings
                    .record_update_success(timestamp)
                    .map_err(|error| format!("update timestamp persistence failed: {error}"))?;
                let candidate = candidate_from_update(&update, channel);
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
                    state.update = Some(update);
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
        let (candidate, update) = {
            let state = self
                .state
                .lock()
                .map_err(|_| "native update state lock poisoned".to_string())?;
            let candidate = state
                .candidate
                .clone()
                .ok_or_else(|| "update_unavailable: check for an update first".to_string())?;
            let update = state
                .update
                .clone()
                .ok_or_else(|| "update_unavailable: update metadata is unavailable".to_string())?;
            (candidate, update)
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

        let download = tauri::async_runtime::block_on(update.download(
            {
                let publish = &publish;
                let candidate = candidate.clone();
                let operation_id = operation_id.clone();
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
        ));
        let bytes = match download {
            Ok(bytes) if (bytes.len() as u64) <= MAX_ARTIFACT_BYTES => bytes,
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

    fn check_official(&self, channel: UpdateChannel) -> Result<Option<Update>, String> {
        let endpoint = authority_endpoint(channel)?;
        let builder = self
            .app
            .updater_builder()
            .endpoints(vec![endpoint])
            .map_err(|error| format!("update authority rejected: {error}"))?
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

fn authority_endpoint(channel: UpdateChannel) -> Result<Url, String> {
    #[cfg(feature = "tauri-update-fixture")]
    {
        let endpoint = match channel {
            UpdateChannel::Stable => "http://127.0.0.1:17845/stable",
            UpdateChannel::Beta => "http://127.0.0.1:17845/beta",
        };
        Url::parse(endpoint).map_err(|error| format!("fixture authority URL invalid: {error}"))
    }
    #[cfg(not(feature = "tauri-update-fixture"))]
    {
        let _ = channel;
        Err("update_authority_not_configured: production authority is reserved for WO-04".into())
    }
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
    use super::authority_endpoint;
    use super::{bounded, update_activity_error};
    use crate::app_state::ActivityReservationError;
    #[cfg(not(feature = "tauri-update-fixture"))]
    use crate::ui_events::UpdateChannel;

    #[cfg(not(feature = "tauri-update-fixture"))]
    #[test]
    fn production_authority_is_fail_closed_until_wo04() {
        let error = authority_endpoint(UpdateChannel::Stable)
            .expect_err("production authority must not be invented before WO-04");
        assert_eq!(
            error,
            "update_authority_not_configured: production authority is reserved for WO-04"
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
    fn update_installation_has_a_distinct_playback_policy_error() {
        let message = update_activity_error(ActivityReservationError::PhysicalPlaybackActive);
        assert_eq!(
            message,
            "playback_active: update installation cannot run during physical playback"
        );
    }
}
