use crate::commands::{
    UpdateCheckAckDto, UpdateCheckRequest, UpdateInstallAckDto, UpdatePreferencesDto,
    UpdatePreferencesPatch,
};
use crate::native_runtime::{
    CoreUpdatePreferencesPatch, NativeDesktopRuntime, settings_error, update_preferences_dto,
};
use crate::ui_events::UpdateSnapshotPayload;

impl NativeDesktopRuntime {
    pub(in crate::native_runtime) fn update_preferences(
        &self,
    ) -> Result<UpdatePreferencesDto, String> {
        let settings = self.reload_settings_snapshot()?;
        Ok(update_preferences_dto(&settings))
    }

    pub(in crate::native_runtime) fn check_update(
        &self,
        request: UpdateCheckRequest,
    ) -> Result<UpdateCheckAckDto, String> {
        let mut settings = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        self.update_service
            .as_ref()
            .ok_or_else(|| {
                "update_service_unavailable: Rust-owned UpdateService is not configured".to_string()
            })?
            .check(&request, &mut settings, |event| self.publish(event))
    }

    pub(crate) fn update_snapshot(&self) -> Result<UpdateSnapshotPayload, String> {
        self.update_service
            .as_ref()
            .ok_or_else(|| {
                "update_service_unavailable: Rust-owned UpdateService is not configured".to_string()
            })
            .map(|service| service.current_snapshot())
    }

    pub(in crate::native_runtime) fn begin_update_handoff(
        &self,
        target_version: String,
    ) -> Result<UpdateInstallAckDto, String> {
        if target_version.is_empty() || target_version.len() > 64 || target_version.contains('\0') {
            return Err("invalid_params: target_version is invalid".into());
        }
        let settings = self.reload_settings_snapshot()?;
        self.update_service
            .as_ref()
            .ok_or_else(|| {
                "update_service_unavailable: Rust-owned UpdateService is not configured".to_string()
            })?
            .install(&settings, &target_version, |event| self.publish(event))
    }

    pub(crate) fn patch_update_preferences(
        &self,
        patch: UpdatePreferencesPatch,
    ) -> Result<UpdatePreferencesDto, String> {
        if patch.channel == Some(crate::ui_events::UpdateChannel::Beta) {
            return Err(
                "channel_unavailable: beta update channel is not supported in production".into(),
            );
        }
        let mut settings = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        let before_channel = settings.snapshot().update.channel.clone();
        let before_skip_version = settings.snapshot().update.skip_version.clone();
        let snapshot = settings
            .patch(&sky_app_core::settings::SettingsPatch {
                update: Some(CoreUpdatePreferencesPatch {
                    auto_check: patch.auto_check,
                    channel: patch.channel.map(|channel| match channel {
                        crate::ui_events::UpdateChannel::Stable => {
                            sky_app_core::settings::UpdateChannel::Stable
                        }
                        crate::ui_events::UpdateChannel::Beta => {
                            sky_app_core::settings::UpdateChannel::Beta
                        }
                    }),
                    skip_version: patch.skip_version,
                }),
                ..Default::default()
            })
            .map_err(settings_error)?
            .clone();
        let candidate_invalidation_needed = before_channel != snapshot.update.channel
            || before_skip_version != snapshot.update.skip_version;
        drop(settings);
        if candidate_invalidation_needed && let Some(update_service) = &self.update_service {
            let channel = match snapshot.update.channel {
                sky_app_core::settings::UpdateChannel::Stable => {
                    crate::ui_events::UpdateChannel::Stable
                }
                sky_app_core::settings::UpdateChannel::Beta => {
                    crate::ui_events::UpdateChannel::Beta
                }
            };
            update_service.reset_and_publish_idle(channel, |event| self.publish(event))?;
        }
        Ok(update_preferences_dto(&snapshot))
    }
}
