use crate::commands::{SettingsDto, SettingsPatch};
use crate::native_runtime::playback::settings_fingerprint;
use crate::native_runtime::{NativeDesktopRuntime, settings_dto, settings_error};
use sky_app_core::settings::{
    ApplicationSettings, PlaybackDefaultsPatch,
    UpdatePreferencesPatch as CoreUpdatePreferencesPatch,
};

impl NativeDesktopRuntime {
    pub(in crate::native_runtime) fn settings_snapshot(
        &self,
    ) -> Result<ApplicationSettings, String> {
        let service = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        Ok(service.snapshot().clone())
    }

    pub(in crate::native_runtime) fn reload_settings_snapshot(
        &self,
    ) -> Result<ApplicationSettings, String> {
        // External settings-file edits become authoritative only at explicit
        // refresh boundaries. Normal startup reads use the coherent in-memory
        // snapshot so background work cannot silently change startup state.
        crate::startup_telemetry::record("settings.reload.start");
        let mut prepare_publication = self
            .playback
            .prepare_publication
            .lock()
            .map_err(|_| "native Prepare publication lock poisoned".to_string())?;
        let mut service = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        let before = service.snapshot().clone();
        service
            .reload()
            .map_err(|error| format!("native settings reload failed: {error}"))?;
        let snapshot = service.snapshot().clone();
        if before.liked_songs != snapshot.liked_songs {
            self.catalog_search_cache
                .lock()
                .map_err(|_| "native catalog search cache lock poisoned".to_string())?
                .invalidate_liked();
            self.invalidate_viewport_projection();
        }
        if before.playback_defaults != snapshot.playback_defaults {
            self.invalidate_analysis_cache();
        }
        let fingerprint = settings_fingerprint(&snapshot)?;
        if prepare_publication
            .settings_fingerprint
            .as_ref()
            .is_some_and(|current| current != &fingerprint)
        {
            self.playback
                .invalidate_settings_locked(&mut prepare_publication, Some(fingerprint));
        } else {
            self.playback
                .set_settings_fingerprint_locked(&mut prepare_publication, fingerprint);
        }
        crate::startup_telemetry::record("settings.reload.end");
        Ok(snapshot)
    }

    pub(in crate::native_runtime) fn reload_settings_dto(&self) -> Result<SettingsDto, String> {
        let settings = self.reload_settings_snapshot()?;
        Ok(settings_dto(&settings))
    }

    pub(crate) fn patch_settings(&self, patch: SettingsPatch) -> Result<SettingsDto, String> {
        if patch.update_preferences.as_ref().and_then(|u| u.channel)
            == Some(crate::ui_events::UpdateChannel::Beta)
        {
            return Err(
                "channel_unavailable: beta update channel is not supported in production".into(),
            );
        }
        let preparation_safe_patch = (patch.appearance_mode.is_some()
            || patch.palette.is_some()
            || patch.auto_play.is_some())
            && patch.telemetry_enabled.is_none()
            && patch.verbose_hud.is_none()
            && patch.playback_defaults.is_none()
            && patch.update_preferences.is_none();
        let core_patch = sky_app_core::settings::SettingsPatch {
            appearance_mode: patch.appearance_mode.map(|mode| mode.as_str().to_owned()),
            palette: patch.palette,
            telemetry_enabled: patch.telemetry_enabled,
            verbose_hud: patch.verbose_hud,
            playback_defaults: patch.playback_defaults.map(|value| PlaybackDefaultsPatch {
                hold_frames: value.hold_frames,
                timing_margin_us: value.timing_margin_us,
                tempo_scale: value.tempo_scale,
                fps: value.fps,
            }),
            auto_play: patch.auto_play,
            update: patch
                .update_preferences
                .map(|value| CoreUpdatePreferencesPatch {
                    auto_check: value.auto_check,
                    channel: value.channel.map(|channel| match channel {
                        crate::ui_events::UpdateChannel::Stable => {
                            sky_app_core::settings::UpdateChannel::Stable
                        }
                        crate::ui_events::UpdateChannel::Beta => {
                            sky_app_core::settings::UpdateChannel::Beta
                        }
                    }),
                    skip_version: value.skip_version,
                }),
        };
        // The Prepare publication gate precedes the settings lock everywhere
        // settings can affect prepared plans. This makes mutation plus cache
        // invalidation atomic with respect to Prepare's final publication.
        let mut prepare_publication = self
            .playback
            .prepare_publication
            .lock()
            .map_err(|_| "native Prepare publication lock poisoned".to_string())?;
        let mut settings = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        let before_fingerprint = settings_fingerprint(settings.snapshot())?;
        let before_channel = settings.snapshot().update.channel.clone();
        let before_skip_version = settings.snapshot().update.skip_version.clone();
        let snapshot = settings.patch(&core_patch).map_err(settings_error)?.clone();
        let candidate_invalidation_needed = before_channel != snapshot.update.channel
            || before_skip_version != snapshot.update.skip_version;
        let after_fingerprint = settings_fingerprint(&snapshot)?;
        if !preparation_safe_patch || before_fingerprint != after_fingerprint {
            self.playback
                .invalidate_settings_locked(&mut prepare_publication, Some(after_fingerprint));
        } else {
            self.playback
                .set_settings_fingerprint_locked(&mut prepare_publication, after_fingerprint);
        }
        if !preparation_safe_patch {
            self.invalidate_analysis_cache();
        }
        drop(settings);
        drop(prepare_publication);
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
        Ok(settings_dto(&snapshot))
    }
}
