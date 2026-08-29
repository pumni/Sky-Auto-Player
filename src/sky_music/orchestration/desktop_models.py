"""Immutable DTOs shared by the future desktop adapter and Python services."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

RiskLevel = Literal["low", "medium", "high", "unknown"]
MetadataState = Literal["pending", "ready", "error"]
Admission = Literal["ready", "confirmation_required", "blocked"]
PlaybackState = Literal[
    "preparing",
    "countdown",
    "playing",
    "paused",
    "focus_lost",
    "stopping",
    "finished",
    "error",
]
FocusState = Literal["focused", "unfocused", "waiting"]
HealthState = Literal["healthy", "degraded", "error"]


@dataclass(frozen=True, slots=True)
class PlaybackConfigDto:
    hold_frames: float
    tempo_scale: float
    fps: int
    dry_run: bool


@dataclass(frozen=True, slots=True)
class NativeBuildDto:
    native_build_commit: str
    native_version: str
    schema_version: int
    native_abi: str
    rustc_version: str
    win32_backend: bool


@dataclass(frozen=True, slots=True)
class PlaybackOptionSetsDto:
    hold_frames: tuple[float, ...]
    tempo_scales: tuple[float, ...]
    fps: tuple[int, ...]


@dataclass(frozen=True, slots=True)
class UpdatePreferencesDto:
    auto_check: bool
    channel: Literal["stable", "beta"]
    skip_version: str


@dataclass(frozen=True, slots=True)
class BootstrapDto:
    app_version: str
    protocol_version: int
    native_build: NativeBuildDto
    playback_defaults: PlaybackConfigDto
    option_sets: PlaybackOptionSetsDto
    theme: str
    telemetry_enabled: bool
    update_preferences: UpdatePreferencesDto
    catalog_generation: int


@dataclass(frozen=True, slots=True)
class SongRowDto:
    song_id: str
    title: str
    duration_us: int | None
    note_count: int | None
    risk_level: RiskLevel
    metadata_state: MetadataState


@dataclass(frozen=True, slots=True)
class RiskSummaryDto:
    level: RiskLevel
    headline: str
    reasons: tuple[str, ...]
    recommendations: tuple[str, ...]


@dataclass(frozen=True, slots=True)
class PlaybackRecommendationDto:
    recommended_hold_frames: float | None
    recommended_tempo_scale: float | None
    summary: str


@dataclass(frozen=True, slots=True)
class SongDetailDto:
    song_id: str
    title: str
    duration_us: int | None
    note_count: int | None
    format_label: str
    risk: RiskSummaryDto
    recommendation: PlaybackRecommendationDto | None


@dataclass(frozen=True, slots=True)
class RiskDecisionDto:
    decision: str
    label: str


@dataclass(frozen=True, slots=True)
class PreparedPlaybackDto:
    prepared_id: str
    song: SongDetailDto
    config: PlaybackConfigDto
    admission: Admission
    risk: RiskSummaryDto
    decisions: tuple[RiskDecisionDto, ...]


@dataclass(frozen=True, slots=True)
class PlaybackSnapshotDto:
    seq: int
    state: PlaybackState
    song_id: str
    title: str
    current_us: int
    total_us: int
    pre_roll_remaining_us: int
    focus_state: FocusState
    health: HealthState
    input_path_degraded: bool
    message: str | None


@dataclass(frozen=True, slots=True)
class DiagnosticsSnapshotDto:
    seq: int
    max_lateness_us: int
    p50_ms: float
    p95_ms: float
    sigma_onset_ms: float
    late_2ms: int
    late_5ms: int
    late_10ms: int
    active_keys: int
    stuck_keys: int
    keys_dropped: int
    chord_split_events: int
    backend_status: str
    release_max_us: int | None
    release_late_2ms: int | None


__all__ = [
    "Admission",
    "BootstrapDto",
    "DiagnosticsSnapshotDto",
    "FocusState",
    "HealthState",
    "MetadataState",
    "NativeBuildDto",
    "PlaybackConfigDto",
    "PlaybackOptionSetsDto",
    "PlaybackRecommendationDto",
    "PlaybackSnapshotDto",
    "PlaybackState",
    "PreparedPlaybackDto",
    "RiskDecisionDto",
    "RiskLevel",
    "RiskSummaryDto",
    "SongDetailDto",
    "SongRowDto",
    "UpdatePreferencesDto",
]
