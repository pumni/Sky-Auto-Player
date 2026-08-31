//! Native desktop application runtime.
//!
//! This is the composition root for commands that have crossed the strangler
//! boundary.  It owns live application state and calls pure app-core services
//! plus outer adapters.  It never delegates a native-owned command to the
//! Python Core.

use crate::commands::{
    BootstrapDto, CatalogDetailRequest, CatalogReloadDto, CatalogRowDto, CatalogSearchDto,
    CatalogSearchRequest, CatalogViewportDto, CatalogViewportRequest, DiagnosticsEnabledDto,
    DiagnosticsSetEnabledRequest, PlaybackAdmission, PlaybackCommandAckDto, PlaybackConfigDto,
    PlaybackDecision, PlaybackDecisionAcceptanceDto, PlaybackDefaultsDto, PlaybackPendingControl,
    PlaybackPlanVariantDto, PlaybackPrepareRequest, PlaybackSessionDto, PlaybackSessionState,
    PlaybackStartRequest, PreparedPlaybackDto, RiskDecisionDto, RiskSummaryDto, SettingsDto,
    SettingsPatch, SongDetailDto, UpdatePreferencesDto, UpdatePreferencesPatch,
};
use crate::ui_events::{
    CatalogChangedPayload, PlaybackEventState, PlaybackFailedPayload, PlaybackFinishedPayload,
    PlaybackFocusState, PlaybackHealthState, PlaybackSnapshotPayload, PlaybackStateChangedPayload,
    UiEvent,
};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sky_app_core::catalog::{CatalogError, CatalogIndex, SongSource, WRatioRanker};
use sky_app_core::settings::{
    ApplicationSettings, PlaybackDefaultsPatch, SettingsError, SettingsService,
    UpdatePreferencesPatch as CoreUpdatePreferencesPatch,
};
use sky_app_core::song::{
    ActionKind, DOWN_LATE_GRACE_US, MIN_TRANSPORT_MARGIN_US, RiskReport, ScheduleMetadata, Song,
    analyze_schedule_with_context, build_schedule, frame_us, parse_song_json,
};
use sky_native_adapters::{FileCatalogSource, JsonSettingsStore};
use sky_player::adapter_support::{
    ActionKind as DispatchActionKind, KeyActionInput, PriorityMode, compile_runtime_intents,
};
use sky_player::engine::{
    BackendConfig, DispatchProfile, EnginePollStatus, FocusOptions, NativeDispatchSession,
    NativeSessionOptions, PriorityOptions, TelemetryMode, TelemetryOptions, TimingOptions,
    WaitOptions,
};
use smallvec::SmallVec;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::ipc::Channel;

pub(crate) const MAX_NATIVE_EVENTS: usize = 128;
const MAX_PREPARED_PLANS: usize = 64;
static NEXT_NATIVE_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) struct NativeDesktopRuntime {
    #[allow(dead_code)]
    install_root: PathBuf,
    settings: Mutex<SettingsService<JsonSettingsStore>>,
    catalog_source: FileCatalogSource,
    catalog: Mutex<CatalogIndex>,
    events: Arc<Mutex<NativeEventHub>>,
    playback: Arc<NativePlaybackService>,
    closed: AtomicBool,
}

impl NativeDesktopRuntime {
    pub(crate) fn for_current_install() -> Result<Self, String> {
        Self::from_install_root(resolve_install_root()?)
    }

    pub(crate) fn from_install_root(install_root: PathBuf) -> Result<Self, String> {
        let settings_path = install_root.join("config.json");
        let settings_store = JsonSettingsStore::new(settings_path);
        let settings = SettingsService::load(settings_store)
            .map_err(|error| format!("native settings startup failed: {error}"))?;
        let songs_dir = install_root.join(settings.snapshot().songs_dir.clone());
        Ok(Self {
            install_root,
            settings: Mutex::new(settings),
            catalog_source: FileCatalogSource::new(songs_dir),
            catalog: Mutex::new(CatalogIndex::default()),
            events: Arc::new(Mutex::new(NativeEventHub::default())),
            playback: Arc::new(NativePlaybackService::new()),
            closed: AtomicBool::new(false),
        })
    }

    #[allow(dead_code)]
    pub(crate) fn install_root(&self) -> &Path {
        &self.install_root
    }

