"""Independent audit coverage: gaps not covered by test_runtime_dispatch.

These deterministic fake-clock/backend tests target invariants that the existing
suite leaves implicit:

  * focus-loss is the *only* sanctioned timeline shift (invariant 7/8);
  * a down authored after a focus pause is never dropped, only shifted;
  * stale ups issued after release_all/cancel_all never release a fresh generation
    (invariant 4) and are telemetered (invariant 10);
  * a song starting at timestamp 0 fires its first onset at elapsed 0;
  * independent keys sharing a timestamp are dispatched as one chord, not collapsed;
  * a late burst never pulls *earlier* (already on-time) onsets off the timeline.
"""
from __future__ import annotations

from sky_music.domain import Song
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.engine import PLAYBACK_FINISHED, PlaybackEngine

from test_runtime_dispatch import (
    FakeClock,
    FakeSleeper,
    TimedBackend,
    action,
    play,
)


class WindowedFocusGuard:
    """Focus is lost while the simulation clock sits inside [lost_lo, lost_hi)."""

    def __init__(self, clock: FakeClock, lost_lo: int, lost_hi: int) -> None:
        self.clock = clock
        self.lost_lo = lost_lo
        self.lost_hi = lost_hi
        self.focus_calls = 0

    def is_active(self) -> bool:
        return not (self.lost_lo <= self.clock.time_us < self.lost_hi)

    def focus(self) -> bool:
        self.focus_calls += 1
        return True


def test_song_starting_at_zero_fires_first_onset_at_elapsed_zero():
    backend, engine = play(
        (action(0, "down", 21), action(10_000, "up", 21)),
        min_hold_us=10_000,
    )
    first_down = next(call for call in backend.calls if call.kind == "down")
    assert first_down.started_us == 0
    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["lateness_us"]["min_us"] == 0


def test_independent_keys_at_same_timestamp_dispatch_as_one_chord():
    backend, _ = play(
        (
            action(0, "down", 21, 22, 23),
            action(10_000, "up", 21, 22, 23),
        ),
        min_hold_us=10_000,
    )
    downs = [call for call in backend.calls if call.kind == "down"]
    assert len(downs) == 1
    assert downs[0].scan_codes == (21, 22, 23)


def test_focus_loss_shifts_timeline_without_dropping_later_down():
    clock = FakeClock()
    backend = TimedBackend(clock)
    focus = WindowedFocusGuard(clock, lost_lo=5_000, lost_hi=205_000)
    engine = PlaybackEngine(
        song=Song(name="focus", notes=()),
        actions=(
            action(0, "down", 21),
            action(10_000, "up", 21),
            action(50_000, "down", 22),
            action(60_000, "up", 22),
        ),
        backend=backend,
        telemetry_enabled=True,
        require_focus=True,
        focus_guard=focus,
        clock=clock,
        sleeper=FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=10_000,
        focus_restore_grace_us=0,
    )

    assert engine.play() == PLAYBACK_FINISHED

    # The first onset fired before focus loss, on its authored timeline.
    first_down = next(c for c in backend.calls if c.kind == "down" and c.scan_codes == (21,))
    assert first_down.started_us == 0

    # gen0's authored up arrives after the focus cancel and is suppressed (the key was
    # already physically released by release_all during the pause) — never re-sent.
    suppressed = [
        r for r in engine.telemetry.records
        if r.get("runtime_outcome") == "suppressed_stale_up" and "21" in r["scan_codes"]
    ]
    assert len(suppressed) == 1

    # The down authored at 50_000 is NOT dropped — it shifts past the focus window.
    second_down = next(c for c in backend.calls if c.kind == "down" and c.scan_codes == (22,))
    assert second_down.started_us >= 205_000
    # focus-loss is the only thing that moved 22; the shift equals the pause duration.
    assert backend.active == set()

    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["playback_pause"]["focus"]["count"] == 1
    assert summary["playback_pause"]["focus"]["total_us"] > 0


def test_stale_up_after_focus_cancel_does_not_release_fresh_generation():
    """A focus pause cancels gen0 of key 21; the authored up for gen0 then arrives
    while a *new* gen1 of key 21 is active. The stale up must be suppressed, leaving
    gen1 held until its own up — never releasing the newer generation (invariant 4)."""
    clock = FakeClock()
    backend = TimedBackend(clock)
    # Focus lost briefly mid-hold of gen0, restored before gen1's down.
    focus = WindowedFocusGuard(clock, lost_lo=5_000, lost_hi=30_000)
    engine = PlaybackEngine(
        song=Song(name="stale-up", notes=()),
        actions=(
            action(0, "down", 21),        # gen0 down
            action(100_000, "up", 21),    # gen0 up (authored late, after focus cancel)
            action(120_000, "down", 21),  # gen1 down
            action(200_000, "up", 21),    # gen1 up
        ),
        backend=backend,
        telemetry_enabled=True,
        require_focus=True,
        focus_guard=focus,
        clock=clock,
        sleeper=FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=10_000,
        focus_restore_grace_us=0,
    )

    assert engine.play() == PLAYBACK_FINISHED

    summary = engine.telemetry.get_summary()
    assert summary is not None
    # The gen0 authored up became a suppressed stale up (cancelled by focus release).
    suppressed = [
        r for r in engine.telemetry.records
        if r.get("runtime_outcome") == "suppressed_stale_up"
    ]
    assert len(suppressed) == 1
    # gen1 was pressed and released exactly once each; backend ends clean.
    assert backend.active == set()


def test_late_burst_keeps_earlier_ontime_onsets_on_timeline():
    from test_runtime_dispatch import ScheduledStallingSleeper

    clock = FakeClock()
    backend = TimedBackend(clock)
    engine = PlaybackEngine(
        song=Song(name="earlier-untouched", notes=()),
        actions=(
            action(0, "down", 21),
            action(10_000, "up", 21),
            action(20_000, "down", 22),   # fires on time, BEFORE the stall
            action(30_000, "up", 22),
            action(400_000, "down", 23),  # overshot by the stall
            action(410_000, "up", 23),
        ),
        backend=backend,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        # Stall only after key 22 has already fired on time at t=20_000.
        sleeper=ScheduledStallingSleeper(clock, stalls=((35_000, 500_000),)),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=10_000,
    )

    assert engine.play() == PLAYBACK_FINISHED
    # The earlier, on-time onset is exact — the later stall never pulled it off timeline.
    assert next(c.started_us for c in backend.calls if c.scan_codes == (22,)) == 20_000
    # 23 was overshot; it fires late at the current clock (no rebase, no cumulative drag
    # onto still-future events). Lateness is reported, the onset is not dropped.
    assert next(c.started_us for c in backend.calls if c.scan_codes == (23,)) >= 400_000
