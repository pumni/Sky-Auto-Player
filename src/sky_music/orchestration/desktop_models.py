"""Immutable DTOs shared by the future desktop adapter and Python services."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

RiskLevel = Literal["low", "medium", "high", "unknown"]
MetadataState = Literal["pending", "ready", "error"]
Admission = Literal["ready", "confirmation_required", "blocked"]
PlaybackDecision = Literal["proceed", "use_recommended", "dry_run"]
PlaybackControl = Literal["stop", "pause", "resume", "skip"]
PlaybackPendingControl = Literal["pause", "resume"]
PlaybackState = Literal[
    "idle",
    "ready",
    "awaiting_confirmation",
    "starting",
    "preparing",
    "countdown",
    "playing",
    "paused",
    "focus_lost",
    "stopping",
    "finished",
    "cancelled",
    "failed",
    "error",
]
FocusState = Literal["focused", "unfocused", "waiting"]
HealthState = Literal["healthy", "degraded", "error"]
CalibrationMode = Literal["quick", "full", "diagnostic"]
CalibrationState = Literal[
    "idle", "starting", "running", "cancelling", "succeeded", "failed", "cancelled"
]
CalibrationOutcome = Literal["succeeded", "failed", "cancelled"]
UpdateChannel = Literal["stable", "beta"]
UpdateState = Literal[
    "idle", "checking", "current", "available", "error", "handoff_in_progress", "handoff_ready"
]


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
    channel: UpdateChannel
    skip_version: str


@dataclass(frozen=True, slots=True)
class UpdateCheckDto:
    state: UpdateState
    current_version: str
    available_version: str | None
    channel: UpdateChannel
    release_notes: str | None
    published_at: str | None
    error: str | None


@dataclass(frozen=True, slots=True)
class UpdateHandoffDto:
    handoff_id: str
    target_version: str
    state: Literal["handoff_ready"]


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
    decision: PlaybackDecision
    label: str


@dataclass(frozen=True, slots=True)
class PreparedPlaybackDto:
    prepared_id: str | None
    song: SongDetailDto
    config: PlaybackConfigDto
    admission: Admission
    risk: RiskSummaryDto
    decisions: tuple[RiskDecisionDto, ...]
    plan_fingerprint: str | None = None
    variants: tuple[PlaybackPlanVariantDto, ...] = ()
    error_code: str | None = None
    error_message: str | None = None


@dataclass(frozen=True, slots=True)
class PlaybackPlanVariantDto:
    decision: PlaybackDecision
    config: PlaybackConfigDto
    plan_fingerprint: str


@dataclass(frozen=True, slots=True)
class PlaybackDecisionAcceptanceDto:
    decision: PlaybackDecision
    accepted: bool


@dataclass(frozen=True, slots=True)
class PlaybackSessionDto:
    session_id: str
    prepared_id: str
    song_id: str
    state: PlaybackState
    config: PlaybackConfigDto
    plan_fingerprint: str


@dataclass(frozen=True, slots=True)
class PlaybackCommandAckDto:
    accepted: bool
    session_id: str
    state: PlaybackState
    pending_command: PlaybackPendingControl | None
    reason: str | None = None


@dataclass(frozen=True, slots=True)
class PlaybackFinishedDto:
    session_id: str
    song_id: str
    outcome: str
    total_us: int
    message: str


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
    session_id: str | None = None


@dataclass(frozen=True, slots=True)
class DiagnosticsEnabledDto:
    enabled: bool


@dataclass(frozen=True, slots=True)
class CalibrationStartDto:
    mode: CalibrationMode = "quick"
    class_name: str | None = None
    polyphony: int | None = None
    samples: int | None = None
    timeout_seconds: float | None = None


@dataclass(frozen=True, slots=True)
class CalibrationStartAckDto:
    operation_id: str
    state: CalibrationState


@dataclass(frozen=True, slots=True)
class CalibrationCancelAckDto:
    operation_id: str
    state: CalibrationState
    accepted: bool


@dataclass(frozen=True, slots=True)
class CalibrationProgressDto:
    operation_id: str
    state: CalibrationState
    phase: str
    completed: int
    total: int
    message: str


@dataclass(frozen=True, slots=True)
class CalibrationFinishedDto:
    operation_id: str
    outcome: CalibrationOutcome
    status: str
    margin_us: int | None
    sample_count: int
    source: str
    message: str
    applied: bool


__all__ = [
    "Admission",
    "BootstrapDto",
    "CalibrationCancelAckDto",
    "CalibrationFinishedDto",
    "CalibrationMode",
    "CalibrationOutcome",
    "CalibrationProgressDto",
    "CalibrationStartAckDto",
    "CalibrationStartDto",
    "CalibrationState",
    "DiagnosticsEnabledDto",
    "DiagnosticsSnapshotDto",
    "FocusState",
    "HealthState",
    "MetadataState",
    "NativeBuildDto",
    "PlaybackCommandAckDto",
    "PlaybackConfigDto",
    "PlaybackControl",
    "PlaybackDecision",
    "PlaybackDecisionAcceptanceDto",
    "PlaybackFinishedDto",
    "PlaybackOptionSetsDto",
    "PlaybackPendingControl",
    "PlaybackPlanVariantDto",
    "PlaybackRecommendationDto",
    "PlaybackSessionDto",
    "PlaybackSnapshotDto",
    "PlaybackState",
    "PreparedPlaybackDto",
    "RiskDecisionDto",
    "RiskLevel",
    "RiskSummaryDto",
    "SongDetailDto",
    "SongRowDto",
    "UpdateChannel",
    "UpdateCheckDto",
    "UpdateHandoffDto",
    "UpdatePreferencesDto",
    "UpdateState",
]