    pub(crate) fn dispatch(&self, method: &str, params: Value) -> Result<Value, String> {
        if self.closed.load(Ordering::Acquire) {
            return Err("native desktop runtime is shut down".into());
        }
        if crate::command_ownership::owner_for(method)
            != Some(crate::command_ownership::CommandOwner::Native)
        {
            return Err(format!(
                "native runtime does not own desktop command: {method}"
            ));
        }
        match method {
            "app.bootstrap" => encode_result(self.bootstrap()),
            "catalog.search" => {
                let request: CatalogSearchRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.search(request))
            }
            "catalog.detail" => {
                let request: NativeCatalogDetailRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.detail(CatalogDetailRequest {
                    song_id: request.song_id,
                    generation: request.generation,
                }))
            }
            "catalog.reload" => encode_result(self.reload()),
            "catalog.set_viewport" => {
                let request: NativeCatalogViewportRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.set_viewport(CatalogViewportRequest {
                    generation: request.generation,
                    first_index: request.first_index,
                    last_index: request.last_index,
                    selected_song_id: request.selected_song_id,
                }))
            }
            "settings.get" => encode_result(self.settings_dto()),
            "settings.patch" => {
                let request: NativeSettingsPatch =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.patch_settings(request.into_public()))
            }
            "update.preferences.get" => encode_result(self.update_preferences()),
            "update.preferences.patch" => {
                let request: NativeUpdatePreferencesPatch =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.patch_update_preferences(request.into_public()))
            }
            "playback.prepare" => {
                let request: NativePlaybackPrepareRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.prepare_playback(request.into_public()))
            }
            "playback.start" => {
                let request: NativePlaybackStartRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.start_playback(request.into_public()))
            }
            "playback.stop" | "playback.pause" | "playback.resume" | "playback.skip" => {
                let request: NativePlaybackSessionRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.playback_command(method, request.session_id))
            }
            "diagnostics.set_enabled" => {
                let request: DiagnosticsSetEnabledRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.set_diagnostics_enabled(request))
            }
            _ => Err(format!("native command is not implemented: {method}")),
        }
    }

    pub(crate) fn bootstrap(&self) -> Result<BootstrapDto, String> {
        let snapshot = self.ensure_catalog_loaded()?;
        let settings = self.settings_snapshot()?;
        // Core remains the delivery source for `core.ready` while the
        // transitional Python-owned commands still require its event stream.
        // Emitting a second CoreReady here would duplicate the stable event on
        // the shared Native/Core channel. The native bootstrap DTO below is
        // authoritative for the command response itself.
        Ok(BootstrapDto {
            app_version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: crate::core::protocol::DESKTOP_PROTOCOL_VERSION,
            native_build: native_build_dto(),
            playback_defaults: playback_defaults(&settings),
            option_sets: crate::commands::PlaybackOptionSetsDto {
                hold_frames: vec![1.0, 1.25, 1.5],
                tempo_scales: vec![0.90, 0.95, 1.0, 1.05, 1.10],
                fps: sky_app_core::settings::VALID_FPS.to_vec(),
            },
            theme: settings.theme.clone(),
            telemetry_enabled: settings.telemetry_enabled,
            update_preferences: update_preferences_dto(&settings),
            catalog_generation: snapshot.generation,
        })
    }

    fn settings_snapshot(&self) -> Result<ApplicationSettings, String> {
        let mut service = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        // Python-owned settings commands update this same file through an
        // atomic replace, while Core keeps a process-local cache. Reloading
        // the native shadow before a read prevents native playback and detail
        // paths from observing stale persisted settings without introducing a
        // second write authority or Native->Python fallback.
        service
            .reload()
            .map_err(|error| format!("native settings reload failed: {error}"))?;
        Ok(service.snapshot().clone())
    }

    fn settings_dto(&self) -> Result<SettingsDto, String> {
        let settings = self.settings_snapshot()?;
        Ok(settings_dto(&settings))
    }

    fn patch_settings(&self, patch: SettingsPatch) -> Result<SettingsDto, String> {
        let core_patch = sky_app_core::settings::SettingsPatch {
            theme: patch.theme,
            telemetry_enabled: patch.telemetry_enabled,
            verbose_hud: patch.verbose_hud,
            playback_defaults: patch.playback_defaults.map(|value| PlaybackDefaultsPatch {
                hold_frames: value.hold_frames,
                tempo_scale: value.tempo_scale,
                fps: value.fps,
            }),
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
        let mut settings = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        let snapshot = settings.patch(&core_patch).map_err(settings_error)?;
        self.playback.invalidate_settings();
        Ok(settings_dto(snapshot))
    }

    fn update_preferences(&self) -> Result<UpdatePreferencesDto, String> {
        let settings = self.settings_snapshot()?;
        Ok(update_preferences_dto(&settings))
    }

    fn patch_update_preferences(
        &self,
        patch: UpdatePreferencesPatch,
    ) -> Result<UpdatePreferencesDto, String> {
        let mut settings = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
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
            .map_err(settings_error)?;
        Ok(update_preferences_dto(snapshot))
    }

    fn ensure_catalog_loaded(&self) -> Result<sky_app_core::catalog::CatalogSnapshot, String> {
        let mut catalog = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?;
        if catalog.generation() == 0 {
            let entries = self.catalog_source.entries().map_err(catalog_error)?;
            catalog.replace_entries(entries).map_err(catalog_error)?;
        }
        Ok(catalog.snapshot())
    }

    fn reload(&self) -> Result<CatalogReloadDto, String> {
        let entries = self.catalog_source.entries().map_err(catalog_error)?;
        let snapshot = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?
            .replace_entries(entries)
            .map_err(catalog_error)?;
        self.playback.invalidate_catalog(snapshot.generation);
        self.publish(UiEvent::CatalogChanged {
            v: crate::core::protocol::DESKTOP_PROTOCOL_VERSION,
            payload: CatalogChangedPayload {
                generation: snapshot.generation,
                total: snapshot.total as u64,
            },
        })?;
        Ok(CatalogReloadDto {
            generation: snapshot.generation,
            total: snapshot.total as u64,
        })
    }

    fn search(&self, request: CatalogSearchRequest) -> Result<CatalogSearchDto, String> {
        self.ensure_catalog_loaded()?;
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?;
        let page = catalog
            .search(
                &WRatioRanker,
                &request.query,
                request.offset as usize,
                request.limit as usize,
                request.generation,
            )
            .map_err(catalog_error)?;
        Ok(CatalogSearchDto {
            items: page
                .items
                .into_iter()
                .map(|row| CatalogRowDto {
                    song_id: row.song_id,
                    title: row.title,
                    duration_us: None,
                    note_count: None,
                    risk_level: "unknown".into(),
                    metadata_state: "pending".into(),
                })
                .collect(),
            offset: request.offset,
            limit: request.limit,
            total: page.total as u64,
            generation: page.generation,
        })
    }

    fn detail(&self, request: CatalogDetailRequest) -> Result<SongDetailDto, String> {
        self.ensure_catalog_loaded()?;
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?;
        let entry = catalog
            .entry_for_song_id(&request.song_id, request.generation)
            .map_err(catalog_error)?;
        let path = PathBuf::from(&entry.canonical_path);
        let bytes = fs::read(&path).map_err(|error| format!("song read failed: {error}"))?;
        let fallback = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(&entry.row.title);
        let song = parse_song_json(&bytes, fallback).map_err(|error| error.to_string())?;
        let settings = self.settings_snapshot()?;
        let schedule = build_schedule(
            &song,
            settings.playback_defaults.hold_frames,
            1.0,
            settings.playback_defaults.fps,
        )
        .map_err(|error| error.to_string())?;
        let risk = analyze_schedule_with_context(
            &schedule,
            Some(&song.notes),
            settings.playback_defaults.hold_frames,
            1.0,
        );
        let risk_level = match risk.severity.as_str() {
            "low" | "medium" | "high" => risk.severity.clone(),
            _ => "unknown".into(),
        };
        let recommendations = if risk_level == "unknown" {
            Vec::new()
        } else {
            risk.recommendations.clone()
        };
        let reasons = if risk_level == "low" {
            Vec::new()
        } else {
            recommendations.clone()
        };
        let recommendation =
            (risk_level != "unknown").then(|| crate::commands::PlaybackRecommendationDto {
                recommended_hold_frames: risk.suggested_hold_frames,
                recommended_tempo_scale: risk.suggested_tempo_scale,
                summary: recommendations
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "Keep the selected settings.".into()),
            });
        Ok(SongDetailDto {
            song_id: entry.row.song_id,
            title: entry.row.title,
            duration_us: schedule.source_duration_us,
            note_count: song.notes.len() as u64,
            format_label: path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("unknown")
                .to_ascii_uppercase(),
            risk: RiskSummaryDto {
                level: risk_level.clone(),
                headline: match risk_level.as_str() {
                    "low" => "Low timing risk".into(),
                    "medium" => "Medium timing risk".into(),
                    "high" => "High timing risk".into(),
                    _ => "Risk unavailable".into(),
                },
                reasons,
                recommendations,
            },
            recommendation,
        })
    }

    fn set_viewport(&self, request: CatalogViewportRequest) -> Result<CatalogViewportDto, String> {
        self.ensure_catalog_loaded()?;
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?;
        let snapshot = catalog.snapshot();
        if snapshot.generation != request.generation {
            return Err("catalog generation is stale".into());
        }
        if snapshot.total == 0 {
            if request.first_index != 0
                || request.last_index != -1
                || request.selected_song_id.is_some()
            {
                return Err("empty catalog viewport must be 0..-1 with no selected song".into());
            }
        } else if request.last_index < request.first_index as i64
            || request.last_index as u64 >= snapshot.total as u64
            || request
                .last_index
                .saturating_sub(request.first_index as i64)
                .saturating_add(1)
                > 2_000
        {
            return Err("catalog viewport is outside bounded index range".into());
        }
        if let Some(song_id) = &request.selected_song_id {
            catalog
                .canonical_path_for_song_id(song_id, Some(request.generation))
                .map_err(catalog_error)?;
        }
        Ok(CatalogViewportDto {
            accepted: true,
            generation: request.generation,
            first_index: request.first_index,
            last_index: request.last_index,
            selected_song_id: request.selected_song_id,
        })
    }

    fn prepare_playback(
        &self,
        request: crate::commands::PlaybackPrepareRequest,
    ) -> Result<PreparedPlaybackDto, String> {
        self.ensure_catalog_loaded()?;
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?;
        let entry = catalog
            .entry_for_song_id(&request.song_id, Some(request.generation))
            .map_err(catalog_error)?;
        let path = PathBuf::from(&entry.canonical_path);
        let bytes = fs::read(&path).map_err(|error| format!("song read failed: {error}"))?;
        let fallback = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(&entry.row.title);
        let song = parse_song_json(&bytes, fallback).map_err(|error| error.to_string())?;
        let schedule = build_schedule(
            &song,
            request.config.hold_frames,
            request.config.tempo_scale,
            request.config.fps,
        )
        .map_err(|error| error.to_string())?;
        let risk = analyze_schedule_with_context(
            &schedule,
            Some(&song.notes),
            request.config.hold_frames,
            request.config.tempo_scale,
        );
        self.playback.prepare(
            request.song_id,
            request.generation,
            request.config,
            song,
            schedule,
            risk,
        )
    }

    fn start_playback(
        &self,
        request: crate::commands::PlaybackStartRequest,
    ) -> Result<PlaybackSessionDto, String> {
        let settings = self.settings_snapshot()?;
        self.playback.start(request, &settings, self.events.clone())
    }

    fn playback_command(
        &self,
        method: &str,
        session_id: String,
    ) -> Result<PlaybackCommandAckDto, String> {
        self.playback
            .command(method, session_id, self.events.clone())
    }

    fn set_diagnostics_enabled(
        &self,
        request: DiagnosticsSetEnabledRequest,
    ) -> Result<DiagnosticsEnabledDto, String> {
        self.playback
            .set_diagnostics_enabled(request.enabled, self.events.clone())?;
        Ok(DiagnosticsEnabledDto {
            enabled: request.enabled,
        })
    }

    pub(crate) fn subscribe(&self, channel: Channel<UiEvent>) -> Result<(), String> {
        self.events
            .lock()
            .map_err(|_| "native event hub lock poisoned".to_string())?
            .subscribe(channel)
    }

    fn publish(&self, event: UiEvent) -> Result<(), String> {
        self.events
            .lock()
            .map_err(|_| "native event hub lock poisoned".to_string())?
            .publish(event)
    }

    pub(crate) fn shutdown(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            self.playback.shutdown(self.events.clone());
            if let Ok(mut events) = self.events.lock() {
                events.close();
            }
        }
    }
}

/// Application-side playback control plane.  The realtime worker remains in
/// `sky_player`; this service owns prepared-plan admission, the single active
/// session rule, and the bounded supervisor loop around that worker.
struct NativePlaybackService {
    prepared: Mutex<HashMap<String, NativePreparedPlan>>,
    active: Arc<Mutex<Option<Arc<NativeActivePlayback>>>>,
    last_terminal: Arc<Mutex<Option<(String, PlaybackSessionState)>>>,
    diagnostics_enabled: Arc<AtomicBool>,
    diagnostics_sequence: Arc<AtomicU64>,
}

