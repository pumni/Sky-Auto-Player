"""Focus-restore grace-window pause accounting.

Extracted from the former ``test_reprobe_pause.py`` (the reprobe half of which tested the
now-deleted mid-play spin-threshold re-probe). This half exercises still-live behaviour:
on focus regain, ``_process_wait_states`` sleeps the ``focus_restore_grace_us`` window,
releases keys, then closes the "focus" pause interval so the whole paused span accrues to
``pause_time_us`` exactly once (never to playback time).
"""

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

    def poll(self) -> str | None:
        if not self.commands:
            return None
        return self.commands.pop(0)


def test_focus_restore_grace_absorbed_into_pause_time() -> None:
    song = Song(name="GraceTest", notes=(Note(time_ms=Millis(0), key=NoteKey("Key0")),))
    sched = build_key_actions(song)

    clock = FakeClock(start_us=60_000)
    sleeper = FakeSleeper(clock)
    engine = PlaybackEngine(
        song=song,
        actions=sched.actions,
        backend=DryRunBackend(),
        controls=SequenceControls([]),
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=sleeper,
        focus_restore_grace_us=50_000,
    )
    state = PlaybackState(start_perf=0)
    state.enter_pause("focus", 10_000)

    waiting, cmd = engine._process_wait_states(
        state, first_action_executed=True, total_time_us=1.0
    )

    assert waiting is False
    assert cmd is None
    assert not state.has_pause_reason("focus")
    # Grace loop advances the clock from 60_000 by the 50 ms window -> 110_000; the paused
    # interval opened at 10_000, so 110_000 - 10_000 = 100_000 us accrues to pause_time once.
    assert state.pause_time_us == 100_000
