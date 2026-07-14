from __future__ import annotations

from sky_music.domain.scheduler_types import FrameTimingPolicy
from sky_music.infrastructure.hotkeys import HotkeyBinding, PlaybackControls
from sky_music.ui.hud import ProgressRenderer


def _controls() -> PlaybackControls:
    return PlaybackControls(
        pause=HotkeyBinding("space", 0x20),
        skip=HotkeyBinding("s", 0x53),
        quit=HotkeyBinding("q", 0x51),
        refocus=HotkeyBinding("r", 0x52),
        panic=HotkeyBinding("esc", 0x1B),
    )


def test_hud_controls_use_width_tiers() -> None:
    renderer = ProgressRenderer(controls=_controls())

    full = renderer._build_controls_line("playing", 100).plain
    compact = renderer._build_controls_line("playing", 80).plain
    minimal = renderer._build_controls_line("playing", 60).plain

    assert "R refocus" in full
    assert "esc panic" in full
    assert "R refocus" not in compact
    assert "esc panic" in compact
    assert "R refocus" not in minimal
    assert "esc panic" not in minimal
    assert "space pause" in minimal
    assert "S skip" in minimal
    assert "Q quit" in minimal


def test_hud_controls_focus_waiting_keeps_refocus_on_narrow_width() -> None:
    renderer = ProgressRenderer(controls=_controls())

    minimal = renderer._build_controls_line("waiting_for_focus", 60).plain

    assert "R refocus" in minimal
    assert "Q quit" in minimal
    assert "dry-run" not in minimal
    assert "panic" not in minimal


def test_verbose_hud_timing_uses_fps_fallback_not_na() -> None:
    renderer = ProgressRenderer(controls=_controls(), verbose=True)
    renderer.active_policy = FrameTimingPolicy(  # type: ignore[arg-type]
        fps=0,  # type: ignore[arg-type]
        frame_us=0,  # type: ignore[arg-type]
        hold_us=10_000,  # type: ignore[arg-type]
        min_hold_us=10_000,  # type: ignore[arg-type]
        focus_restore_grace_us=100_000,  # type: ignore[arg-type]
        profile_name="fallback",
    )

    renderer.render(0.0, 1.0, "Test Song", force=True)
    renderer.finish()

    assert renderer._live is None
    assert renderer._initialized is False