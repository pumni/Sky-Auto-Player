"""Regression and unit tests for bounded Win32 foreground verification.

Verifies the pre-playback focus handshake in `window_target.py` and `engine.py`:
- exact HWND resolution and cached target integrity
- fast-path immediate success when target is already foreground
- tolerance of asynchronous Win32 foreground activation transitions
- tolerance of `focus()` returning False if exact HWND becomes foreground
- strict fail-closed behavior on true timeout, window destruction, or target change
- zero impact on native pre-roll duration or dispatch timing
"""

from __future__ import annotations

import asyncio
from pathlib import Path
from typing import Any

import pytest

from sky_music.config import AppConfig
from sky_music.domain import Song
from sky_music.domain.analyzer import ScheduleRiskReport
from sky_music.domain.scheduler import ScheduleMetadata
from sky_music.domain.scheduler_types import FrameTimingPolicy, KeyAction
from sky_music.infrastructure.background import WorkerSnapshot
from sky_music.orchestration.engine import PlaybackEngine
from sky_music.platform.win32 import window_target


class FakeClock:
    """Deterministic, fast monotonic clock for bounded polling tests."""

    def __init__(self, initial_ns: int = 1_000_000_000) -> None:
        self.now_ns = initial_ns
        self.sleep_calls: list[float] = []

    def monotonic_ns(self) -> int:
        return self.now_ns

    def sleep(self, seconds: float) -> None:
        self.sleep_calls.append(seconds)
        self.now_ns += int(seconds * 1_000_000_000)


class CountingFocusGuard:
    """Test focus guard that tracks focus calls and returns a configured value."""

    def __init__(self, return_value: bool = True) -> None:
        self.return_value = return_value
        self.focus_calls = 0
        self.is_active_calls = 0

    def is_active(self) -> bool:
        self.is_active_calls += 1
        return True

    def focus(self) -> bool:
        self.focus_calls += 1
        return self.return_value


def _make_engine(
    *,
    require_focus: bool = True,
    dry_run: bool = False,
    pre_roll_us: int = 3_000_000,
    focus_guard: Any = None,
) -> PlaybackEngine:
    song = Song(name="Test Song", notes=())
    actions: tuple[KeyAction, ...] = ()
    return PlaybackEngine(
        song=song,
        actions=actions,
        min_release_gap_us=16_667,
        require_focus=require_focus,
        dry_run=dry_run,
        pre_roll_us=pre_roll_us,
        focus_guard=focus_guard,
    )


# ---------------------------------------------------------------------------
# Test 1 — already foreground: zero focus calls, fast pass
# ---------------------------------------------------------------------------

def test_prepare_focus_already_foreground_skips_focus_call(monkeypatch: pytest.MonkeyPatch) -> None:
    target_hwnd = 12345
    window_target.reset_window_cache()

    monkeypatch.setattr(window_target, "get_sky_window", lambda: target_hwnd)
    monkeypatch.setattr(window_target.user32, "GetForegroundWindow", lambda: target_hwnd)
    monkeypatch.setattr(window_target.user32, "IsWindow", lambda _hwnd: 1)

    focus_guard = CountingFocusGuard(return_value=True)
    engine = _make_engine(focus_guard=focus_guard)

    result = engine.prepare_focus_for_playback()

    assert result == target_hwnd
    assert engine._prepared_target_hwnd == target_hwnd
    assert focus_guard.focus_calls == 0


# ---------------------------------------------------------------------------
# Test 2 — normal asynchronous transition: focus=True, foreground delayed
# ---------------------------------------------------------------------------

