"""Post-play reachable-memory hygiene.

Verifies the two retention holdouts flagged in the hygiene plan are released after
PlaybackEngine.play() returns, without regressing summary accuracy:

  1. ``TelemetryLogger.records`` is cleared once persisted to disk (production path),
     and ``get_summary()`` still returns the cached ``_last_summary`` for late callers
     (engine ``_log_timing_summary``, CLI reports, tests).
  2. ``inputs._ARRAY_CACHE`` is cleared from engine.play()'s finally block so a long
     session of many songs does not accumulate chord-shaped ctypes arrays.

These are reachable-object hygiene tests — not Task Manager RSS claims. They use a
deterministic fake clock + minimal backend so they run fast and reproducibly.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any
from unittest.mock import patch

from sky_music.domain import Song
from sky_music.domain.scheduler_types import (
    ActionKind,
    KeyAction,
    Microseconds,
    ScanCode,
)
from sky_music.infrastructure.backend import (
    BackendHealth,
    InputSendResult,
    ReleaseAllOutcome,
)
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.engine import PlaybackEngine
from sky_music.platform.win32 import inputs


class _TinyBackend:
    """Minimal InputBackend — no real SendInput, no threading needed."""

    def __init__(self) -> None:
        self.active: set[int] = set()

    def key_down(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        self.active.update(scan_codes)
        return InputSendResult(sent=scan_codes, skipped_duplicates=(), success=True)

    def key_up(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        self.active.difference_update(scan_codes)
        return InputSendResult(sent=scan_codes, skipped_duplicates=(), success=True)

    def release_all(self) -> ReleaseAllOutcome:
        attempted = tuple(sorted(self.active))
        self.active.clear()
        return ReleaseAllOutcome(
            attempted=attempted,
            released_successfully=True,
            stuck_keys=(),
            verification_inconclusive=False,
        )

    def release_all_full_instrument(self) -> ReleaseAllOutcome:
        return self.release_all()

    def get_health(self) -> BackendHealth:
        return BackendHealth(
            active_count=len(self.active),
            possibly_active_count=0,
            failed_release_count=0,
            last_error=None,
        )

    def get_send_diagnostics(self) -> dict[str, int]:
        return {}


class _FakeClock:
    def __init__(self) -> None:
        self.time_us = 0

    def now_us(self) -> int:
        return self.time_us


class _FakeSleeper:
    is_high_resolution = False

    def __init__(self, clock: _FakeClock) -> None:
        self.clock = clock

    def sleep(self, seconds: float) -> None:
        self.clock.time_us += max(1, int(seconds * 1_000_000))


def _action(at_us: int, kind: str, scan: int) -> KeyAction:
    return KeyAction(
        kind=ActionKind(kind),  # type: ignore[arg-type]
        scan_codes=(ScanCode(scan),),
        at_us=Microseconds(at_us),
        reason="hygiene",
    )


def _song_actions() -> tuple[Song, tuple[KeyAction, ...]]:
    # A short alternating down/up timeline so the test is fast but produces real records.
    scan_pool = (21, 22, 23)
    actions: list[KeyAction] = []
    for i in range(12):
        scan = scan_pool[(i // 2) % len(scan_pool)]
        if i % 2 == 0:
            actions.append(_action(i * 20_000, "down", scan))
        else:
            actions.append(_action(i * 20_000, "up", scan))
    return Song(name="hygiene", notes=()), tuple(actions)


def _run_engine(*, telemetry_enabled: bool, retain: bool = False) -> PlaybackEngine:
    """Build + play a tiny engine; return it for post-play assertions."""
    clock = _FakeClock()
    song, actions = _song_actions()
    engine = PlaybackEngine(
        song=song,
        actions=actions,
        backend=_TinyBackend(),
        telemetry_enabled=telemetry_enabled,
        require_focus=False,
        clock=clock,
        sleeper=_FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        use_dispatch_thread=False,
        retain_telemetry_records_after_save=retain,
    )
    engine.play()
    return engine


# ---------------------------------------------------------------------------
# Phase 1 — telemetry records hygiene
# ---------------------------------------------------------------------------


def test_telemetry_records_cleared_after_save_when_enabled(tmp_path: Path) -> None:
    """Production path: enabled + save() persists CSV/summary, then records is empty."""
    csv_path = tmp_path / "playback_telemetry_test.csv"
    clock = _FakeClock()
    song, actions = _song_actions()
    engine = PlaybackEngine(
        song=song,
        actions=actions,
        backend=_TinyBackend(),
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=_FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        use_dispatch_thread=False,
    )
    # Point the engine's logger at a controlled tmp path so save() writes where we can assert.
    engine.telemetry.log_filepath = csv_path

    assert engine.play() == "finished"

    assert len(engine.telemetry.records) == 0, "records must be cleared after save()"
    assert csv_path.exists(), "CSV must still be written despite records being cleared"
    summary_path = csv_path.with_suffix(".summary.json")
    assert summary_path.exists(), "companion summary JSON must be written"


def test_get_summary_returns_cached_summary_after_records_cleared(tmp_path: Path) -> None:
    """After save() clears records, get_summary() returns the cached dict, not None.

    Mirrors the real late-caller path: engine._log_timing_summary → get_summary() runs
    AFTER DispatchLoop.run's finally has called save(), which has cleared records.
    """
    csv_path = tmp_path / "playback_telemetry_test.csv"
    clock = _FakeClock()
    song, actions = _song_actions()
    engine = PlaybackEngine(
        song=song,
        actions=actions,
        backend=_TinyBackend(),
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=_FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        use_dispatch_thread=False,
    )
    engine.telemetry.log_filepath = csv_path

    engine.play()

    summary_after_clear = engine.telemetry.get_summary()
    assert summary_after_clear is not None
    # Sanity: aggregate is well-formed and reflects the run that just executed.
    assert summary_after_clear.get("total_events", 0) > 0
    assert "lateness_us" in summary_after_clear
    assert isinstance(summary_after_clear["lateness_us"], dict)


def test_telemetry_disabled_keeps_records_empty() -> None:
    """When telemetry_enabled=False, records stays empty throughout and no summary is cached."""
    engine = _run_engine(telemetry_enabled=False)
    assert engine.telemetry.records == []
    # No save() ran → _last_summary was never set; get_summary() returns None.
    assert engine.telemetry.get_summary() is None


def test_retain_flag_keeps_records_after_save(tmp_path: Path) -> None:
    """The test-only hook keeps records intact so tests asserting on raw records work."""
    csv_path = tmp_path / "playback_telemetry_test.csv"
    clock = _FakeClock()
    song, actions = _song_actions()
    engine = PlaybackEngine(
        song=song,
        actions=actions,
        backend=_TinyBackend(),
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=_FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        use_dispatch_thread=False,
        retain_telemetry_records_after_save=True,
    )
    engine.telemetry.log_filepath = csv_path
    engine.play()

    assert len(engine.telemetry.records) > 0, "retention flag must keep records"
    assert csv_path.exists(), "CSV is still written — flag only blocks the in-memory clear"


def test_save_failure_keeps_records(tmp_path: Path) -> None:
    """If the CSV write fails, records must NOT be cleared — they may be retried later."""
    csv_path = tmp_path / "playback_telemetry_test.csv"
    clock = _FakeClock()
    song, actions = _song_actions()
    engine = PlaybackEngine(
        song=song,
        actions=actions,
        backend=_TinyBackend(),
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=_FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        use_dispatch_thread=False,
    )
    engine.telemetry.log_filepath = csv_path

    # Force the CSV open to raise mid-save; records should be retained by the failure guard.
    original_open = Path.open

    def failing_open(self: Path, *args: Any, **kwargs: Any) -> Any:
        if str(self) == str(csv_path):
            raise OSError("simulated disk failure")
        return original_open(self, *args, **kwargs)

    with patch.object(Path, "open", failing_open):
        engine.play()

    assert len(engine.telemetry.records) > 0, "records must survive a save failure"
    assert not csv_path.exists(), "no CSV should be written when open fails"


# ---------------------------------------------------------------------------
# Phase 2 — _ARRAY_CACHE lifecycle
# ---------------------------------------------------------------------------


def test_clear_array_cache_drops_entries() -> None:
    """clear_array_cache() empties the process-global cached-array table and returns the count."""
    inputs._ARRAY_CACHE.clear()
    inputs._INPUT_CACHE.clear()
    with patch.object(inputs.user32, "SendInput", lambda n, arr, sz: n):
        inputs.prewarm_input_arrays([((0x15,), False), ((0x15, 0x16), True)])
    assert len(inputs._ARRAY_CACHE) == 2

    cleared = inputs.clear_array_cache()
    assert cleared == 2
    assert len(inputs._ARRAY_CACHE) == 0


def test_clear_array_cache_is_idempotent() -> None:
    """Calling clear twice is safe; the second call reports zero cleared."""
    inputs._ARRAY_CACHE.clear()
    with patch.object(inputs.user32, "SendInput", lambda n, arr, sz: n):
        inputs.prewarm_input_arrays([((0x17,), False)])

    first = inputs.clear_array_cache()
    second = inputs.clear_array_cache()
    assert first == 1
    assert second == 0
    assert len(inputs._ARRAY_CACHE) == 0


def test_clear_array_cache_keeps_input_cache() -> None:
    """The tiny per-key INPUT cache must NOT be cleared — it's reused on next prewarm."""
    inputs._ARRAY_CACHE.clear()
    inputs._INPUT_CACHE.clear()
    with patch.object(inputs.user32, "SendInput", lambda n, arr, sz: n):
        inputs.prewarm_input_arrays([((0x15,), False)])
    assert len(inputs._INPUT_CACHE) >= 1

    inputs.clear_array_cache()

    assert len(inputs._ARRAY_CACHE) == 0
    assert len(inputs._INPUT_CACHE) >= 1, "per-key INPUT cache must survive array clear"


def test_engine_play_clears_array_cache_in_finally() -> None:
    """engine.play() finally clears _ARRAY_CACHE so cross-song RSS does not accumulate arrays."""
    inputs._ARRAY_CACHE.clear()
    inputs._INPUT_CACHE.clear()
    # Seed the cache with shapes the test backend would not have populated itself, so we know
    # the clear came from the engine finally and not from natural LRU eviction mid-play.
    with patch.object(inputs.user32, "SendInput", lambda n, arr, sz: n):
        inputs.prewarm_input_arrays([((0x15,), False), ((0x15, 0x16), True)])
    assert len(inputs._ARRAY_CACHE) == 2

    # _run_engine() has already called play() internally; its finally cleared the cache.
    _ = _run_engine(telemetry_enabled=False)

    assert len(inputs._ARRAY_CACHE) == 0, "engine.play() finally must clear the array cache"
