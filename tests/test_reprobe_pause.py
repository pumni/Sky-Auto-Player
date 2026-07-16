from __future__ import annotations

from sky_music.domain import Millis, Note, NoteKey, Song
from sky_music.domain.scheduler import build_key_actions
from sky_music.infrastructure.backend import DryRunBackend
from sky_music.orchestration.engine import PlaybackEngine, PlaybackState


class FakeClock:
    def __init__(self, start_us: int = 0) -> None:
        self.time_us = start_us

    def now_us(self) -> int:
        return self.time_us

    def sleep_us(self, duration_us: int) -> None:
        self.time_us += duration_us


class FakeSleeper:
    def __init__(self, clock: FakeClock) -> None:
        self.clock = clock
        self.sleeps: list[float] = []

    def sleep(self, seconds: float) -> None:
        self.sleeps.append(seconds)
        self.clock.sleep_us(max(1, int(seconds * 1_000_000)))


class SequenceControls:
    def __init__(self, commands: list[str | None]) -> None:
        self.commands = list(commands)
        self.enabled = True

    def poll(self) -> str | None:
        if not self.commands:
            return None
        return self.commands.pop(0)


def test_reprobe_pause_duration_absorption() -> None:
    song = Song(name="ReprobeTest", notes=(Note(time_ms=Millis(0), key=NoteKey("Key0")),))
    sched = build_key_actions(song)
    
    # Scenario A: Playback without reprobe
    clock_a = FakeClock(start_us=60_000)
    sleeper_a = FakeSleeper(clock_a)
    engine_a = PlaybackEngine(
        song=song,
        actions=sched.actions,
        backend=DryRunBackend(),
        controls=SequenceControls([]),
        telemetry_enabled=True,
        require_focus=False,
        clock=clock_a,
        sleeper=sleeper_a,
        focus_restore_grace_us=50_000,
        enable_reprobe=False,
    )
    state_a = PlaybackState(start_perf=0)
    state_a.enter_pause("focus", 10_000)

    waiting_a, cmd_a = engine_a._process_wait_states(state_a, first_action_executed=True, total_time_us=1.0)
    assert waiting_a is False
    assert cmd_a is None
    assert not state_a.has_pause_reason("focus")
    # Clock advanced by grace period (50ms) -> 110_000
    # Pause duration = 110_000 - 10_000 = 100_000
    assert state_a.pause_time_us == 100_000

    # Scenario B: Playback with reprobe enabled
    # The reprobe takes 10 sleeps of 2ms (20ms total)
    clock_b = FakeClock(start_us=60_000)
    sleeper_b = FakeSleeper(clock_b)
    engine_b = PlaybackEngine(
        song=song,
        actions=sched.actions,
        backend=DryRunBackend(),
        controls=SequenceControls([]),
        telemetry_enabled=True,
        require_focus=False,
        clock=clock_b,
        sleeper=sleeper_b,
        focus_restore_grace_us=50_000,
        enable_reprobe=True,
    )
    state_b = PlaybackState(start_perf=0)
    state_b.enter_pause("focus", 10_000)

    waiting_b, cmd_b = engine_b._process_wait_states(state_b, first_action_executed=True, total_time_us=1.0)
    assert waiting_b is False
    assert cmd_b is None
    assert not state_b.has_pause_reason("focus")
    # Clock advanced by grace period (50ms) + reprobe (20ms) -> 130_000
    # Pause duration = 130_000 - 10_000 = 120_000
    assert state_b.pause_time_us == 120_000
    
    # Check that spin threshold was updated (default fake sleeper will have 0 errors, so new_threshold=700 due to floor)
    assert engine_b._compat_dispatch_loop().spin_threshold_us == 700
    
    # Check telemetry options record reprobe keys
    opts = engine_b.telemetry.runtime_options
    assert "reprobe_wake_errors_us" in opts
    assert opts.get("reprobe_effective_spin_threshold_us") == 700