def test_prepare_focus_asynchronous_transition_succeeds(monkeypatch: pytest.MonkeyPatch) -> None:
    target_hwnd = 12345
    other_hwnd = 99999
    window_target.reset_window_cache()

    fake_clock = FakeClock()
    monkeypatch.setattr(window_target.time, "monotonic_ns", fake_clock.monotonic_ns)
    monkeypatch.setattr(window_target.time, "sleep", fake_clock.sleep)

    # Foreground changes:
    # 1. engine fast-path check (other)
    # 2. wait_for_foreground fast-path check (other)
    # 3. poll 0 (other) -> sleep 1
    # 4. poll 1 (other) -> sleep 2
    # 5. poll 2 (target) -> success
    foreground_sequence = [other_hwnd, other_hwnd, other_hwnd, other_hwnd, target_hwnd]

    def mock_get_foreground() -> int:
        if len(foreground_sequence) > 1:
            return foreground_sequence.pop(0)
        return foreground_sequence[0]

    monkeypatch.setattr(window_target, "get_sky_window", lambda: target_hwnd)
    monkeypatch.setattr(window_target.user32, "GetForegroundWindow", mock_get_foreground)
    monkeypatch.setattr(window_target.user32, "IsWindow", lambda _hwnd: 1)

    focus_guard = CountingFocusGuard(return_value=True)
    engine = _make_engine(focus_guard=focus_guard)

    result = engine.prepare_focus_for_playback()

    assert result == target_hwnd
    assert engine._prepared_target_hwnd == target_hwnd
    assert focus_guard.focus_calls == 1
    assert len(fake_clock.sleep_calls) == 2


# ---------------------------------------------------------------------------
# Test 3 — API says False but foreground succeeds (Critical Bugfix Case)
# ---------------------------------------------------------------------------

def test_prepare_focus_succeeds_even_when_focus_api_returns_false(monkeypatch: pytest.MonkeyPatch) -> None:
    target_hwnd = 12345
    other_hwnd = 99999
    window_target.reset_window_cache()

    fake_clock = FakeClock()
    monkeypatch.setattr(window_target.time, "monotonic_ns", fake_clock.monotonic_ns)
    monkeypatch.setattr(window_target.time, "sleep", fake_clock.sleep)

    foreground_sequence = [other_hwnd, other_hwnd, target_hwnd]

    def mock_get_foreground() -> int:
        if len(foreground_sequence) > 1:
            return foreground_sequence.pop(0)
        return foreground_sequence[0]

    monkeypatch.setattr(window_target, "get_sky_window", lambda: target_hwnd)
    monkeypatch.setattr(window_target.user32, "GetForegroundWindow", mock_get_foreground)
    monkeypatch.setattr(window_target.user32, "IsWindow", lambda _hwnd: 1)

    # Focus guard returns False (simulating Windows SetForegroundWindow returning 0)
    focus_guard = CountingFocusGuard(return_value=False)
    engine = _make_engine(focus_guard=focus_guard)

    result = engine.prepare_focus_for_playback()

    assert result == target_hwnd
    assert engine._prepared_target_hwnd == target_hwnd
    assert focus_guard.focus_calls == 1


# ---------------------------------------------------------------------------
# Test 4 — transition never finishes: bounded window timeout -> fail closed
# ---------------------------------------------------------------------------

def test_prepare_focus_times_out_and_fails_closed(monkeypatch: pytest.MonkeyPatch) -> None:
    target_hwnd = 12345
    other_hwnd = 99999
    window_target.reset_window_cache()

    fake_clock = FakeClock()
    monkeypatch.setattr(window_target.time, "monotonic_ns", fake_clock.monotonic_ns)
    monkeypatch.setattr(window_target.time, "sleep", fake_clock.sleep)

    monkeypatch.setattr(window_target, "get_sky_window", lambda: target_hwnd)
    monkeypatch.setattr(window_target.user32, "GetForegroundWindow", lambda: other_hwnd)
    monkeypatch.setattr(window_target.user32, "IsWindow", lambda _hwnd: 1)

    focus_guard = CountingFocusGuard(return_value=True)
    engine = _make_engine(focus_guard=focus_guard)

    result = engine.prepare_focus_for_playback()

    assert result is False
    assert engine._prepared_target_hwnd is None
    # Must call focus exactly once, never retry in loop
    assert focus_guard.focus_calls == 1
    # Bounded verification window must have polled ~20 times (100ms / 5ms)
    assert len(fake_clock.sleep_calls) == 20