#[derive(Clone)]
struct NativePreparedPlan {
    song_id: String,
    generation: u64,
    song: Song,
    dto: PreparedPlaybackDto,
    variants: HashMap<PlaybackDecision, NativePlaybackVariant>,
}

#[derive(Clone)]
struct NativePlaybackVariant {
    config: PlaybackConfigDto,
    schedule: ScheduleMetadata,
    fingerprint: String,
}

struct NativeActivePlayback {
    session_id: String,
    prepared_id: String,
    song_id: String,
    title: String,
    total_us: u64,
    config: PlaybackConfigDto,
    plan_fingerprint: String,
    physical: bool,
    state: Mutex<PlaybackSessionState>,
    pending: Mutex<Option<PlaybackPendingControl>>,
    player: Option<Arc<NativeDispatchSession>>,
    started_at: Instant,
    paused_since: Mutex<Option<Instant>>,
    paused_total: Mutex<Duration>,
    stop_requested: AtomicBool,
    skip_requested: AtomicBool,
    done: AtomicBool,
    sequence: AtomicU64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativePlaybackPrepareRequest {
    song_id: String,
    generation: u64,
    config: PlaybackConfigDto,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativePlaybackStartRequest {
    prepared_id: String,
    decisions: Vec<PlaybackDecisionAcceptanceDto>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativePlaybackSessionRequest {
    session_id: String,
}

impl NativePlaybackPrepareRequest {
    fn into_public(self) -> PlaybackPrepareRequest {
        PlaybackPrepareRequest {
            song_id: self.song_id,
            generation: self.generation,
            config: self.config,
        }
    }
}

impl NativePlaybackStartRequest {
    fn into_public(self) -> PlaybackStartRequest {
        PlaybackStartRequest {
            prepared_id: self.prepared_id,
            decisions: self.decisions,
        }
    }
}

fn opaque_native_id() -> String {
    let sequence = NEXT_NATIVE_ID.fetch_add(1, Ordering::Relaxed);
    let mut hasher = Sha256::new();
    hasher.update(sequence.to_le_bytes());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    hasher.update(now.as_nanos().to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    let digest = hasher.finalize();
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn plan_fingerprint(
    song_id: &str,
    config: &PlaybackConfigDto,
    schedule: &ScheduleMetadata,
) -> Result<String, String> {
    let frame = frame_us(config.fps).map_err(|error| error.to_string())?;
    let actions = schedule
        .actions
        .iter()
        .map(|action| {
            serde_json::json!([
                match action.kind {
                    ActionKind::Down => "down",
                    ActionKind::Up => "up",
                },
                action.at_us,
                action.scan_codes,
            ])
        })
        .collect::<Vec<_>>();
    let payload = serde_json::json!({
        "song_id": song_id,
        "config": config,
        "policy": {
            "fps": config.fps,
            "min_hold_us": ((config.hold_frames * frame as f64).ceil() as u64)
                .saturating_add(DOWN_LATE_GRACE_US)
                .saturating_add(MIN_TRANSPORT_MARGIN_US),
            "min_release_gap_us": frame
                .saturating_add(DOWN_LATE_GRACE_US)
                .saturating_add(MIN_TRANSPORT_MARGIN_US),
        },
        "actions": actions,
    });
    let bytes = serde_json::to_vec(&payload).map_err(json_error)?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn risk_summary(risk: &RiskReport) -> RiskSummaryDto {
    RiskSummaryDto {
        level: risk.severity.clone(),
        headline: match risk.severity.as_str() {
            "low" => "Low timing risk".into(),
            "medium" => "Medium timing risk".into(),
            "high" => "High timing risk".into(),
            _ => risk.reason.clone(),
        },
        reasons: if risk.reason.is_empty() {
            Vec::new()
        } else {
            vec![risk.reason.clone()]
        },
        recommendations: risk.recommendations.clone(),
    }
}

fn song_detail(
    song_id: &str,
    song: &Song,
    schedule: &ScheduleMetadata,
    risk: &RiskReport,
) -> SongDetailDto {
    let recommendation = crate::commands::PlaybackRecommendationDto {
        recommended_hold_frames: risk.suggested_hold_frames,
        recommended_tempo_scale: risk.suggested_tempo_scale,
        summary: risk
            .recommendations
            .first()
            .cloned()
            .unwrap_or_else(|| "Keep the selected settings.".into()),
    };
    SongDetailDto {
        song_id: song_id.to_owned(),
        title: song.name.clone(),
        duration_us: schedule.source_duration_us,
        note_count: song.notes.len() as u64,
        format_label: "SHEET".into(),
        risk: risk_summary(risk),
        recommendation: Some(recommendation),
    }
}

fn compile_dispatch_schedule(
    schedule: &ScheduleMetadata,
) -> Result<sky_player::adapter_support::RuntimeSchedule, String> {
    let actions = schedule
        .actions
        .iter()
        .enumerate()
        .map(|(index, action)| KeyActionInput {
            source_action_index: index as u32,
            kind: match action.kind {
                ActionKind::Down => DispatchActionKind::Down,
                ActionKind::Up => DispatchActionKind::Up,
            },
            scheduled_us: action.at_us,
            scan_codes: action
                .scan_codes
                .iter()
                .copied()
                .collect::<SmallVec<[u16; 4]>>(),
            reason: Arc::<str>::from(action.reason.as_str()),
        })
        .collect::<Vec<_>>();
    compile_runtime_intents(&actions, &sky_app_core::song::SKY_SCAN_CODES)
        .map_err(|error| format!("native schedule compilation failed: {error}"))
}

impl NativePlaybackService {
    fn new() -> Self {
        Self {
            prepared: Mutex::new(HashMap::new()),
            active: Arc::new(Mutex::new(None)),
            last_terminal: Arc::new(Mutex::new(None)),
            diagnostics_enabled: Arc::new(AtomicBool::new(false)),
            diagnostics_sequence: Arc::new(AtomicU64::new(0)),
        }
    }

    fn set_diagnostics_enabled(
        &self,
        enabled: bool,
        events: Arc<Mutex<NativeEventHub>>,
    ) -> Result<(), String> {
        self.diagnostics_enabled.store(enabled, Ordering::Release);
        if enabled {
            self.publish_diagnostics_snapshot(&events, None)?;
        }
        Ok(())
    }

    fn prepare(
        &self,
        song_id: String,
        generation: u64,
        config: PlaybackConfigDto,
        song: Song,
        schedule: ScheduleMetadata,
        risk: RiskReport,
    ) -> Result<PreparedPlaybackDto, String> {
        let detail = song_detail(&song_id, &song, &schedule, &risk);
        if !config.dry_run && schedule.impossible_same_key_repeats > 0 {
            return Ok(PreparedPlaybackDto {
                prepared_id: None,
                song: detail,
                config,
                admission: PlaybackAdmission::Blocked,
                risk: risk_summary(&risk),
                decisions: Vec::new(),
                plan_fingerprint: None,
                variants: Vec::new(),
                error_code: Some("physical_infeasible".into()),
                error_message: Some("playback contains infeasible same-key repeats".into()),
            });
        }
        let fingerprint = plan_fingerprint(&song_id, &config, &schedule)?;
        let admission = if risk.severity == "low" {
            PlaybackAdmission::Ready
        } else {
            PlaybackAdmission::ConfirmationRequired
        };
        let prepared_id = opaque_native_id();
        let base_variant = NativePlaybackVariant {
            config: config.clone(),
            schedule: schedule.clone(),
            fingerprint: fingerprint.clone(),
        };
        let mut decisions = Vec::new();
        let mut variants = HashMap::from([(PlaybackDecision::Proceed, base_variant)]);
        let mut variant_dtos = vec![PlaybackPlanVariantDto {
            decision: PlaybackDecision::Proceed,
            config: config.clone(),
            plan_fingerprint: fingerprint.clone(),
        }];
        if risk.severity != "low" {
            decisions.push(RiskDecisionDto {
                decision: PlaybackDecision::Proceed,
                label: "Proceed with current settings".into(),
            });
            if let (Some(hold_frames), Some(tempo_scale)) =
                (risk.suggested_hold_frames, risk.suggested_tempo_scale)
                && (hold_frames != config.hold_frames || tempo_scale != config.tempo_scale)
                && let Ok(recommended_schedule) =
                    build_schedule(&song, hold_frames, tempo_scale, config.fps)
                && (config.dry_run || recommended_schedule.impossible_same_key_repeats == 0)
            {
                let recommended_config = PlaybackConfigDto {
                    hold_frames,
                    tempo_scale,
                    fps: config.fps,
                    dry_run: config.dry_run,
                };
                let recommended_fingerprint =
                    plan_fingerprint(&song_id, &recommended_config, &recommended_schedule)?;
                variants.insert(
                    PlaybackDecision::UseRecommended,
                    NativePlaybackVariant {
                        config: recommended_config.clone(),
                        schedule: recommended_schedule,
                        fingerprint: recommended_fingerprint.clone(),
                    },
                );
                variant_dtos.push(PlaybackPlanVariantDto {
                    decision: PlaybackDecision::UseRecommended,
                    config: recommended_config,
                    plan_fingerprint: recommended_fingerprint,
                });
                decisions.push(RiskDecisionDto {
                    decision: PlaybackDecision::UseRecommended,
                    label: "Use recommended settings".into(),
                });
            }
            if !config.dry_run {
                let dry_run_config = PlaybackConfigDto {
                    dry_run: true,
                    ..config.clone()
                };
                let dry_run_fingerprint = plan_fingerprint(&song_id, &dry_run_config, &schedule)?;
                variants.insert(
                    PlaybackDecision::DryRun,
                    NativePlaybackVariant {
                        config: dry_run_config.clone(),
                        schedule: schedule.clone(),
                        fingerprint: dry_run_fingerprint.clone(),
                    },
                );
                variant_dtos.push(PlaybackPlanVariantDto {
                    decision: PlaybackDecision::DryRun,
                    config: dry_run_config,
                    plan_fingerprint: dry_run_fingerprint,
                });
                decisions.push(RiskDecisionDto {
                    decision: PlaybackDecision::DryRun,
                    label: "Run a dry-run first".into(),
                });
            }
        }
        let dto = PreparedPlaybackDto {
            prepared_id: Some(prepared_id.clone()),
            song: detail,
            config,
            admission,
            risk: risk_summary(&risk),
            decisions,
            plan_fingerprint: Some(fingerprint.clone()),
            variants: variant_dtos,
            error_code: None,
            error_message: None,
        };
        let mut prepared = self
            .prepared
            .lock()
            .map_err(|_| "native prepared-plan lock poisoned".to_string())?;
        prepared.insert(
            prepared_id,
            NativePreparedPlan {
                song_id,
                generation,
                song,
                dto: dto.clone(),
                variants,
            },
        );
        while prepared.len() > MAX_PREPARED_PLANS {
            let oldest = prepared.keys().next().cloned();
            if let Some(oldest) = oldest {
                prepared.remove(&oldest);
            }
        }
        Ok(dto)
    }

    fn start(
        &self,
        request: PlaybackStartRequest,
        settings: &ApplicationSettings,
        events: Arc<Mutex<NativeEventHub>>,
    ) -> Result<PlaybackSessionDto, String> {
        let mut active_slot = self
            .active
            .lock()
            .map_err(|_| "native active-playback lock poisoned".to_string())?;
        if active_slot.is_some() {
            return Err("another playback session is active".into());
        }
        let mut prepared = self
            .prepared
            .lock()
            .map_err(|_| "native prepared-plan lock poisoned".to_string())?;
        let record = prepared
            .get(&request.prepared_id)
            .cloned()
            .ok_or_else(|| "prepared playback is stale or already consumed".to_string())?;
        let accepted = request
            .decisions
            .iter()
            .filter(|item| item.accepted)
            .collect::<Vec<_>>();
        let required = record
            .dto
            .decisions
            .iter()
            .map(|item| item.decision)
            .collect::<Vec<_>>();
        let selected = if record.dto.admission == PlaybackAdmission::Ready {
            if !request.decisions.is_empty() {
                return Err("ready playback accepts no risk decisions".into());
            }
            PlaybackDecision::Proceed
        } else {
            if accepted.len() != 1
                || request.decisions.len() != 1
                || !required.contains(&accepted[0].decision)
            {
                return Err("an exact risk decision is required".into());
            }
            accepted[0].decision
        };
        let variant = record
            .variants
            .get(&selected)
            .cloned()
            .ok_or_else(|| "selected risk decision has no prepared plan".to_string())?;
        let player = if variant.config.dry_run {
            None
        } else {
            Some(self.create_native_player(&variant.schedule, &variant.config, settings)?)
        };
        let session_id = opaque_native_id();
        let active = Arc::new(NativeActivePlayback {
            session_id: session_id.clone(),
            prepared_id: request.prepared_id.clone(),
            song_id: record.song_id.clone(),
            title: record.song.name.clone(),
            total_us: variant.schedule.duration_us,
            config: variant.config.clone(),
            plan_fingerprint: variant.fingerprint.clone(),
            physical: player.is_some(),
            state: Mutex::new(PlaybackSessionState::Starting),
            pending: Mutex::new(None),
            player,
            started_at: Instant::now(),
            paused_since: Mutex::new(None),
            paused_total: Mutex::new(Duration::ZERO),
            stop_requested: AtomicBool::new(false),
            skip_requested: AtomicBool::new(false),
            done: AtomicBool::new(false),
            sequence: AtomicU64::new(0),
        });
        prepared.remove(&request.prepared_id);
        *active_slot = Some(active.clone());
        drop(prepared);
        drop(active_slot);
        if let Err(error) =
            publish_playback_state(&events, &active, PlaybackEventState::Starting, None, None)
        {
            if let Some(player) = &active.player {
                let _ = player.panic_release();
                let _ = player.quit();
                let _ = player.join(Duration::from_secs(5));
            }
            if let Ok(mut slot) = self.active.lock()
                && slot
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &active))
            {
                *slot = None;
            }
            if let Ok(mut prepared) = self.prepared.lock() {
                prepared.insert(request.prepared_id, record);
            }
            return Err(error);
        }
        let service = Arc::new(self.clone_handle());
        let active_for_thread = active.clone();
        let spawn_result = thread::Builder::new()
            .name("sky-native-playback-supervisor".into())
            .spawn(move || service.monitor(active_for_thread, events));
        if let Err(error) = spawn_result {
            if let Some(player) = &active.player {
                let _ = player.panic_release();
                let _ = player.quit();
                let _ = player.join(Duration::from_secs(5));
            }
            if let Ok(mut slot) = self.active.lock()
                && slot
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &active))
            {
                *slot = None;
            }
            if let Ok(mut prepared) = self.prepared.lock() {
                prepared.insert(request.prepared_id, record);
            }
            return Err(format!(
                "failed to start native playback supervisor: {error}"
            ));
        }
        Ok(PlaybackSessionDto {
            session_id,
            prepared_id: active.prepared_id.clone(),
            song_id: active.song_id.clone(),
            state: PlaybackSessionState::Starting,
            config: active.config.clone(),
            plan_fingerprint: active.plan_fingerprint.clone(),
        })
    }

    fn clone_handle(&self) -> NativePlaybackServiceHandle {
        NativePlaybackServiceHandle {
            active: self.active.clone(),
            last_terminal: self.last_terminal.clone(),
            diagnostics_enabled: self.diagnostics_enabled.clone(),
            diagnostics_sequence: self.diagnostics_sequence.clone(),
        }
    }

    fn create_native_player(
        &self,
        schedule: &ScheduleMetadata,
        config: &PlaybackConfigDto,
        settings: &ApplicationSettings,
    ) -> Result<Arc<NativeDispatchSession>, String> {
        let runtime_schedule = compile_dispatch_schedule(schedule)?;
        let target = sky_dispatch_win32::focus::find_sky_window(
            &settings.sky_process_names,
            settings.allow_title_fallback,
        )
        .ok_or_else(|| "no admissible visible Sky window was found".to_string())?;
        if !sky_dispatch_win32::focus::focus_window(target) {
            return Err("validated Sky window could not be focused".into());
        }
        let frame = frame_us(config.fps).map_err(|error| error.to_string())?;
        let player = Arc::new(NativeDispatchSession::new(NativeSessionOptions {
            schedule: runtime_schedule,
            backend: BackendConfig::Production,
            profile: DispatchProfile::Production,
            timing: TimingOptions {
                game_fps: config.fps,
                min_hold_us: ((config.hold_frames * frame as f64).ceil() as u64)
                    .saturating_add(500)
                    .saturating_add(300),
                min_release_gap_us: frame.saturating_add(500).saturating_add(300),
                down_late_grace_us: 500,
                strict_timing: false,
                strict_down_completion_late_us: 2_000,
                strict_up_completion_late_us: 2_000,
                input_path_warn_us: 300,
            },
            focus: FocusOptions {
                require_focus: true,
                focus_restore_grace_us: 100_000,
            },
            wait: WaitOptions {
                enable_waitable_timer: true,
                enable_event_wait: true,
                supervisor_lease_timeout_us: 0,
                #[cfg(feature = "tauri-test")]
                test_spin_threshold_us: None,
                #[cfg(feature = "tauri-test")]
                test_wait_policy: sky_player::engine::TestWaitPolicy::LegacyTestWideSpin,
            },
            telemetry: TelemetryOptions {
                mode: TelemetryMode::Ring,
                capacity: 64,
            },
            priority: PriorityOptions {
                mode: PriorityMode::Auto,
            },
            #[cfg(feature = "tauri-test")]
            startup_ordering_hook: None,
            #[cfg(feature = "tauri-test")]
            restore_race_hook: None,
            #[cfg(feature = "tauri-test")]
            timer_lifecycle_context: None,
        })?);
        player.set_target_hwnd(target);
        player.set_focus_hint(true);
        player.arm(0)?;
        Ok(player)
    }

    fn monitor(&self, active: Arc<NativeActivePlayback>, events: Arc<Mutex<NativeEventHub>>) {
        let mut last_snapshot = Instant::now();
        let mut last_event_state = PlaybackEventState::Starting;
        loop {
            if active.stop_requested.load(Ordering::Acquire) {
                if let Some(player) = &active.player {
                    let _ = player.quit();
                    let _ = player.join(Duration::from_secs(5));
                }
                let _ = set_playback_state(&active, PlaybackSessionState::Finished);
                let outcome = if active.skip_requested.load(Ordering::Acquire) {
                    "skipped"
                } else {
                    "quit"
                };
                if publish_playback_finished(&events, &active, outcome, "Playback stopped").is_err()
                {
                    cleanup_failed_event_delivery(&active);
                }
                break;
            }
            let (elapsed, pre_roll_remaining, paused, finished, status) =
                if let Some(player) = &active.player {
                    let _ = player.heartbeat();
                    let state = player.poll_state();
                    (
                        state.elapsed_us,
                        state.pre_roll_remaining_us,
                        state.is_paused,
                        state.is_finished,
                        state.status.as_str(),
                    )
                } else {
                    let elapsed = dry_run_elapsed(&active);
                    let paused = active
                        .state
                        .lock()
                        .map(|state| *state == PlaybackSessionState::Paused)
                        .unwrap_or(false);
                    (
                        elapsed,
                        0,
                        paused,
                        elapsed >= active.total_us,
                        if paused { "paused" } else { "playing" },
                    )
                };
            let event_state = if paused {
                PlaybackEventState::Paused
            } else if status == EnginePollStatus::Playing.as_str() {
                PlaybackEventState::Playing
            } else if matches!(
                status,
                "error" | "panicked" | "poisoned" | "invalid" | "quit"
            ) {
                PlaybackEventState::Failed
            } else {
                PlaybackEventState::Starting
            };
            if event_state != last_event_state {
                let state = match event_state {
                    PlaybackEventState::Starting => PlaybackSessionState::Starting,
                    PlaybackEventState::Playing => PlaybackSessionState::Playing,
                    PlaybackEventState::Paused => PlaybackSessionState::Paused,
                    PlaybackEventState::Stopping => PlaybackSessionState::Stopping,
                    PlaybackEventState::Finished => PlaybackSessionState::Finished,
                    PlaybackEventState::Failed => PlaybackSessionState::Failed,
                };
                let _ = set_playback_state(&active, state);
                if matches!(
                    (
                        event_state,
                        active.pending.lock().ok().and_then(|pending| *pending)
                    ),
                    (
                        PlaybackEventState::Paused,
                        Some(PlaybackPendingControl::Pause)
                    ) | (
                        PlaybackEventState::Playing,
                        Some(PlaybackPendingControl::Resume)
                    )
                ) && let Ok(mut pending) = active.pending.lock()
                {
                    *pending = None;
                }
                if publish_playback_state(&events, &active, event_state, None, None).is_err() {
                    cleanup_failed_event_delivery(&active);
                    break;
                }
                last_event_state = event_state;
            }
            if last_snapshot.elapsed() >= Duration::from_millis(100) {
                if publish_playback_snapshot(&events, &active, elapsed, pre_roll_remaining, paused)
                    .is_err()
                {
                    cleanup_failed_event_delivery(&active);
                    break;
                }
                if self.diagnostics_enabled.load(Ordering::Acquire)
                    && publish_diagnostics_snapshot_for_active(
                        &events,
                        &active,
                        &self.diagnostics_sequence,
                    )
                    .is_err()
                {
                    cleanup_failed_event_delivery(&active);
                    break;
                }
                last_snapshot = Instant::now();
            }
            if finished {
                let outcome = if status == EnginePollStatus::Skipped.as_str()
                    || active.skip_requested.load(Ordering::Acquire)
                {
                    "skipped"
                } else {
                    "finished"
                };
                if matches!(status, "error" | "panicked" | "poisoned" | "invalid") {
                    let _ = set_playback_state(&active, PlaybackSessionState::Failed);
                    if publish_playback_failed(
                        &events,
                        &active,
                        "native_player_failed",
                        "Native playback worker failed",
                    )
                    .is_err()
                    {
                        cleanup_failed_event_delivery(&active);
                    }
                } else {
                    let _ = set_playback_state(&active, PlaybackSessionState::Finished);
                    if publish_playback_finished(&events, &active, outcome, "Playback finished")
                        .is_err()
                    {
                        cleanup_failed_event_delivery(&active);
                    }
                }
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        if let Ok(mut slot) = self.active.lock()
            && slot
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &active))
        {
            *slot = None;
        }
        let terminal_state = active
            .state
            .lock()
            .map(|state| *state)
            .unwrap_or(PlaybackSessionState::Failed);
        if let Ok(mut terminal) = self.last_terminal.lock() {
            *terminal = Some((active.session_id.clone(), terminal_state));
        }
        active.done.store(true, Ordering::Release);
    }

    fn command(
        &self,
        method: &str,
        session_id: String,
        events: Arc<Mutex<NativeEventHub>>,
    ) -> Result<PlaybackCommandAckDto, String> {
        let active = self
            .active
            .lock()
            .map_err(|_| "native active-playback lock poisoned".to_string())?
            .clone();
        let Some(active) = active else {
            if method == "playback.stop"
                && self
                    .last_terminal
                    .lock()
                    .ok()
                    .and_then(|value| value.clone())
                    .is_some_and(|(id, _)| id == session_id)
            {
                return Ok(PlaybackCommandAckDto {
                    accepted: true,
                    session_id,
                    state: PlaybackSessionState::Finished,
                    pending_command: None,
                    reason: None,
                });
            }
            return Err("there is no active playback session".into());
        };
        if active.session_id != session_id {
            return Err("session_id is stale or foreign".into());
        }
        let current = *active
            .state
            .lock()
            .map_err(|_| "native playback state lock poisoned".to_string())?;
        match method {
            "playback.stop" => {
                if !matches!(
                    current,
                    PlaybackSessionState::Starting
                        | PlaybackSessionState::Playing
                        | PlaybackSessionState::Paused
                ) {
                    return Ok(PlaybackCommandAckDto {
                        accepted: true,
                        session_id,
                        state: current,
                        pending_command: *active
                            .pending
                            .lock()
                            .map_err(|_| "native playback control lock poisoned".to_string())?,
                        reason: None,
                    });
                }
                active.stop_requested.store(true, Ordering::Release);
                *active
                    .pending
                    .lock()
                    .map_err(|_| "native playback control lock poisoned".to_string())? = None;
                let _ = set_playback_state(&active, PlaybackSessionState::Stopping);
                if let Some(player) = &active.player {
                    player.quit()?;
                }
                publish_playback_state(&events, &active, PlaybackEventState::Stopping, None, None)?;
            }
            "playback.pause" => {
                if current != PlaybackSessionState::Playing {
                    return Err("pause requires a playing session".into());
                }
                let mut pending = active
                    .pending
                    .lock()
                    .map_err(|_| "native playback control lock poisoned".to_string())?;
                if *pending == Some(PlaybackPendingControl::Pause) {
                    return Ok(PlaybackCommandAckDto {
                        accepted: true,
                        session_id,
                        state: current,
                        pending_command: *pending,
                        reason: Some("already_pending".into()),
                    });
                }
                if pending.is_some() {
                    return Err("another playback control is awaiting acknowledgement".into());
                }
                *pending = Some(PlaybackPendingControl::Pause);
                drop(pending);
                if let Some(player) = &active.player {
                    if let Err(error) = player.pause() {
                        if let Ok(mut pending) = active.pending.lock() {
                            *pending = None;
                        }
                        return Err(error.to_string());
                    }
                } else {
                    *active
                        .paused_since
                        .lock()
                        .map_err(|_| "native playback pause lock poisoned".to_string())? =
                        Some(Instant::now());
                    set_playback_state(&active, PlaybackSessionState::Paused)?;
                    publish_playback_state(
                        &events,
                        &active,
                        PlaybackEventState::Paused,
                        None,
                        None,
                    )?;
                }
            }
            "playback.resume" => {
                if current != PlaybackSessionState::Paused {
                    return Err("resume requires a paused session".into());
                }
                let mut pending = active
                    .pending
                    .lock()
                    .map_err(|_| "native playback control lock poisoned".to_string())?;
                if *pending == Some(PlaybackPendingControl::Resume) {
                    return Ok(PlaybackCommandAckDto {
                        accepted: true,
                        session_id,
                        state: current,
                        pending_command: *pending,
                        reason: Some("already_pending".into()),
                    });
                }
                if pending.is_some() {
                    return Err("another playback control is awaiting acknowledgement".into());
                }
                *pending = Some(PlaybackPendingControl::Resume);
                drop(pending);
                if let Some(player) = &active.player {
                    if let Err(error) = player.resume() {
                        if let Ok(mut pending) = active.pending.lock() {
                            *pending = None;
                        }
                        return Err(error.to_string());
                    }
                } else {
                    let now = Instant::now();
                    let paused_since = active
                        .paused_since
                        .lock()
                        .map_err(|_| "native playback pause lock poisoned".to_string())?
                        .take();
                    if let Some(paused_since) = paused_since {
                        *active
                            .paused_total
                            .lock()
                            .map_err(|_| "native playback pause lock poisoned".to_string())? +=
                            now.saturating_duration_since(paused_since);
                    }
                    set_playback_state(&active, PlaybackSessionState::Playing)?;
                    publish_playback_state(
                        &events,
                        &active,
                        PlaybackEventState::Playing,
                        None,
                        None,
                    )?;
                }
            }
            "playback.skip" => {
                active.skip_requested.store(true, Ordering::Release);
                if let Some(player) = &active.player {
                    player.skip()?;
                } else {
                    active.stop_requested.store(true, Ordering::Release);
                }
            }
            _ => return Err(format!("unsupported native playback command: {method}")),
        }
        Ok(PlaybackCommandAckDto {
            accepted: true,
            session_id,
            state: *active
                .state
                .lock()
                .map_err(|_| "native playback state lock poisoned".to_string())?,
            pending_command: *active
                .pending
                .lock()
                .map_err(|_| "native playback control lock poisoned".to_string())?,
            reason: None,
        })
    }

    fn invalidate_catalog(&self, generation: u64) {
        if let Ok(mut prepared) = self.prepared.lock() {
            prepared.retain(|_, record| record.generation == generation);
        }
    }

    fn invalidate_settings(&self) {
        if let Ok(mut prepared) = self.prepared.lock() {
            prepared.clear();
        }
    }

    fn publish_diagnostics_snapshot(
        &self,
        events: &Arc<Mutex<NativeEventHub>>,
        active: Option<Arc<NativeActivePlayback>>,
    ) -> Result<(), String> {
        if let Some(active) =
            active.or_else(|| self.active.lock().ok().and_then(|slot| slot.clone()))
            && active.player.is_some()
        {
            return publish_diagnostics_snapshot_for_active(
                events,
                &active,
                &self.diagnostics_sequence,
            );
        }
        publish_empty_diagnostics_snapshot(events, &self.diagnostics_sequence)
    }

    fn shutdown(&self, events: Arc<Mutex<NativeEventHub>>) {
        let active = self.active.lock().ok().and_then(|slot| slot.clone());
        if let Some(active) = active {
            active.stop_requested.store(true, Ordering::Release);
            if let Some(player) = &active.player {
                let _ = player.panic_release();
                let _ = player.quit();
                let _ = player.join(Duration::from_secs(5));
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            while !active.done.load(Ordering::Acquire) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            let _ = publish_playback_state(
                &events,
                &active,
                PlaybackEventState::Finished,
                Some("shutdown".into()),
                Some("quit".into()),
            );
        }
    }
}

#[derive(Clone)]
struct NativePlaybackServiceHandle {
    active: Arc<Mutex<Option<Arc<NativeActivePlayback>>>>,
    last_terminal: Arc<Mutex<Option<(String, PlaybackSessionState)>>>,
    diagnostics_enabled: Arc<AtomicBool>,
    diagnostics_sequence: Arc<AtomicU64>,
}

impl NativePlaybackServiceHandle {
    fn monitor(&self, active: Arc<NativeActivePlayback>, events: Arc<Mutex<NativeEventHub>>) {
        let service = NativePlaybackService {
            prepared: Mutex::new(HashMap::new()),
            active: self.active.clone(),
            last_terminal: self.last_terminal.clone(),
            diagnostics_enabled: self.diagnostics_enabled.clone(),
            diagnostics_sequence: self.diagnostics_sequence.clone(),
        };
        service.monitor(active, events);
    }
}

fn publish_diagnostics_snapshot_for_active(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    sequence: &AtomicU64,
) -> Result<(), String> {
    let Some(player) = &active.player else {
        return Ok(());
    };
    let snapshot = player.snapshot_lite();
    let recent = snapshot.recent_latencies_us.as_slice();
    let payload = crate::ui_events::DiagnosticsSnapshotDto {
        seq: sequence.fetch_add(1, Ordering::Relaxed) + 1,
        max_lateness_us: snapshot.max_lateness_us,
        p50_ms: percentile_ms(recent, 0.50),
        p95_ms: percentile_ms(recent, 0.95),
        sigma_onset_ms: population_sigma_ms(recent),
        late_2ms: snapshot.late_2ms,
        late_5ms: snapshot.late_5ms,
        late_10ms: snapshot.late_10ms,
        active_keys: snapshot.active_count as u64,
        stuck_keys: snapshot.possibly_active_count as u64,
        keys_dropped: snapshot.keys_dropped,
        chord_split_events: snapshot.chord_split_events,
        backend_status: if snapshot.has_terminal_error {
            crate::ui_events::DiagnosticsBackendStatus::Error
        } else {
            crate::ui_events::DiagnosticsBackendStatus::Healthy
        },
        release_max_us: (snapshot.release_max_us > 0).then_some(snapshot.release_max_us),
        release_late_2ms: (snapshot.release_late_2ms > 0).then_some(snapshot.release_late_2ms),
        session_id: Some(active.session_id.clone()),
    };
    events
        .lock()
        .map_err(|_| "native event hub lock poisoned".to_string())?
        .publish(UiEvent::DiagnosticsSnapshot {
            v: crate::core::protocol::DESKTOP_PROTOCOL_VERSION,
            payload,
        })
}

fn publish_empty_diagnostics_snapshot(
    events: &Arc<Mutex<NativeEventHub>>,
    sequence: &AtomicU64,
) -> Result<(), String> {
    events
        .lock()
        .map_err(|_| "native event hub lock poisoned".to_string())?
        .publish(UiEvent::DiagnosticsSnapshot {
            v: crate::core::protocol::DESKTOP_PROTOCOL_VERSION,
            payload: crate::ui_events::DiagnosticsSnapshotDto {
                seq: sequence.fetch_add(1, Ordering::Relaxed) + 1,
                max_lateness_us: 0,
                p50_ms: 0.0,
                p95_ms: 0.0,
                sigma_onset_ms: 0.0,
                late_2ms: 0,
                late_5ms: 0,
                late_10ms: 0,
                active_keys: 0,
                stuck_keys: 0,
                keys_dropped: 0,
                chord_split_events: 0,
                backend_status: crate::ui_events::DiagnosticsBackendStatus::Unavailable,
                release_max_us: None,
                release_late_2ms: None,
                session_id: None,
            },
        })
}

fn cleanup_failed_event_delivery(active: &NativeActivePlayback) {
    active.stop_requested.store(true, Ordering::Release);
    let _ = set_playback_state(active, PlaybackSessionState::Failed);
    if let Some(player) = &active.player {
        let _ = player.panic_release();
        let _ = player.quit();
        let _ = player.join(Duration::from_secs(5));
    }
}

fn set_playback_state(
    active: &NativeActivePlayback,
    state: PlaybackSessionState,
) -> Result<(), String> {
    *active
        .state
        .lock()
        .map_err(|_| "native playback state lock poisoned".to_string())? = state;
    Ok(())
}

fn dry_run_elapsed(active: &NativeActivePlayback) -> u64 {
    let paused_total = active
        .paused_total
        .lock()
        .map(|value| *value)
        .unwrap_or_default();
    let current_pause = active
        .paused_since
        .lock()
        .ok()
        .and_then(|value| *value)
        .map(|value| Instant::now().saturating_duration_since(value))
        .unwrap_or_default();
    active
        .started_at
        .elapsed()
        .saturating_sub(paused_total)
        .saturating_sub(current_pause)
        .as_micros()
        .min(u128::from(active.total_us)) as u64
}

fn publish_playback_state(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    state: PlaybackEventState,
    message: Option<String>,
    outcome: Option<String>,
) -> Result<(), String> {
    let event = UiEvent::PlaybackStateChanged {
        v: crate::core::protocol::DESKTOP_PROTOCOL_VERSION,
        payload: PlaybackStateChangedPayload {
            session_id: active.session_id.clone(),
            song_id: active.song_id.clone(),
            state,
            physical: active.physical,
            message,
            outcome,
        },
    };
    events
        .lock()
        .map_err(|_| "native event hub lock poisoned".to_string())?
        .publish(event)
}

fn publish_playback_snapshot(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    elapsed_us: u64,
    pre_roll_remaining_us: u64,
    paused: bool,
) -> Result<(), String> {
    let event = UiEvent::PlaybackSnapshot {
        v: crate::core::protocol::DESKTOP_PROTOCOL_VERSION,
        payload: PlaybackSnapshotPayload {
            session_id: active.session_id.clone(),
            seq: active.sequence.fetch_add(1, Ordering::Relaxed) + 1,
            state: if paused {
                PlaybackEventState::Paused
            } else if pre_roll_remaining_us > 0 {
                PlaybackEventState::Starting
            } else {
                PlaybackEventState::Playing
            },
            song_id: active.song_id.clone(),
            title: active.title.clone(),
            current_us: elapsed_us,
            total_us: active.total_us,
            pre_roll_remaining_us,
            focus_state: PlaybackFocusState::Focused,
            health: PlaybackHealthState::Healthy,
            input_path_degraded: false,
            message: None,
        },
    };
    events
        .lock()
        .map_err(|_| "native event hub lock poisoned".to_string())?
        .publish(event)
}

fn publish_playback_failed(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    code: &str,
    message: &str,
) -> Result<(), String> {
    events
        .lock()
        .map_err(|_| "native event hub lock poisoned".to_string())?
        .publish(UiEvent::PlaybackFailed {
            v: crate::core::protocol::DESKTOP_PROTOCOL_VERSION,
            payload: PlaybackFailedPayload {
                session_id: active.session_id.clone(),
                song_id: active.song_id.clone(),
                code: code.into(),
                message: message.into(),
            },
        })
}

fn publish_playback_finished(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    outcome: &str,
    message: &str,
) -> Result<(), String> {
    events
        .lock()
        .map_err(|_| "native event hub lock poisoned".to_string())?
        .publish(UiEvent::PlaybackFinished {
            v: crate::core::protocol::DESKTOP_PROTOCOL_VERSION,
            payload: PlaybackFinishedPayload {
                session_id: active.session_id.clone(),
                song_id: active.song_id.clone(),
                outcome: outcome.into(),
                total_us: active.total_us,
                message: message.into(),
            },
        })
}

fn resolve_install_root() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os("SKY_INSTALL_ROOT") {
        return fs::canonicalize(value)
            .map_err(|error| format!("invalid SKY_INSTALL_ROOT: {error}"));
    }
    if cfg!(debug_assertions) {
        let root = std::env::var_os("SKY_DESKTOP_REPOSITORY_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..\\.."));
        return fs::canonicalize(root)
            .map_err(|error| format!("invalid debug repository root: {error}"));
    }
    let executable =
        std::env::current_exe().map_err(|error| format!("cannot resolve executable: {error}"))?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "executable has no install root".into())
}

