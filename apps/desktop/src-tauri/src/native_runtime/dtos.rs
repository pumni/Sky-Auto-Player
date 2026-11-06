use crate::commands::{
    AppearanceMode, NativeBuildDto, PlaybackDefaultsDto, SettingsDto,
    TimingMarginRecommendationDto, UpdatePreferencesDto,
};
use sky_app_core::settings::{AppearanceMode as CoreAppearanceMode, ApplicationSettings};

pub(crate) fn native_build_dto() -> NativeBuildDto {
    NativeBuildDto {
        native_build_commit: option_env!("SKY_NATIVE_BUILD_COMMIT")
            .unwrap_or("unknown")
            .into(),
        native_version: env!("CARGO_PKG_VERSION").into(),
        schema_version: sky_dispatch_win32::calibration::CALIBRATION_SCHEMA_VERSION as u64,
        native_abi: option_env!("SKY_NATIVE_ABI")
            .unwrap_or("native-win32")
            .into(),
        rustc_version: option_env!("SKY_RUSTC_VERSION").unwrap_or("unknown").into(),
        win32_backend: sky_dispatch_win32::win32_available(),
    }
}

pub(super) fn playback_defaults(settings: &ApplicationSettings) -> PlaybackDefaultsDto {
    PlaybackDefaultsDto {
        hold_frames: settings.playback_defaults.hold_frames,
        timing_margin_us: settings.playback_defaults.timing_margin_us,
        tempo_scale: settings.playback_defaults.tempo_scale,
        fps: settings.playback_defaults.fps,
        dry_run: false,
    }
}

pub(super) fn update_preferences_dto(settings: &ApplicationSettings) -> UpdatePreferencesDto {
    UpdatePreferencesDto {
        auto_check: settings.update.auto_check,
        channel: match settings.update.channel {
            sky_app_core::settings::UpdateChannel::Stable => {
                crate::ui_events::UpdateChannel::Stable
            }
            sky_app_core::settings::UpdateChannel::Beta => crate::ui_events::UpdateChannel::Beta,
        },
        skip_version: settings.update.skip_version.clone(),
        check_interval_s: settings.update.check_interval_s,
        last_check_ts: settings.update.last_check_ts,
        last_error_ts: settings.update.last_error_ts,
    }
}

pub(super) fn settings_dto(
    settings: &ApplicationSettings,
    timing_margin_recommendation: TimingMarginRecommendationDto,
) -> SettingsDto {
    SettingsDto {
        appearance_mode: match settings.appearance_mode {
            CoreAppearanceMode::System => AppearanceMode::System,
            CoreAppearanceMode::Light => AppearanceMode::Light,
            CoreAppearanceMode::Dark => AppearanceMode::Dark,
        },
        palette: settings.palette.clone(),
        ui_background_mode: settings.ui_background_mode.clone(),
        playback_defaults: playback_defaults(settings),
        auto_play: settings.playback_behavior.auto_play,
        timing_margin_recommendation,
        telemetry_enabled: settings.telemetry_enabled,
        verbose_hud: settings.verbose_hud,
        update_preferences: update_preferences_dto(settings),
    }
}
