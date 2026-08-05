"""Renderer-neutral playback presentation state."""

from __future__ import annotations

from dataclasses import dataclass

from sky_music.orchestration.native_models import (
    STATUS_LABELS,
    BackendHealth,
    PlaybackStatus,
)
from sky_music.ui.playback_notices import PlaybackNotice


@dataclass(frozen=True, slots=True)
class PlaybackBackendView:
    """Backend counters needed by either live HUD renderer."""

    healthy: bool
    active_keys: int
    stuck_keys: int
    keys_dropped: int
    chord_split_events: int
    label: str


@dataclass(frozen=True, slots=True)
class PlaybackTimingView:
    """Timing counters and optional distribution statistics for the HUD."""

    late_2ms: int
    late_5ms: int
    late_10ms: int
    max_lateness_ms: float
    p50_ms: float
    p95_ms: float
    sigma_onset_ms: float


@dataclass(frozen=True, slots=True)
class PlaybackHudViewModel:
    """Shared playback facts; renderers decide how to present them."""

    status: str
    status_label: str
    song_name: str
    current_seconds: float
    total_seconds: float
    eta_seconds: float
    progress_fraction: float
    input_path_degraded: bool
    sendinput_path_degraded: bool
    bookkeeping_degraded: bool
    wait_path_degraded: bool
    backend: PlaybackBackendView
    timing: PlaybackTimingView
    notices: tuple[PlaybackNotice, ...] = ()


def _status_label(status: str) -> str:
    try:
        return STATUS_LABELS[PlaybackStatus(status)]
    except (KeyError, ValueError):
        return status.replace("_", " ").title()


def backend_view(backend_health: BackendHealth | None) -> PlaybackBackendView:
    """Normalize native backend counters once for both renderers."""

    active_keys = int(getattr(backend_health, "active_count", 0) or 0)
    stuck_keys = int(getattr(backend_health, "failed_release_count", 0) or 0)
    keys_dropped = int(getattr(backend_health, "keys_dropped", 0) or 0)
    chord_split_events = int(getattr(backend_health, "chord_split_events", 0) or 0)
    return PlaybackBackendView(
        healthy=stuck_keys == 0,
        active_keys=active_keys,
        stuck_keys=stuck_keys,
        keys_dropped=keys_dropped,
        chord_split_events=chord_split_events,
        label="healthy" if stuck_keys == 0 else f"stuck:{stuck_keys}",
    )


def build_playback_hud_view(
    *,
    current_seconds: float,
    total_seconds: float,
    song_name: str,
    status: str,
    input_path_degraded: bool = False,
    sendinput_path_degraded: bool = False,
    bookkeeping_degraded: bool = False,
    wait_path_degraded: bool = False,
    backend_health: BackendHealth | None = None,
    late_2ms: int = 0,
    late_5ms: int = 0,
    late_10ms: int = 0,
    max_lateness_us: int = 0,
    p50_ms: float = 0.0,
    p95_ms: float = 0.0,
    sigma_onset_ms: float = 0.0,
    notices: tuple[PlaybackNotice, ...] = (),
) -> PlaybackHudViewModel:
    """Build the facts shared by Rich and Textual playback surfaces."""

    total_safe = max(total_seconds, 0.001)
    current_safe = max(0.0, current_seconds)
    return PlaybackHudViewModel(
        status=status,
        status_label=_status_label(status),
        song_name=song_name,
        current_seconds=current_safe,
        total_seconds=total_seconds,
        eta_seconds=max(0.0, total_seconds - current_safe),
        progress_fraction=min(1.0, current_safe / total_safe),
        input_path_degraded=input_path_degraded,
        sendinput_path_degraded=sendinput_path_degraded,
        bookkeeping_degraded=bookkeeping_degraded,
        wait_path_degraded=wait_path_degraded,
        backend=backend_view(backend_health),
        timing=PlaybackTimingView(
            late_2ms=late_2ms,
            late_5ms=late_5ms,
            late_10ms=late_10ms,
            max_lateness_ms=max_lateness_us / 1000.0,
            p50_ms=p50_ms,
            p95_ms=p95_ms,
            sigma_onset_ms=sigma_onset_ms,
        ),
        notices=notices,
    )
