"""Single-owner pause state machine tests (finding A2).

Phase 1 of docs/2026-07_core-dispatch-refactor-and-isolation-plan.md §4.2.
"""

from __future__ import annotations

from sky_music.orchestration.dispatch_loop import PlaybackState


class FakeClock:
    def __init__(self, time_us: int = 0) -> None:
        self.time_us = time_us

    def now_us(self) -> int:
        return self.time_us


def test_focus_then_manual_pause_no_double_count() -> None:
    """focus-lost → manual pause → manual unpause → focus regain counts once.

    Timeline:
      t=10_000 lose focus (open interval)
      t=20_000 manual pause (still one interval)
      t=50_000 manual unpause (interval still open under focus)
      t=80_000 focus regain (close interval)

    Expected pause_time_us == 70_000 (one contiguous 10k→80k), not 90_000.
    """
    state = PlaybackState(start_perf=0)
    state.enter_pause("focus", 10_000)
    state.enter_pause("manual", 20_000)
    closed_manual = state.exit_pause("manual", 50_000)
    assert closed_manual is None, "manual exit while focus active must not accumulate"
    closed_focus = state.exit_pause("focus", 80_000)
    assert closed_focus is not None
    duration_us, attribution = closed_focus
    assert duration_us == 70_000
    assert attribution == "focus"  # first reason that opened the interval
    assert state.pause_time_us == 70_000
    assert not state.is_paused()


def test_elapsed_frozen_while_double_paused() -> None:
    """With both reasons active, get_elapsed_us is constant as the clock advances."""
    clock = FakeClock(5_000)
    state = PlaybackState(start_perf=0)
    state.enter_pause("focus", 10_000)
    state.enter_pause("manual", 20_000)
    frozen = state.get_elapsed_us(clock)
    clock.time_us = 50_000
    assert state.get_elapsed_us(clock) == frozen
    clock.time_us = 100_000
    assert state.get_elapsed_us(clock) == frozen
    # Frozen at interval start, not decreasing with wall time
    assert frozen == 10_000


def test_single_reason_roundtrip_unchanged() -> None:
    """Plain manual and plain focus produce the same pause_time_us as the old dual model."""
    # Manual only: pause at 10k, unpause at 40k → 30k
    manual = PlaybackState(start_perf=0)
    manual.enter_pause("manual", 10_000)
    closed = manual.exit_pause("manual", 40_000)
    assert closed == (30_000, "manual")
    assert manual.pause_time_us == 30_000

    # Focus only: lose at 5k, regain at 55k → 50k
    focus = PlaybackState(start_perf=0)
    focus.enter_pause("focus", 5_000)
    closed_f = focus.exit_pause("focus", 55_000)
    assert closed_f == (50_000, "focus")
    assert focus.pause_time_us == 50_000