# ---------------------------------------------------------------------------
# Test 5 — invalid Sky target: fails immediately without calling focus
# ---------------------------------------------------------------------------

def test_prepare_focus_invalid_sky_target_fails_without_focus_call(monkeypatch: pytest.MonkeyPatch) -> None:
    window_target.reset_window_cache()

    monkeypatch.setattr(window_target, "is_sky_window_valid", lambda: False)

    focus_guard = CountingFocusGuard(return_value=True)
    engine = _make_engine(focus_guard=focus_guard)

    result = engine.prepare_focus_for_playback()

    assert result is False
    assert focus_guard.focus_calls == 0


# ---------------------------------------------------------------------------
# Test 6 — zero or negative HWND: fails immediately
# ---------------------------------------------------------------------------

def test_prepare_focus_zero_cached_hwnd_fails(monkeypatch: pytest.MonkeyPatch) -> None:
    window_target.reset_window_cache()

    monkeypatch.setattr(window_target, "is_sky_window_valid", lambda: True)
    monkeypatch.setattr(window_target, "cached_target_hwnd", lambda: 0)

    focus_guard = CountingFocusGuard(return_value=True)
    engine = _make_engine(focus_guard=focus_guard)

    result = engine.prepare_focus_for_playback()

    assert result is False
    assert focus_guard.focus_calls == 0


# ---------------------------------------------------------------------------
# Test 7 — target changes during verification: fails closed (exact target invariant)
# ---------------------------------------------------------------------------

def test_prepare_focus_fails_if_target_hwnd_mutates_during_verification(monkeypatch: pytest.MonkeyPatch) -> None:
    initial_target = 12345
    new_target = 67890
    window_target.reset_window_cache()

    fake_clock = FakeClock()
    monkeypatch.setattr(window_target.time, "monotonic_ns", fake_clock.monotonic_ns)
    monkeypatch.setattr(window_target.time, "sleep", fake_clock.sleep)

    monkeypatch.setattr(window_target, "get_sky_window", lambda: initial_target)
    monkeypatch.setattr(window_target.user32, "GetForegroundWindow", lambda: new_target)
    monkeypatch.setattr(window_target.user32, "IsWindow", lambda _hwnd: 1)

    class MutatingFocusGuard:
        def __init__(self) -> None:
            self.focus_calls = 0

        def is_active(self) -> bool:
            return True

        def focus(self) -> bool:
            self.focus_calls += 1
            # Sky recreated window mid-transaction
            window_target._target_hwnd = new_target
            return True

    focus_guard = MutatingFocusGuard()
    engine = _make_engine(focus_guard=focus_guard)

    result = engine.prepare_focus_for_playback()

    # Must fail closed: cannot accept new_target in this transaction
    assert result is False
    assert engine._prepared_target_hwnd is None


# ---------------------------------------------------------------------------
# Test 8 — HWND destroyed during verification: terminates promptly
# ---------------------------------------------------------------------------

def test_prepare_focus_fails_promptly_if_hwnd_destroyed(monkeypatch: pytest.MonkeyPatch) -> None:
    target_hwnd = 12345
    window_target.reset_window_cache()

    fake_clock = FakeClock()
    monkeypatch.setattr(window_target.time, "monotonic_ns", fake_clock.monotonic_ns)
    monkeypatch.setattr(window_target.time, "sleep", fake_clock.sleep)

    monkeypatch.setattr(window_target, "get_sky_window", lambda: target_hwnd)
    monkeypatch.setattr(window_target.user32, "GetForegroundWindow", lambda: 99999)

    monkeypatch.setattr(window_target.user32, "IsWindow", lambda _hwnd: 0)

    focus_guard = CountingFocusGuard(return_value=True)
    engine = _make_engine(focus_guard=focus_guard)

    result = engine.prepare_focus_for_playback()

    assert result is False
    assert focus_guard.focus_calls == 1
    # Terminated immediately on window destruction without looping out all 20 sleep cycles
    assert len(fake_clock.sleep_calls) == 0


