use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::settings::{PlaybackDefaultsDto, PlaybackOptionSetsDto, SettingsDto};
use super::update::UpdatePreferencesDto;
use crate::ui_events::CatalogReadiness;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NativeBuildDto {
    pub native_build_commit: String,
    pub native_version: String,
    pub schema_version: u64,
    pub native_abi: String,
    pub rustc_version: String,
    pub win32_backend: bool,
}
/// Bootstrap payload. `gui_smoke` is a native-authoritative packaged GUI-smoke
/// mode indicator, not a security capability or an unforgeable renderer token.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BootstrapDto {
    pub app_version: String,
    pub protocol_version: u64,
    pub native_build: NativeBuildDto,
    /// Authoritative coherent settings snapshot for initial frontend state.
    pub settings: SettingsDto,
    /// Flattened fields remain additive compatibility aliases for protocol v1 clients.
    pub playback_defaults: PlaybackDefaultsDto,
    pub option_sets: PlaybackOptionSetsDto,
    /// Canonical flattened palette field for protocol-v1 clients.
    pub palette: String,
    /// Legacy/deprecated protocol-v1 compatibility alias for `palette`.
    pub theme: String,
    pub telemetry_enabled: bool,
    pub update_preferences: UpdatePreferencesDto,
    pub catalog_state: CatalogReadiness,
    pub catalog_generation: Option<u64>,
    #[serde(default)]
    pub gui_smoke: bool,
}
