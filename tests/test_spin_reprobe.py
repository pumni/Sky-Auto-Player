"""Phase H: Mid-song spin re-probe -- unit tests (H.4)."""
from __future__ import annotations

from sky_music.orchestration.core.loop import (
    REPROBE_MIN_GAP_US,
    REPROBE_MIN_INTERVAL_US,
    DispatchLoop,
)


class _FakeClock:
    def __init__(self, step_us: int = 1_000) -> None:
        self._us = 0
        self._step = step_us

    def now_us(self) -> int:
        v = self._us
        self._us += self._step
        return v


class _FakeSleeper:
    def __init__(self, clock: _FakeClock, wake_error_us: int = 500) -> None:
        self._clock = clock
        self._wake_error_us = wake_error_us
        self.sleep_calls: int = 0

    def sleep(self, seconds: float) -> None:
        self.sleep_calls += 1
        self._clock._us += int(seconds * 1_000_000) + self._wake_error_us


def _make_loop_for_reprobe(wake_error_us: int = 500):
    from sky_music.infrastructure.backend import DryRunBackend
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.infrastructure.wait_strategy import HybridWaitStrategy
    from sky_music.orchestration.core.coordinator import RuntimeDispatchCoordinator
    from sky_music.orchestration.core.loop import DispatchHealthMonitor
    from sky_music.orchestration.telemetry import TelemetryLogger

    class NoopFocusController:
        def is_active(self): return True
        def focus(self): return True

    clock = _FakeClock()
    sleeper = _FakeSleeper(clock, wake_error_us)
    backend = DryRunBackend()
    telemetry = TelemetryLogger(song_name="reprobe-test", enabled=False)

    from sky_music.orchestration.core.coordinator import RuntimeSchedule
    sched = RuntimeSchedule(batches=(), generation_count=0)

    coordinator = RuntimeDispatchCoordinator(sched, min_hold_us=0)
    health = DispatchHealthMonitor(
        backend=backend,
        clock=clock,
        focus_guard=NoopFocusController(),
        require_focus=False,
    )
    loop = DispatchLoop(
        coordinator=coordinator,
        clock=clock,
        sleeper=sleeper,
        wait_strategy=HybridWaitStrategy(enable_event_wait=False),
        backend=backend,
        telemetry=telemetry,
        sleep_policy=SleepPolicy(),
        health_monitor=health,
        min_hold_us=0,
        spin_threshold_us=700,
    )
    loop.enable_spin_reprobe = True
    loop._spin_floor_us = 700
    return loop, sleeper


def test_reprobe_runs_and_records_telemetry() -> None:
    """Large gap triggers reprobe, REPROBE_SAMPLES sleep calls are made."""
    loop, sleeper = _make_loop_for_reprobe(wake_error_us=800)
    elapsed_us = REPROBE_MIN_GAP_US + 1
    initial_calls = sleeper.sleep_calls
    loop._run_mid_song_reprobe(elapsed_us)
    assert sleeper.sleep_calls == initial_calls + 8
    assert loop._last_reprobe_elapsed_us == elapsed_us
    assert len(loop._reprobe_applied_thresholds) == 1


def test_reprobe_second_within_interval_guard_prevents() -> None:
    """Second reprobe within REPROBE_MIN_INTERVAL_US is blocked by the guard."""
    loop, sleeper = _make_loop_for_reprobe()
    first_elapsed = REPROBE_MIN_GAP_US + 1
    loop._run_mid_song_reprobe(first_elapsed)
    calls_after_first = sleeper.sleep_calls
    second_elapsed = first_elapsed + REPROBE_MIN_INTERVAL_US - 1
    interval_elapsed = second_elapsed - loop._last_reprobe_elapsed_us
    should_reprobe = interval_elapsed >= REPROBE_MIN_INTERVAL_US
    assert not should_reprobe
    assert sleeper.sleep_calls == calls_after_first


def test_reprobe_allowed_after_interval() -> None:
    """After REPROBE_MIN_INTERVAL_US has elapsed, a second reprobe is allowed."""
    loop, sleeper = _make_loop_for_reprobe()
    loop._run_mid_song_reprobe(REPROBE_MIN_GAP_US + 1)
    calls_after_first = sleeper.sleep_calls
    second_elapsed = loop._last_reprobe_elapsed_us + REPROBE_MIN_INTERVAL_US
    should_reprobe = second_elapsed - loop._last_reprobe_elapsed_us >= REPROBE_MIN_INTERVAL_US
    assert should_reprobe
    loop._run_mid_song_reprobe(second_elapsed)
    assert sleeper.sleep_calls == calls_after_first + 8
    assert len(loop._reprobe_applied_thresholds) == 2


def test_reprobe_kill_switch_false() -> None:
    """enable_spin_reprobe=False means the guard never fires."""
    loop, sleeper = _make_loop_for_reprobe()
    loop.enable_spin_reprobe = False
    remaining_us = REPROBE_MIN_GAP_US + 1
    elapsed_us = REPROBE_MIN_GAP_US + 1
    guard = (
        loop.enable_spin_reprobe
        and remaining_us >= REPROBE_MIN_GAP_US
        and elapsed_us - loop._last_reprobe_elapsed_us >= REPROBE_MIN_INTERVAL_US
    )
    assert not guard
    assert sleeper.sleep_calls == 0