# ---------------------------------------------------------------------------
# Test 9 — fake clock ensures sub-millisecond deterministic execution
# ---------------------------------------------------------------------------

def test_bounded_verification_runs_deterministically_with_fake_clock(monkeypatch: pytest.MonkeyPatch) -> None:
    target_hwnd = 12345
    window_target._target_hwnd = target_hwnd

    fake_clock = FakeClock(initial_ns=500_000_000)
    monkeypatch.setattr(window_target.time, "monotonic_ns", fake_clock.monotonic_ns)
    monkeypatch.setattr(window_target.time, "sleep", fake_clock.sleep)
    monkeypatch.setattr(window_target.user32, "GetForegroundWindow", lambda: 99999)
    monkeypatch.setattr(window_target.user32, "IsWindow", lambda _hwnd: 1)

    success = window_target.wait_for_foreground_hwnd(target_hwnd, timeout_ms=100, poll_ms=5)

    assert success is False
    assert len(fake_clock.sleep_calls) == 20
    assert fake_clock.now_ns >= 500_000_000 + 100_000_000


# ---------------------------------------------------------------------------
# Test 10 — UI does not start playback on verification timeout (Fail Closed)
# ---------------------------------------------------------------------------

def test_ui_shows_focus_error_and_does_not_start_playback_on_timeout(monkeypatch: pytest.MonkeyPatch) -> None:
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app import playback_app as playback_module
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.ui.textual_app.playback_app import PlaybackCard
    from sky_music.ui.textual_app.playback_controller import PlaybackPlan
    from sky_music.ui.textual_app.screens import picker as picker_module

    class MockMetadataCoordinator:
        def __init__(self, *args, **kwargs) -> None:
            self.closed = False

        @property
        def name(self) -> str:
            return "mock-metadata"

        @property
        def phase(self) -> str:
            return "picker"

        def refresh(self, paths) -> None:
            pass

        def cancel(self) -> None:
            pass

        def close(self, *, wait: bool = False) -> None:
            self.closed = True

        def snapshot(self) -> Any:
            return WorkerSnapshot(
                name=self.name,
                phase=self.phase,
                closed=self.closed,
                pending_count=0,
                running_count=0,
            )

    def mock_prepare_playback(song_path, session, cfg, is_dry_run=False):
        song = Song(name="Mock Song", notes=())
        policy = FrameTimingPolicy.from_hold_frames(1.0, 60)
        return PlaybackPlan(
            actions=(),
            sched_meta=ScheduleMetadata(actions=(), source_duration_us=5_000_000, playback_duration_us=5_000_000),  # type: ignore[arg-type]
            session=session,
            active_policy=policy,
            song=song,
            risk_report=ScheduleRiskReport(
                severity="low",
                impossible_repeats=0,
                impossible_same_key_repeats=0,
                compressed_holds=0,
                max_polyphony=1,
                min_any_note_gap_us=None,
                min_same_key_gap_us=None,
                dense_clusters=(),
                recommendations=(),
            ),
            cfg=cfg,
        )

    class MockTelemetry:
        def record_schedule_metadata(self, sched_meta) -> None:
            pass

    play_called: list[bool] = []

    class FailingPlaybackEngine:
        def __init__(self, *args, **kwargs) -> None:
            self.telemetry = MockTelemetry()

        def prepare_focus_for_playback(self) -> bool:
            return False

        def play(self) -> str:
            play_called.append(True)
            return "finished"

    monkeypatch.setattr("sky_music.ui.picker_helpers.get_song_choices", lambda force_refresh=False: [Path("songs/Alpha.json")])
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(picker_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(app_module, "prepare_playback", mock_prepare_playback)
    monkeypatch.setattr(playback_module, "is_hotkey_down", lambda hotkey: False)

    import sky_music.orchestration.engine as engine_module
    monkeypatch.setattr(engine_module, "PlaybackEngine", FailingPlaybackEngine)

    async def run_failing_focus_test() -> None:
        app = SkyPickerApp(
            initial_dry_run=False,
            unified_mode=True,
            countdown_seconds=0,
            cfg=AppConfig(),
        )
        async with app.run_test() as pilot:
            await pilot.pause()
            await pilot.press("enter")
            await pilot.pause(0.1)

            card = app.query_one("#playback-card", PlaybackCard)
            assert card is not None
            assert card._mode == "error"
            assert card._error_title == "Focus Error"

    asyncio.run(run_failing_focus_test())
    assert len(play_called) == 0


# ---------------------------------------------------------------------------
# Test 11 — UI starts playback when delayed verification succeeds
# ---------------------------------------------------------------------------

def test_ui_starts_playback_on_delayed_focus_verification_success(monkeypatch: pytest.MonkeyPatch) -> None:
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app import playback_app as playback_module
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.ui.textual_app.playback_controller import PlaybackPlan
    from sky_music.ui.textual_app.screens import picker as picker_module

    class MockMetadataCoordinator:
        def __init__(self, *args, **kwargs) -> None:
            self.closed = False

        @property
        def name(self) -> str:
            return "mock-metadata"

        @property
        def phase(self) -> str:
            return "picker"

        def refresh(self, paths) -> None:
            pass

        def cancel(self) -> None:
            pass

        def close(self, *, wait: bool = False) -> None:
            self.closed = True

        def snapshot(self) -> Any:
            return WorkerSnapshot(
                name=self.name,
                phase=self.phase,
                closed=self.closed,
                pending_count=0,
                running_count=0,
            )

    def mock_prepare_playback(song_path, session, cfg, is_dry_run=False):
        song = Song(name="Mock Song", notes=())
        policy = FrameTimingPolicy.from_hold_frames(1.0, 60)
        return PlaybackPlan(
            actions=(),
            sched_meta=ScheduleMetadata(actions=(), source_duration_us=5_000_000, playback_duration_us=5_000_000),  # type: ignore[arg-type]
            session=session,
            active_policy=policy,
            song=song,
            risk_report=ScheduleRiskReport(
                severity="low",
                impossible_repeats=0,
                impossible_same_key_repeats=0,
                compressed_holds=0,
                max_polyphony=1,
                min_any_note_gap_us=None,
                min_same_key_gap_us=None,
                dense_clusters=(),
                recommendations=(),
            ),
            cfg=cfg,
        )

    class MockTelemetry:
        def record_schedule_metadata(self, sched_meta) -> None:
            pass

    play_called: list[bool] = []

    class SuccessfulPlaybackEngine:
        def __init__(self, *args, **kwargs) -> None:
            self.telemetry = MockTelemetry()

        def prepare_focus_for_playback(self) -> int:
            return 12345

        def play(self) -> str:
            play_called.append(True)
            return "finished"

    monkeypatch.setattr("sky_music.ui.picker_helpers.get_song_choices", lambda force_refresh=False: [Path("songs/Alpha.json")])
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(picker_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(app_module, "prepare_playback", mock_prepare_playback)
    monkeypatch.setattr(playback_module, "is_hotkey_down", lambda hotkey: False)

    import sky_music.orchestration.engine as engine_module
    monkeypatch.setattr(engine_module, "PlaybackEngine", SuccessfulPlaybackEngine)

    async def run_successful_focus_test() -> None:
        app = SkyPickerApp(
            initial_dry_run=False,
            unified_mode=True,
            countdown_seconds=0,
            cfg=AppConfig(),
        )
        async with app.run_test() as pilot:
            await pilot.pause()
            await pilot.press("enter")
            await pilot.pause(0.3)

    asyncio.run(run_successful_focus_test())
    assert len(play_called) == 1


# ---------------------------------------------------------------------------
# Test 12 — pre-roll duration is not modified by focus verification duration
# ---------------------------------------------------------------------------

def test_pre_roll_us_remains_exact_and_unmodified_by_focus_handshake(monkeypatch: pytest.MonkeyPatch) -> None:
    target_hwnd = 12345
    window_target.reset_window_cache()

    fake_clock = FakeClock()
    monkeypatch.setattr(window_target.time, "monotonic_ns", fake_clock.monotonic_ns)
    monkeypatch.setattr(window_target.time, "sleep", fake_clock.sleep)

    foreground_sequence = [99999, 99999, target_hwnd]

    def mock_get_foreground() -> int:
        if len(foreground_sequence) > 1:
            return foreground_sequence.pop(0)
        return foreground_sequence[0]

    monkeypatch.setattr(window_target, "get_sky_window", lambda: target_hwnd)
    monkeypatch.setattr(window_target.user32, "GetForegroundWindow", mock_get_foreground)
    monkeypatch.setattr(window_target.user32, "IsWindow", lambda _hwnd: 1)

    focus_guard = CountingFocusGuard(return_value=True)
    engine = _make_engine(focus_guard=focus_guard, pre_roll_us=3_000_000)

    result = engine.prepare_focus_for_playback()

    assert result == target_hwnd
    # The pre-roll duration MUST remain exactly 3,000,000 µs
    assert engine.pre_roll_us == 3_000_000


# ---------------------------------------------------------------------------
# Test 13 — behavior guard: no re-enumeration or extra focus calls during polling
# ---------------------------------------------------------------------------

def test_no_window_reenumeration_during_bounded_polling(monkeypatch: pytest.MonkeyPatch) -> None:
    target_hwnd = 12345
    window_target.reset_window_cache()

    fake_clock = FakeClock()
    monkeypatch.setattr(window_target.time, "monotonic_ns", fake_clock.monotonic_ns)
    monkeypatch.setattr(window_target.time, "sleep", fake_clock.sleep)

    valid_calls = 0
    reset_calls = 0

    def mock_get_sky() -> int:
        nonlocal valid_calls
        valid_calls += 1
        return target_hwnd

    def mock_reset() -> None:
        nonlocal reset_calls
        reset_calls += 1
        window_target._target_hwnd = None

    monkeypatch.setattr(window_target, "get_sky_window", mock_get_sky)
    monkeypatch.setattr(window_target, "reset_window_cache", mock_reset)
    monkeypatch.setattr(window_target.user32, "GetForegroundWindow", lambda: 99999)
    monkeypatch.setattr(window_target.user32, "IsWindow", lambda _hwnd: 1)

    focus_guard = CountingFocusGuard(return_value=True)
    engine = _make_engine(focus_guard=focus_guard)

    result = engine.prepare_focus_for_playback()

    assert result is False
    # Verified exactly 1 reset, 1 validation, 1 focus call across all 20 polling steps
    assert reset_calls == 1
    assert valid_calls == 1
    assert focus_guard.focus_calls == 1
    assert len(fake_clock.sleep_calls) == 20


# ---------------------------------------------------------------------------
# Test 14 — argument validation on window_target helper functions
# ---------------------------------------------------------------------------

def test_window_target_helpers_strictly_validate_arguments() -> None:
    with pytest.raises(ValueError, match="hwnd must be a positive integer"):
        window_target.is_hwnd_foreground(0)

    with pytest.raises(ValueError, match="hwnd must be a positive integer"):
        window_target.is_hwnd_foreground(-1)  # type: ignore[arg-type]

    with pytest.raises(ValueError, match="hwnd must be a positive integer"):
        window_target.is_hwnd_foreground("invalid")  # type: ignore[arg-type]

    with pytest.raises(ValueError, match="hwnd must be a positive integer"):
        window_target.wait_for_foreground_hwnd(0)

    with pytest.raises(ValueError, match="timeout_ms must be a non-negative integer"):
        window_target.wait_for_foreground_hwnd(123, timeout_ms=-1)

    with pytest.raises(ValueError, match="poll_ms must be a positive integer"):
        window_target.wait_for_foreground_hwnd(123, poll_ms=0)
