from __future__ import annotations

import inspect
from io import StringIO

from rich.console import Console

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
    renderer.active_policy = FrameTimingPolicy.from_hold_frames(1.0, 60, margin_us=0)

    renderer.render(0.0, 1.0, "Test Song", force=True)
    renderer.finish()

    assert renderer._live is None
    assert renderer._initialized is False


def test_hud_uses_current_input_latency_snapshot_without_latching() -> None:
    renderer = ProgressRenderer(controls=_controls())

    renderer.render(0.0, 1.0, "Test Song", force=True, input_path_degraded=True)
    assert renderer.input_path_degraded is True
    renderer.render(0.0, 1.0, "Test Song", force=True, input_path_degraded=False)
    assert renderer.input_path_degraded is False
    renderer.finish()


def test_hud_input_latency_warning_is_neutral_and_exact() -> None:
    from sky_music.ui import playback_notices

    source = inspect.getsource(playback_notices.PlaybackNoticeLedger)
    assert "Input dispatch latency is elevated; playback timing may be unstable." in source
    assert "hook" not in source.lower()
    assert "filter keys" not in source.lower()


def test_hud_retains_schedule_warning_state_for_live_renderer() -> None:
    renderer = ProgressRenderer(
        controls=_controls(),
        schedule_warnings=("schedule repeat warning",),
    )
    renderer.render(0.0, 1.0, "Test Song", force=True)
    state = renderer._notice_ledger.update()
    renderer.finish()

    assert [notice.message for notice in state.persistent_notices] == [
        "schedule repeat warning"
    ]


def test_progress_renderer_surfaces_missed_note_counters(monkeypatch) -> None:
    import sky_music.ui.hud as hud_module

    class FakeLive:
        def __init__(self, renderable, **_kwargs) -> None:
            self.renderable = renderable

        def start(self) -> None:
            return None

        def update(self, renderable) -> None:
            self.renderable = renderable

        def stop(self) -> None:
            return None

    monkeypatch.setattr(hud_module, "Live", FakeLive)

    normal = ProgressRenderer()
    normal.render(
        0.0,
        1.0,
        "Test Song",
        force=True,
        missed_down_boundaries=2,
        missed_down_keys=5,
        missed_hard_late_boundaries=2,
        late_authorized_boundaries=4,
    )
    normal_output = StringIO()
    assert normal._live is not None
    Console(file=normal_output, width=120).print(normal._live.renderable)
    assert "missed notes: 5" in normal_output.getvalue()
    normal.finish()

    verbose = ProgressRenderer(verbose=True)
    verbose.render(
        0.0,
        1.0,
        "Test Song",
        force=True,
        missed_down_boundaries=2,
        missed_hard_late_boundaries=2,
        late_authorized_boundaries=4,
    )
    verbose_output = StringIO()
    assert verbose._live is not None
    Console(file=verbose_output, width=120).print(verbose._live.renderable)
    assert "missed:2" in verbose_output.getvalue()
    assert "hard:2" in verbose_output.getvalue()
    assert "late-ok:4" in verbose_output.getvalue()
    verbose.finish()
