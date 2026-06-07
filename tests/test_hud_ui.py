from __future__ import annotations

from sky_music.infrastructure.hotkeys import HotkeyBinding, PlaybackControls
from sky_music.ui.hud import ProgressRenderer
from sky_music.ui.text_render import strip_ansi


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

    full = strip_ansi(renderer._build_controls_line("playing", 100, "", "", "", ""))
    compact = strip_ansi(renderer._build_controls_line("playing", 80, "", "", "", ""))
    minimal = strip_ansi(renderer._build_controls_line("playing", 60, "", "", "", ""))

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

    minimal = strip_ansi(renderer._build_controls_line("waiting_for_focus", 60, "", "", "", ""))

    assert "R refocus" in minimal
    assert "Q quit" in minimal
    assert "dry-run" not in minimal
    assert "panic" not in minimal
