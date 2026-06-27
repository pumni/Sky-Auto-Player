from __future__ import annotations

from typing import cast

import pytest

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
from sky_music.infrastructure.timing import Clock, SleepPolicy
from sky_music.infrastructure.wait_strategy import HybridWaitStrategy
from sky_music.orchestration.engine import PLAYBACK_FINISHED, PlaybackEngine


class FakeClock:
    def __init__(self) -> None:
        self.time_us = 0
        self.log: list[str] = []

    def now_us(self) -> int:
        self.log.append("now_us")
        return self.time_us


class TeleportSpinStrategy(HybridWaitStrategy):
    """Test wait strategy: spinning advances the fake clock instead of busy-waiting forever."""

    def spin_until_us(self, target_system_us: int, clock: Clock) -> None:
        fake = cast(FakeClock, clock)
        fake.time_us = max(fake.time_us, target_system_us)


class WaitableTimerSleeper:
    # Capability flag: HybridWaitStrategy selects the timer-aware ladder on this, not on the
    # class name.
    is_high_resolution = True

    def __init__(self, clock: FakeClock) -> None:
        self.clock = clock
        self.sleeps: list[float] = []

    def sleep(self, seconds: float) -> None:
        self.sleeps.append(seconds)
        self.clock.time_us += max(1, int(seconds * 1_000_000))


class RegularFakeSleeper:
    def __init__(self, clock: FakeClock) -> None:
        self.clock = clock
        self.sleeps: list[float] = []

    def sleep(self, seconds: float) -> None:
        self.sleeps.append(seconds)
        self.clock.time_us += max(1, int(seconds * 1_000_000))


