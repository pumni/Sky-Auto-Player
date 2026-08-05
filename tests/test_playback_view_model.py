from __future__ import annotations

from sky_music.orchestration.native_models import BackendHealth
from sky_music.ui.playback_view_model import build_playback_hud_view


def _health(*, stuck: int = 0, dropped: int = 0) -> BackendHealth:
    return BackendHealth(
        active_count=2,
        possibly_active_count=1,
        failed_release_count=stuck,
        last_error=None,
        keys_dropped=dropped,
        chord_split_events=3,
    )


def test_rich_and_textual_share_the_same_hud_facts() -> None:
    view = build_playback_hud_view(
        current_seconds=2.5,
        total_seconds=10.0,
        song_name="song",
        status="focus_lost",
        backend_health=_health(stuck=2, dropped=4),
        late_2ms=5,
        late_5ms=3,
        late_10ms=1,
        max_lateness_us=2_500,
        p50_ms=0.4,
        p95_ms=1.2,
        sigma_onset_ms=0.2,
    )

    assert view.status_label == "Focus Lost"
    assert view.progress_fraction == 0.25
    assert view.eta_seconds == 7.5
    assert view.backend.healthy is False
    assert view.backend.stuck_keys == 2
    assert view.backend.keys_dropped == 4
    assert view.timing.max_lateness_ms == 2.5
    assert view.timing.p95_ms == 1.2


def test_hud_view_clamps_progress_and_preserves_unknown_renderer_mode() -> None:
    view = build_playback_hud_view(
        current_seconds=-1.0,
        total_seconds=0.0,
        song_name="song",
        status="countdown",
    )

    assert view.current_seconds == 0.0
    assert view.progress_fraction == 0.0
    assert view.eta_seconds == 0.0
    assert view.status_label == "Countdown"