fn native_build_dto() -> crate::commands::NativeBuildDto {
    crate::commands::NativeBuildDto {
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

fn playback_defaults(settings: &ApplicationSettings) -> PlaybackDefaultsDto {
    PlaybackDefaultsDto {
        hold_frames: settings.playback_defaults.hold_frames,
        tempo_scale: settings.playback_defaults.tempo_scale,
        fps: settings.playback_defaults.fps,
        dry_run: false,
    }
}

fn update_preferences_dto(settings: &ApplicationSettings) -> UpdatePreferencesDto {
    UpdatePreferencesDto {
        auto_check: settings.update.auto_check,
        channel: match settings.update.channel {
            sky_app_core::settings::UpdateChannel::Stable => {
                crate::ui_events::UpdateChannel::Stable
            }
            sky_app_core::settings::UpdateChannel::Beta => crate::ui_events::UpdateChannel::Beta,
        },
        skip_version: settings.update.skip_version.clone(),
    }
}

fn settings_dto(settings: &ApplicationSettings) -> SettingsDto {
    SettingsDto {
        theme: settings.theme.clone(),
        ui_background_mode: settings.ui_background_mode.clone(),
        playback_defaults: playback_defaults(settings),
        telemetry_enabled: settings.telemetry_enabled,
        verbose_hud: settings.verbose_hud,
        update_preferences: update_preferences_dto(settings),
    }
}

fn json_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeCatalogDetailRequest {
    song_id: String,
    generation: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeCatalogViewportRequest {
    generation: u64,
    first_index: u64,
    last_index: i64,
    selected_song_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativePlaybackPatch {
    hold_frames: Option<f64>,
    tempo_scale: Option<f64>,
    fps: Option<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeUpdatePreferencesPatch {
    auto_check: Option<bool>,
    channel: Option<crate::ui_events::UpdateChannel>,
    skip_version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeSettingsPatch {
    theme: Option<String>,
    telemetry_enabled: Option<bool>,
    verbose_hud: Option<bool>,
    playback_defaults: Option<NativePlaybackPatch>,
    update_preferences: Option<NativeUpdatePreferencesPatch>,
}

impl NativeUpdatePreferencesPatch {
    fn into_public(self) -> UpdatePreferencesPatch {
        UpdatePreferencesPatch {
            auto_check: self.auto_check,
            channel: self.channel,
            skip_version: self.skip_version,
        }
    }
}

impl NativeSettingsPatch {
    fn into_public(self) -> SettingsPatch {
        SettingsPatch {
            theme: self.theme,
            telemetry_enabled: self.telemetry_enabled,
            verbose_hud: self.verbose_hud,
            playback_defaults: self
                .playback_defaults
                .map(|value| crate::commands::PlaybackPatch {
                    hold_frames: value.hold_frames,
                    tempo_scale: value.tempo_scale,
                    fps: value.fps,
                }),
            update_preferences: self.update_preferences.map(|value| value.into_public()),
        }
    }
}

fn encode_result<T: serde::Serialize>(result: Result<T, String>) -> Result<Value, String> {
    result
        .map_err(|error| error.to_string())
        .and_then(|value| serde_json::to_value(value).map_err(json_error))
}
fn settings_error(error: SettingsError) -> String {
    error.to_string()
}
fn catalog_error(error: CatalogError) -> String {
    error.to_string()
}

#[derive(Default)]
struct NativeEventHub {
    buffered: VecDeque<UiEvent>,
    channel: Option<Channel<UiEvent>>,
    closed: bool,
}

impl NativeEventHub {
    fn publish(&mut self, event: UiEvent) -> Result<(), String> {
        if self.closed {
            return Err("native event hub is closed".into());
        }
        validate_ui_event(&event)?;
        if let Some(channel) = &self.channel {
            if let Err(error) = channel.send(event) {
                // A failed delivery is not a queueing opportunity: buffering
                // after a subscriber failure would hide the loss of the
                // delivery contract and let a native worker continue without
                // a consumer.  Close the hub and let the owner perform its
                // bounded cleanup path.
                self.channel = None;
                self.closed = true;
                self.buffered.clear();
                return Err(format!("native UI event delivery failed: {error}"));
            }
            return Ok(());
        }
        if let Some(key) = snapshot_key(&event) {
            if let Some(existing) = self
                .buffered
                .iter_mut()
                .find(|candidate| snapshot_key(candidate).as_ref() == Some(&key))
            {
                *existing = event;
                return Ok(());
            }
            if self.buffered.len() >= MAX_NATIVE_EVENTS
                && !remove_oldest_snapshot(&mut self.buffered)
            {
                return Err("native event hub lifecycle buffer overflow".into());
            }
        } else if self.buffered.len() >= MAX_NATIVE_EVENTS
            && !remove_oldest_snapshot(&mut self.buffered)
        {
            // Lifecycle events are never silently discarded.  The caller must
            // initiate bounded cleanup when this fail-closed signal occurs.
            return Err("native event hub lifecycle buffer overflow".into());
        }
        self.buffered.push_back(event);
        Ok(())
    }

    fn subscribe(&mut self, channel: Channel<UiEvent>) -> Result<(), String> {
        if self.closed {
            return Err("native event hub is closed".into());
        }
        for event in &self.buffered {
            if let Err(error) = channel.send(event.clone()) {
                self.closed = true;
                self.buffered.clear();
                return Err(format!("native UI event replay failed: {error}"));
            }
        }
        self.buffered.clear();
        self.channel = Some(channel);
        Ok(())
    }

    fn close(&mut self) {
        self.closed = true;
        self.channel = None;
        self.buffered.clear();
    }
}

fn validate_ui_event(event: &UiEvent) -> Result<(), String> {
    match event {
        UiEvent::CoreReady { payload, .. } => UiEvent::validate_ready(payload),
        UiEvent::CoreFatal { payload, .. } => UiEvent::validate_fatal(payload),
        UiEvent::CatalogChanged { payload, .. } => UiEvent::validate_catalog_changed(payload),
        UiEvent::PlaybackStateChanged { payload, .. } => {
            UiEvent::validate_playback_state_changed(payload)
        }
        UiEvent::PlaybackSnapshot { payload, .. } => UiEvent::validate_playback_snapshot(payload),
        UiEvent::PlaybackFinished { payload, .. } => UiEvent::validate_playback_finished(payload),
        UiEvent::PlaybackFailed { payload, .. } => UiEvent::validate_playback_failed(payload),
        UiEvent::DiagnosticsSnapshot { payload, .. } => {
            UiEvent::validate_diagnostics_snapshot(payload)
        }
        UiEvent::CalibrationProgress { payload, .. } => {
            UiEvent::validate_calibration_progress(payload)
        }
        UiEvent::CalibrationFinished { payload, .. } => {
            UiEvent::validate_calibration_finished(payload)
        }
        UiEvent::UpdateAvailable { payload, .. } => UiEvent::validate_update_available(payload),
        UiEvent::UpdateResult { payload, .. } => UiEvent::validate_update_result(payload),
        UiEvent::UpdateHandoffReady { payload, .. } => UiEvent::validate_update_handoff(payload),
    }
}

fn snapshot_key(event: &UiEvent) -> Option<(u8, String)> {
    match event {
        UiEvent::PlaybackSnapshot { payload, .. } => Some((1, payload.session_id.clone())),
        UiEvent::DiagnosticsSnapshot { payload, .. } => {
            Some((2, payload.session_id.clone().unwrap_or_default()))
        }
        UiEvent::CalibrationProgress { payload, .. } => Some((3, payload.operation_id.clone())),
        _ => None,
    }
}

/// Match the Python diagnostics percentile contract.  Python's ``round``
/// uses ties-to-even, so using `f64::round()` here would diverge for small
/// sample sets at an exact half index.
fn percentile_ms(values: &[i64], fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    let position = fraction * (ordered.len() - 1) as f64;
    let lower = position.floor();
    let fractional = position - lower;
    let index = if fractional < 0.5 {
        lower as usize
    } else if fractional > 0.5 || (lower as usize) % 2 == 1 {
        (lower as usize + 1).min(ordered.len() - 1)
    } else {
        lower as usize
    };
    ordered[index] as f64 / 1000.0
}

fn population_sigma_ms(values: &[i64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mean = values.iter().map(|value| *value as f64).sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| {
            let delta = *value as f64 - mean;
            delta * delta
        })
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt() / 1000.0
}

fn remove_oldest_snapshot(buffered: &mut VecDeque<UiEvent>) -> bool {
    let Some(index) = buffered
        .iter()
        .position(|event| snapshot_key(event).is_some())
    else {
        return false;
    };
    buffered.remove(index).is_some()
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_NATIVE_EVENTS, NativeDesktopRuntime, NativeEventHub, percentile_ms,
        population_sigma_ms, resolve_install_root,
    };
    use crate::ui_events::{
        PlaybackEventState, PlaybackFocusState, PlaybackHealthState, PlaybackSnapshotPayload,
        UiEvent,
    };
    use serde_json::Value;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn debug_install_root_is_repository_root_not_cwd() {
        if cfg!(debug_assertions) {
            let root = resolve_install_root().expect("root");
            assert!(root.join("rust").is_dir());
            assert!(root.join("desktop").is_dir());
        }
    }

    #[test]
    fn native_bootstrap_uses_explicit_install_root_and_returns_plain_dto() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-native-runtime-{suffix}"));
        fs::create_dir_all(root.join("songs")).expect("root");
        fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
        let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
        let value = runtime
            .dispatch("app.bootstrap", Value::Object(Default::default()))
            .expect("bootstrap");
        assert!(value.get("app_version").is_some());
        assert!(value.get("Ok").is_none());
        assert_eq!(runtime.install_root(), root.as_path());
        assert!(
            runtime
                .dispatch("settings.get", Value::Object(Default::default()))
                .is_err()
        );
        let _ = fs::remove_dir_all(root);
    }

    fn snapshot(session_id: &str, seq: u64) -> UiEvent {
        UiEvent::PlaybackSnapshot {
            v: 1,
            payload: PlaybackSnapshotPayload {
                session_id: session_id.into(),
                seq,
                state: PlaybackEventState::Playing,
                song_id: "0123456789abcdef0123456789abcdef".into(),
                title: "demo".into(),
                current_us: seq,
                total_us: 100,
                pre_roll_remaining_us: 0,
                focus_state: PlaybackFocusState::Focused,
                health: PlaybackHealthState::Healthy,
                input_path_degraded: false,
                message: None,
            },
        }
    }

    #[test]
    fn event_hub_coalesces_snapshots_and_fails_closed_for_lifecycle_overflow() {
        let mut hub = NativeEventHub::default();
        let session_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        hub.publish(snapshot(session_id, 1)).expect("snapshot");
        hub.publish(snapshot(session_id, 2)).expect("coalesce");
        assert_eq!(hub.buffered.len(), 1);
        assert!(
            matches!(hub.buffered.front(), Some(UiEvent::PlaybackSnapshot { payload, .. }) if payload.seq == 2)
        );

        for index in 0..MAX_NATIVE_EVENTS {
            hub.publish(UiEvent::CatalogChanged {
                v: 1,
                payload: crate::ui_events::CatalogChangedPayload {
                    generation: index as u64 + 1,
                    total: 0,
                },
            })
            .expect("snapshot slot can be evicted before lifecycle fill");
        }
        assert!(
            hub.publish(UiEvent::CatalogChanged {
                v: 1,
                payload: crate::ui_events::CatalogChangedPayload {
                    generation: 999,
                    total: 0,
                },
            })
            .is_err()
        );
    }

    #[test]
    fn diagnostics_statistics_match_python_rounding_and_population_sigma() {
        assert_eq!(percentile_ms(&[1, 2, 3, 4], 0.50), 0.003);
        assert_eq!(percentile_ms(&[1, 2, 3, 4], 0.95), 0.004);
        assert_eq!(
            population_sigma_ms(&[1, 2, 3, 4]),
            1.118033988749895 / 1000.0
        );
        assert_eq!(percentile_ms(&[], 0.50), 0.0);
        assert_eq!(population_sigma_ms(&[]), 0.0);
    }

    #[test]
    fn settings_and_catalog_changes_invalidate_prepared_native_playback() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-native-state-{suffix}"));
        fs::create_dir_all(root.join("songs")).expect("songs root");
        fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
        fs::write(
            root.join("songs/demo.json"),
            r#"{"name":"Demo","songNotes":[{"time":0,"key":"Key0"}]}"#,
        )
        .expect("song");
        let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
        let bootstrap = runtime
            .dispatch("app.bootstrap", Value::Object(Default::default()))
            .expect("bootstrap");
        let generation = bootstrap["catalog_generation"]
            .as_u64()
            .expect("generation");
        let search = runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({"query":"demo","offset":0,"limit":10,"generation":generation}),
            )
            .expect("search");
        let song_id = search["items"][0]["song_id"]
            .as_str()
            .expect("song ID")
            .to_owned();
        let prepared = runtime
            .dispatch(
                "playback.prepare",
                serde_json::json!({
                    "song_id": song_id,
                    "generation": generation,
                    "config": {"hold_frames":1.0,"tempo_scale":1.0,"fps":60,"dry_run":true}
                }),
            )
            .expect("prepare");
        let prepared_id = prepared["prepared_id"]
            .as_str()
            .expect("prepared ID")
            .to_owned();
        runtime
            .patch_settings(crate::commands::SettingsPatch {
                theme: None,
                telemetry_enabled: None,
                verbose_hud: None,
                playback_defaults: Some(crate::commands::PlaybackPatch {
                    hold_frames: None,
                    tempo_scale: Some(0.95),
                    fps: None,
                }),
                update_preferences: None,
            })
            .expect("native settings invalidation seam");
        assert!(
            runtime
                .dispatch(
                    "playback.start",
                    serde_json::json!({"prepared_id":prepared_id,"decisions":[]}),
                )
                .is_err()
        );

        let prepared = runtime
            .dispatch(
                "playback.prepare",
                serde_json::json!({
                    "song_id": song_id,
                    "generation": generation,
                    "config": {"hold_frames":1.0,"tempo_scale":1.0,"fps":60,"dry_run":true}
                }),
            )
            .expect("prepare after settings patch");
        let prepared_id = prepared["prepared_id"]
            .as_str()
            .expect("prepared ID")
            .to_owned();
        runtime
            .dispatch("catalog.reload", Value::Object(Default::default()))
            .expect("reload");
        assert!(
            runtime
                .dispatch(
                    "playback.start",
                    serde_json::json!({"prepared_id":prepared_id,"decisions":[]}),
                )
                .is_err()
        );
        runtime.shutdown();
        let _ = fs::remove_dir_all(root);
    }
}