class TimedBackend:
    def __init__(self, clock: FakeClock) -> None:
        self.clock = clock
        self.active: set[int] = set()

    def key_down(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        self.active.update(scan_codes)
        return InputSendResult(sent=scan_codes, skipped_duplicates=(), success=True)

    def key_up(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        self.active.difference_update(scan_codes)
        return InputSendResult(sent=scan_codes, skipped_duplicates=(), success=True)

    def release_all(self) -> ReleaseAllOutcome:
        self.active.clear()
        return ReleaseAllOutcome(
            attempted=(),
            released_successfully=True,
            stuck_keys=(),
            verification_inconclusive=False,
        )

    def get_health(self) -> BackendHealth:
        return BackendHealth(0, 0, 0, None)

    def get_send_diagnostics(self) -> dict[str, int]:
        return {}


def test_hybrid_strategy_timer_aware_ladder() -> None:
    clock = FakeClock()
    waitable_sleeper = WaitableTimerSleeper(clock)
    strategy = TeleportSpinStrategy()
    policy = SleepPolicy(
        spin_threshold_us=500,
        coarse_sleep_max_us=20_000,
        coarse_sleep_threshold_us=5_000,
        medium_sleep_s=0.001,
    )

    def wait_step(target_us: int, sleeper) -> None:
        strategy.wait_until_us(
            target_system_us=target_us,
            clock=clock,
            sleeper=sleeper,
            spin_threshold_us=500,
            policy=policy,
        )

    # 1. Timer-aware ladder (sleeper declares is_high_resolution)
    # Case A: remaining large (50,000 us): sleeps up to 1ms caps towards target - guard.
    # remaining_to_sleep = 49,500 -> sleep_us = min(49,500, 1000) = 1000 us.
    clock.time_us = 0
    wait_step(50_000, waitable_sleeper)
    assert len(waitable_sleeper.sleeps) == 1
    assert waitable_sleeper.sleeps[-1] == 0.001  # capped at 1ms

    # Case B: remaining small (800 us): remaining_to_sleep = 300 us (0.0003s).
    clock.time_us = 0
    waitable_sleeper.sleeps.clear()
    wait_step(800, waitable_sleeper)
    assert len(waitable_sleeper.sleeps) == 1
    assert waitable_sleeper.sleeps[-1] == pytest.approx(0.0003)

    # Case C: remaining within guard (400 us <= 500): no sleep, the strategy busy-spins to the
    # target (teleported by the test strategy).
    clock.time_us = 0
    waitable_sleeper.sleeps.clear()
    wait_step(400, waitable_sleeper)
    assert len(waitable_sleeper.sleeps) == 0
    assert clock.time_us == 400

    # 2. Fallback standard ladder (sleeper without is_high_resolution)
    regular_sleeper = RegularFakeSleeper(clock)

    # Case A: remaining > coarse_sleep_max: sleeps min(20,000, 50,000 - 5,000) = 0.02s.
    clock.time_us = 0
    wait_step(50_000, regular_sleeper)
    assert len(regular_sleeper.sleeps) == 1
    assert regular_sleeper.sleeps[-1] == 0.02

    # Case B: coarse_threshold < remaining <= coarse_max (6,000 us): medium 1ms tick.
    clock.time_us = 0
    regular_sleeper.sleeps.clear()
    wait_step(6_000, regular_sleeper)
    assert len(regular_sleeper.sleeps) == 1
    assert regular_sleeper.sleeps[-1] == 0.001

    # Case C: spin_threshold < remaining <= coarse_threshold (2,000 us): yield slice.
    clock.time_us = 0
    regular_sleeper.sleeps.clear()
    wait_step(2_000, regular_sleeper)
    assert len(regular_sleeper.sleeps) == 1
    assert regular_sleeper.sleeps[-1] == 0.0


def test_wake_error_probe() -> None:
    # A fake sleeper of known error distribution: constant overshoot of 300 us
    class OvershootingSleeper:
        def __init__(self, clock: FakeClock, error_us: int) -> None:
            self.clock = clock
            self.error_us = error_us
            self.sleeps: list[float] = []

        def sleep(self, seconds: float) -> None:
            self.sleeps.append(seconds)
            requested = int(seconds * 1_000_000)
            self.clock.time_us += max(1, requested + self.error_us)

    song = Song(name="test_probe", notes=())
    actions = (KeyAction(kind=ActionKind.DOWN, scan_codes=(ScanCode(1),), at_us=Microseconds(50000), reason="d1"),)
    
    clock = FakeClock()
    backend = TimedBackend(clock)
    
    # Overshooting sleeper with +300 us wake error
    sleeper = OvershootingSleeper(clock, error_us=300)
    
    engine = PlaybackEngine(
        song=song,
        actions=actions,
        backend=backend,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=sleeper,
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        use_dispatch_thread=False,
        enable_adaptive_spin=True,
        wait_strategy=TeleportSpinStrategy(),
    )

    # Run engine play (which runs the probe)
    res = engine.play()
    assert res == PLAYBACK_FINISHED
    
    # Probes: 10 sleeps of 2 ms
    assert len(sleeper.sleeps) >= 10
    # The first 10 sleeps should be exactly 2ms (0.002s)
    for s in sleeper.sleeps[:10]:
        assert s == 0.002
        
    # Derived effective threshold:
    # wake_error = 2300 - 2000 = 300 us
    # effective_spin_threshold_us = clamp(300 + 100, 700, 3000) = 700 us
    assert engine.effective_spin_threshold_us == 700
    assert engine.current_spin_threshold_us == 700

    # Verify options recorded
    opts = engine.telemetry.runtime_options
    assert opts.get("effective_spin_threshold_us") == 700
    assert opts.get("enable_adaptive_spin") is True
    probe_errors = opts.get("probe_wake_errors_us")
    assert isinstance(probe_errors, list)
    assert len(probe_errors) == 10


def test_probes_complete_before_perf_anchor() -> None:
    # Verify strict clock anchor ordering: probe sleeps happen BEFORE start_perf is captured
    class TracingClock(FakeClock):
        def __init__(self) -> None:
            super().__init__()
            self.trace: list[str] = []

        def now_us(self) -> int:
            self.trace.append(f"now_us_at_{self.time_us}")
            return self.time_us

    class TracingSleeper:
        def __init__(self, clock: TracingClock) -> None:
            self.clock = clock

        def sleep(self, seconds: float) -> None:
            self.clock.trace.append(f"sleep_for_{int(seconds * 1_000_000)}")
            self.clock.time_us += max(1, int(seconds * 1_000_000))

    clock = TracingClock()
    sleeper = TracingSleeper(clock)
    backend = TimedBackend(clock)
    song = Song(name="test_order", notes=())
    actions = (KeyAction(kind=ActionKind.DOWN, scan_codes=(ScanCode(1),), at_us=Microseconds(50000), reason="d1"),)

    engine = PlaybackEngine(
        song=song,
        actions=actions,
        backend=backend,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=sleeper,
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        use_dispatch_thread=False,
        enable_adaptive_spin=True,
        wait_strategy=TeleportSpinStrategy(),
    )

    engine.play()

    # The trace should start with 10 repetitions of now_us -> sleep_for_2000 -> now_us (the probe loops),
    # followed by the now_us call for capturing start_perf (anchor).
    # Specifically, the 10th sleep must be followed by a now_us for the 10th loop end, then some options recording,
    # and then the now_us that instantiates PlaybackState(start_perf=...).
    # Let's assert that the first 10 sleep events appear BEFORE the now_us that establishes start_perf.
    
    # Let's find index of sleep events using exact string matching
    sleep_indices = [i for i, event in enumerate(clock.trace) if event == "sleep_for_2000"]
    assert len(sleep_indices) == 10
    
    # The PlaybackState start_perf is captured after the 10th probe.
    # The 10th probe loop does:
    #   t0 = self.clock.now_us()     (now_us_at_18000)
    #   realtime_sleeper.sleep(0.002)(sleep_for_2000)
    #   t1 = self.clock.now_us()     (now_us_at_20000)
    # The next now_us call in the trace will be capturing start_perf (or inside telemetry record if clock was used).
    # In any case, all 10 "sleep_for_2000" events must have completed before the state epoch.
    last_probe_sleep_idx = sleep_indices[-1]
    
    # Let's verify that the trace after the last probe sleep contains the playback dispatch loops (which sleep for other durations, or do sends)
    # and no more probe sleeps of 2000.
    remaining_trace = clock.trace[last_probe_sleep_idx + 1:]
    assert not any(event == "sleep_for_2000" for event in remaining_trace)
