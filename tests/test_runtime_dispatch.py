from __future__ import annotations

from dataclasses import dataclass

from sky_music.domain import Song
from sky_music.domain.scheduler_types import KeyAction, Microseconds, ScanCode
from sky_music.infrastructure.backend import BackendHealth, InputSendResult, ReleaseAllOutcome
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.engine import PLAYBACK_FINISHED, PLAYBACK_QUIT, PlaybackEngine
from sky_music.orchestration.runtime_dispatch import compile_runtime_intents
from sky_music.orchestration.telemetry import TelemetryLogger
import sky_music.orchestration.engine as engine_module


class FakeClock:
    def __init__(self) -> None:
        self.time_us = 0

    def now_us(self) -> int:
        return self.time_us


class FakeSleeper:
    def __init__(self, clock: FakeClock) -> None:
        self.clock = clock

    def sleep(self, seconds: float) -> None:
        self.clock.time_us += max(1, int(seconds * 1_000_000))


class OneShotStallingSleeper(FakeSleeper):
    def __init__(self, clock: FakeClock, stall_us: int) -> None:
        super().__init__(clock)
        self.stall_us = stall_us
        self.stalled = False

    def sleep(self, seconds: float) -> None:
        if not self.stalled:
            self.clock.time_us += self.stall_us
            self.stalled = True
            return
        super().sleep(seconds)


class ScheduledStallingSleeper(FakeSleeper):
    def __init__(self, clock: FakeClock, stalls: tuple[tuple[int, int], ...]) -> None:
        super().__init__(clock)
        self.stalls = list(stalls)

    def sleep(self, seconds: float) -> None:
        if self.stalls and self.clock.time_us >= self.stalls[0][0]:
            _, stall_us = self.stalls.pop(0)
            self.clock.time_us += stall_us
            return
        super().sleep(seconds)


class TimeCommandControls:
    def __init__(self, clock: FakeClock, commands: tuple[tuple[int, str], ...]) -> None:
        self.clock = clock
        self.commands = list(commands)

    def poll(self) -> str | None:
        if not self.commands:
            return None
        at_us, command = self.commands[0]
        if self.clock.time_us < at_us:
            return None
        self.commands.pop(0)
        return command


@dataclass(frozen=True, slots=True)
class TimedCall:
    kind: str
    scan_codes: tuple[int, ...]
    started_us: int
    completed_us: int


class TimedBackend:
    def __init__(self, clock: FakeClock, send_duration_us: int = 0) -> None:
        self.clock = clock
        self.send_duration_us = send_duration_us
        self.active: set[int] = set()
        self.calls: list[TimedCall] = []

    def _finish(self, kind: str, scan_codes: tuple[int, ...]) -> None:
        started_us = self.clock.time_us
        self.clock.time_us += self.send_duration_us
        self.calls.append(TimedCall(kind, scan_codes, started_us, self.clock.time_us))

    def key_down(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        sent = tuple(scan_code for scan_code in scan_codes if scan_code not in self.active)
        skipped = tuple(scan_code for scan_code in scan_codes if scan_code in self.active)
        if sent:
            self._finish("down", sent)
            self.active.update(sent)
        return InputSendResult(sent=sent, skipped_duplicates=skipped, success=True)

    def key_up(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        sent = tuple(scan_code for scan_code in scan_codes if scan_code in self.active)
        skipped = tuple(scan_code for scan_code in scan_codes if scan_code not in self.active)
        if sent:
            self._finish("up", sent)
            self.active.difference_update(sent)
        return InputSendResult(sent=sent, skipped_duplicates=skipped, success=True)

    def release_all(self) -> ReleaseAllOutcome:
        attempted = tuple(sorted(self.active))
        self.active.clear()
        return ReleaseAllOutcome(
            attempted=attempted,
            released_successfully=True,
            stuck_keys=(),
            verification_inconclusive=False,
        )

    def get_health(self) -> BackendHealth:
        return BackendHealth(
            active_count=len(self.active),
            possibly_active_count=0,
            failed_release_count=0,
            last_error=None,
        )


def action(at_us: int, kind: str, *scan_codes: int) -> KeyAction:
    return KeyAction(
        kind=kind,  # type: ignore[arg-type]
        scan_codes=tuple(ScanCode(scan_code) for scan_code in scan_codes),
        at_us=Microseconds(at_us),
        reason="test",
    )


def play(
    actions: tuple[KeyAction, ...],
    *,
    min_hold_us: int,
    send_duration_us: int = 0,
) -> tuple[TimedBackend, PlaybackEngine]:
    clock = FakeClock()
    backend = TimedBackend(clock, send_duration_us=send_duration_us)
    engine = PlaybackEngine(
        song=Song(name="runtime", notes=()),
        actions=actions,
        backend=backend,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=min_hold_us,
    )
    assert engine.play() == PLAYBACK_FINISHED
    return backend, engine


def test_runtime_compiler_pairs_overlapping_same_key_generations_fifo():
    schedule = compile_runtime_intents(
        (
            action(0, "down", 21),
            action(5, "down", 21),
            action(10, "up", 21),
            action(15, "up", 21),
        )
    )

    generation_ids = [
        batch.intents[0].generation_id
        for batch in schedule.batches
    ]
    assert generation_ids == [0, 1, 0, 1]


def test_runtime_compiler_preserves_action_batches_and_timestamps():
    actions = (
        action(1_000, "down", 21, 22),
        action(2_000, "up", 21, 22),
    )

    schedule = compile_runtime_intents(actions)

    assert [
        (batch.kind, batch.scheduled_us, tuple(intent.scan_code for intent in batch.intents))
        for batch in schedule.batches
    ] == [
        ("down", 1_000, (21, 22)),
        ("up", 2_000, (21, 22)),
    ]


def test_release_guard_anchors_hold_to_down_dispatch_start():
    # The visibility floor is measured start-to-start: the up may fire at the authored time even
    # though the down's SendInput took 300us, because the observed hold (up_start - down_start)
    # already equals min_hold. Anchoring to the down's *completion* instead would over-hold by one
    # send_duration and break same-key repeats authored just above min_hold.
    backend, engine = play(
        (action(0, "down", 21), action(1_000, "up", 21)),
        min_hold_us=1_000,
        send_duration_us=300,
    )

    assert [(call.kind, call.started_us, call.completed_us) for call in backend.calls] == [
        ("down", 0, 300),
        ("up", 1_000, 1_300),
    ]
    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["note_hold_duration_us"]["min_us"] == 1_000
    assert summary["note_hold_duration_us"]["p50_us"] == 1_000
    assert summary["confirmed_hold_lower_bound_us"]["min_us"] == 1_300
    assert summary["confirmed_hold_lower_bound_us"]["p50_us"] == 1_300
    assert summary["confirmed_hold_shortfall_count"] == 0
    # The authored up already satisfies the floor, so no runtime deferral is needed.
    assert summary["deferred_release_count"] == 0


def test_hold_floor_preserved_when_thread_stalls_during_hold():
    # A thread stall between the down and the authored up must still leave the observed hold at or
    # above min_hold. This is the legitimate lateness protection the guard exists for (distinct from
    # the spurious send_duration over-hold removed above).
    clock = FakeClock()
    backend = TimedBackend(clock, send_duration_us=0)
    engine = PlaybackEngine(
        song=Song(name="stall-hold", notes=()),
        actions=(action(0, "down", 21), action(1_000, "up", 21)),
        backend=backend,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        # Stall 400us right at the start so the first down dispatches at t=400.
        sleeper=OneShotStallingSleeper(clock, stall_us=400),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=1_000,
    )
    engine.play()
    down = next(c for c in backend.calls if c.kind == "down")
    up = next(c for c in backend.calls if c.kind == "up")
    assert up.started_us - down.started_us >= 1_000


def test_same_key_repeat_at_min_hold_floor_presses_on_time_with_send_latency():
    # Regression for the intermittent "missing notes" symptom on local_precise: a same-key repeat
    # whose authored interval equals min_hold is feasible per the scheduler, so the runtime must
    # release the first note then press the repeat on time — never drop it — even when SendInput
    # has non-zero latency.
    backend, engine = play(
        (
            action(0, "down", 21),
            action(1_000, "up", 21),
            action(1_000, "down", 21),
            action(2_000, "up", 21),
        ),
        min_hold_us=1_000,
        send_duration_us=300,
    )

    downs = [c for c in backend.calls if c.kind == "down" and c.scan_codes == (21,)]
    assert len(downs) == 2  # the repeat is NOT dropped
    assert downs[0].started_us == 0
    # The repeat presses on the authored timeline (1_000). It may trail by at most one key_up
    # SendInput latency because release-then-press is inherently sequential on one thread; with the
    # 300us fake send that is 1_300, on real hardware it is ~tens of microseconds.
    assert 1_000 <= downs[1].started_us <= 1_000 + 300
    # gen1 observed a full min_hold before being released for the repeat.
    up1 = next(c for c in backend.calls if c.kind == "up" and c.scan_codes == (21,))
    assert up1.started_us - downs[0].started_us >= 1_000
    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["runtime_conflict_dropped_down_count"] == 0


def test_scheduler_feasible_repeat_is_runtime_feasible_invariant():
    # Contract: interval >= min_hold (scheduler-feasible) must NEVER be dropped by the runtime,
    # for any SendInput latency. This is the invariant that keeps the two layers in agreement.
    min_hold = 6_945  # local_precise @144fps
    for extra in (0, 50, 150, 300, 1_000):
        for send_duration_us in (0, 100, 250):
            interval = min_hold + extra
            backend, _ = play(
                (
                    action(0, "down", 21),
                    action(min_hold, "up", 21),
                    action(interval, "down", 21),
                    action(interval + min_hold, "up", 21),
                ),
                min_hold_us=min_hold,
                send_duration_us=send_duration_us,
            )
            downs = [c for c in backend.calls if c.kind == "down"]
            assert len(downs) == 2, (
                f"dropped a scheduler-feasible repeat: interval=min_hold+{extra}, "
                f"send_duration={send_duration_us}"
            )


def _build_repeat_song(name, intervals, *, key=7, reps=12, block_gap=1500):
    """Mirror tests/make_test_song.py repeat_clean()/repeat_floor() shape as a domain Song."""
    from sky_music.domain import Note
    from sky_music.domain.domain import Millis, NoteKey

    notes, t = [], 0
    for interval in intervals:
        for _ in range(reps):
            notes.append(Note(time_ms=Millis(t), key=NoteKey(f"Key{key}")))
            t += interval
        t += block_gap
    return Song(name=name, notes=tuple(notes))


def test_repeat_clean_ground_truth_song_never_drops_end_to_end():
    # End-to-end gate tied to the real scheduler + frame policy: the Tier-2 ground-truth probe
    # (TEST_repeat_clean_*, headroom far above jitter AND above the game re-trigger wall) must emit
    # 100% of its same-key downs even under an EXAGGERATED 300us SendInput latency (>~5x real). If
    # this fails, an in-game audio onset count can no longer be trusted as a verdict on the game,
    # and a same-key repeat the scheduler called feasible is being lost in the runtime again.
    from sky_music.domain.scheduler import build_key_actions
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.layouts import SKY_15_KEY_PROFILE

    cases = (
        (144, (20, 24, 30, 40, 55, 70)),
        (60, (28, 34, 42, 55, 75, 100)),
    )
    for fps, intervals in cases:
        song = _build_repeat_song(f"clean_{fps}", intervals)
        policy = FrameTimingPolicy.local_precise(fps=fps)
        sched = build_key_actions(song, policy=policy)
        assert sched.impossible_same_key_repeats == 0  # the probe must be fully feasible

        clock = FakeClock()
        backend = TimedBackend(clock, send_duration_us=300)
        engine = PlaybackEngine(
            song=song,
            actions=sched.actions,
            backend=backend,
            telemetry_enabled=True,
            require_focus=False,
            clock=clock,
            sleeper=FakeSleeper(clock),
            sleep_policy=SleepPolicy(spin_threshold_us=-1),
            min_hold_us=int(policy.min_hold_us),
        )
        engine.play()
        summary = engine.telemetry.get_summary()
        assert summary is not None
        assert summary["sent_down_count"] == len(song.notes), f"fps={fps}"
        assert summary["runtime_conflict_dropped_down_count"] == 0, f"fps={fps}"
        assert summary["confirmed_hold_shortfall_count"] == 0, f"fps={fps}"


def test_deferred_release_does_not_delay_unrelated_down():
    # The authored hold (0->200) is shorter than min_hold (1_000), so gen21's release is genuinely
    # deferred to t=1_000. An unrelated down on key 22 at t=500 must still fire on time and must NOT
    # be dragged to the deferred-release deadline. send_duration=0 isolates the deferral effect from
    # SendInput latency.
    backend, _ = play(
        (
            action(0, "down", 21),
            action(200, "up", 21),
            action(500, "down", 22),
            action(2_000, "up", 22),
        ),
        min_hold_us=1_000,
        send_duration_us=0,
    )

    assert next(
        call.started_us for call in backend.calls if call.kind == "down" and call.scan_codes == (22,)
    ) == 500
    # The release is held back to the min_hold floor, independently of the unrelated down.
    assert next(
        call.started_us for call in backend.calls if call.kind == "up" and call.scan_codes == (21,)
    ) == 1_000


def test_dropped_generation_up_cannot_release_later_generation():
    backend, engine = play(
        (
            action(0, "down", 21),
            action(5, "down", 21),
            action(10, "up", 21),
            action(15, "down", 21),
            action(20, "up", 21),
            action(25, "up", 21),
        ),
        min_hold_us=10,
    )

    assert [(call.kind, call.started_us) for call in backend.calls] == [
        ("down", 0),
        ("up", 10),
        ("down", 15),
        ("up", 25),
    ]
    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["runtime_conflict_dropped_down_count"] == 1


def test_mixed_chord_conflict_still_sends_playable_key():
    backend, engine = play(
        (
            action(0, "down", 21),
            action(5, "down", 21, 22),
            action(10, "up", 21),
            action(15, "up", 21, 22),
        ),
        min_hold_us=10,
    )

    assert any(call.kind == "down" and call.scan_codes == (22,) for call in backend.calls)
    assert not any(call.kind == "down" and call.scan_codes == (21, 22) for call in backend.calls)
    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["runtime_conflict_dropped_down_count"] == 1


def test_generation_status_counts_surface_released_conflict_and_cancelled_run():
    clock = FakeClock()
    backend = TimedBackend(clock)
    engine = PlaybackEngine(
        song=Song(name="generation-status", notes=()),
        actions=(
            action(0, "down", 21),
            action(1_000, "up", 21),
            action(2_000, "down", 22),
            action(2_100, "down", 22),
            action(10_000, "up", 22),
        ),
        backend=backend,
        controls=TimeCommandControls(clock, ((3_000, "pause"), (3_100, "quit"))),
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=1_000,
    )

    assert engine.play() == PLAYBACK_QUIT
    counts = engine._runtime_coordinator.generation_status_counts()
    assert counts["released"] == 1
    assert counts["dropped_conflict"] == 1
    assert counts["cancelled"] == 1

    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["released_count"] == 1
    assert summary["dropped_conflict_count"] == 1
    assert summary["cancelled_generation_count"] == 1
    assert summary["dropped_backend_count"] == 0
    assert summary["dropped_conflict_count"] == summary["runtime_conflict_dropped_down_count"]


def test_strict_runtime_conflict_stops_cleanly_and_releases_active_key():
    clock = FakeClock()
    backend = TimedBackend(clock, send_duration_us=300)
    engine = PlaybackEngine(
        song=Song(name="strict", notes=()),
        # interval 500 < min_hold 1000 => genuinely infeasible repeat (scheduler flags this as an
        # impossible_repeat); strict runtime policy must abort rather than overlap the keys.
        actions=(
            action(0, "down", 21),
            action(500, "down", 21),
            action(1_000, "up", 21),
            action(1_500, "up", 21),
        ),
        backend=backend,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=1_000,
        same_key_conflict_policy="strict",
    )

    assert engine.play() == PLAYBACK_QUIT
    assert backend.active == set()


def test_non_dispatch_records_do_not_pollute_lateness_statistics():
    logger = TelemetryLogger("truthful", enabled=True)
    logger.record(
        event_index=0,
        kind="down",
        scheduled_us=0,
        actual_us=10,
        lateness_us=10,
        send_duration_us=20,
        scan_codes=(21,),
        sent_scan_codes=(21,),
        reason="sent",
    )
    logger.record(
        event_index=1,
        kind="down",
        scheduled_us=0,
        actual_us=10_000,
        lateness_us=10_000,
        send_duration_us=0,
        scan_codes=(21,),
        sent_scan_codes=(),
        runtime_outcome="dropped_conflict",
        reason="drop",
    )

    summary = logger.get_summary()

    assert summary is not None
    assert summary["lateness_us"]["max_us"] == 10
    assert summary["attempted_dispatches"] == 1


def test_deferred_release_does_not_pollute_scheduler_lateness_statistics():
    logger = TelemetryLogger("deferred", enabled=True)
    logger.record(
        event_index=0,
        kind="down",
        scheduled_us=0,
        actual_us=100,
        lateness_us=100,
        send_duration_us=20,
        scan_codes=(21,),
        sent_scan_codes=(21,),
        reason="onset",
    )
    logger.record(
        event_index=1,
        kind="up",
        scheduled_us=1_000,
        actual_us=11_000,
        lateness_us=10_000,
        send_duration_us=20,
        scan_codes=(21,),
        sent_scan_codes=(21,),
        runtime_outcome="deferred_release",
        deferred_by_us=10_000,
        reason="release",
    )

    summary = logger.get_summary()

    assert summary is not None
    assert summary["lateness_us"]["max_us"] == 100
    assert summary["deferred_release_count"] == 1
    assert summary["release_deferral_us"]["max_us"] == 10_000


def test_telemetry_summary_reports_lateness_threshold_counts():
    logger = TelemetryLogger("lateness-thresholds", enabled=True)
    for event_index, lateness_us in enumerate((100, 3_000, 6_000, 11_000)):
        logger.record(
            event_index=event_index,
            kind="down",
            scheduled_us=event_index * 100_000,
            actual_us=event_index * 100_000 + lateness_us,
            lateness_us=lateness_us,
            send_duration_us=10,
            scan_codes=(21 + event_index,),
            sent_scan_codes=(21 + event_index,),
            reason="sent",
        )

    summary = logger.get_summary()

    assert summary is not None
    assert summary["lateness_us"]["over_2ms"] == 3
    assert summary["lateness_us"]["over_5ms"] == 2
    assert summary["lateness_us"]["over_10ms"] == 1
    assert summary["lateness_us"]["max_us"] == 11_000


def test_telemetry_reports_backend_dropped_downs_and_catch_up_bursts():
    logger = TelemetryLogger("catch-up", enabled=True)
    logger.record(
        event_index=0,
        kind="down",
        scheduled_us=0,
        actual_us=0,
        lateness_us=0,
        send_duration_us=10,
        scan_codes=(21, 22),
        sent_scan_codes=(21,),
        skipped_scan_codes=(22,),
        reason="onset",
    )
    for event_index, scheduled_us in enumerate((100_000, 110_000, 120_000), start=1):
        logger.record(
            event_index=event_index,
            kind="down",
            scheduled_us=scheduled_us,
            actual_us=150_000 + event_index,
            lateness_us=150_000 + event_index - scheduled_us,
            send_duration_us=10,
            scan_codes=(22 + event_index,),
            sent_scan_codes=(22 + event_index,),
            reason="onset",
        )

    summary = logger.get_summary()

    assert summary is not None
    assert summary["runtime_backend_dropped_down_count"] == 1
    assert summary["catch_up_bursts"] == {
        "count": 1,
        "down_dispatch_count": 3,
        "max_collapsed_dispatches": 3,
        "max_authored_span_us": 20_000,
    }


def test_telemetry_reports_down_timeline_drift_and_pause_causes():
    logger = TelemetryLogger("drift", enabled=True)
    for event_index, actual_us in enumerate((10, 1_020, 2_040)):
        logger.record(
            event_index=event_index,
            kind="down",
            scheduled_us=event_index * 1_000,
            actual_us=actual_us,
            lateness_us=actual_us - event_index * 1_000,
            send_duration_us=5,
            scan_codes=(21 + event_index,),
            sent_scan_codes=(21 + event_index,),
            reason="sent",
        )
    logger.record_pause("focus", 25_000)

    summary = logger.get_summary()

    assert summary is not None
    assert summary["down_timeline_drift_us"] == 30
    assert summary["playback_pause"]["focus"] == {
        "count": 1,
        "total_us": 25_000,
        "max_us": 25_000,
    }


def test_runtime_compilation_happens_before_playback_clock_starts(monkeypatch):
    clock = FakeClock()
    backend = TimedBackend(clock)
    real_compile = engine_module.compile_runtime_intents

    def slow_compile(actions: tuple[KeyAction, ...]):
        clock.time_us += 25_000
        return real_compile(actions)

    monkeypatch.setattr(engine_module, "compile_runtime_intents", slow_compile)
    engine = PlaybackEngine(
        song=Song(name="precompiled", notes=()),
        actions=(action(0, "down", 21), action(1_000, "up", 21)),
        backend=backend,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=1_000,
    )

    assert clock.time_us == 25_000
    assert engine.play() == PLAYBACK_FINISHED
    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["lateness_us"]["min_us"] == 0
    assert backend.calls[0].started_us == 25_000


def test_late_burst_never_shifts_the_absolute_music_timeline():
    clock = FakeClock()
    backend = TimedBackend(clock)
    engine = PlaybackEngine(
        song=Song(name="stalled", notes=()),
        actions=(
            action(0, "down", 21),
            action(10_000, "up", 21),
            action(100_000, "down", 22),
            action(110_000, "up", 22),
            action(500_000, "down", 23),
            action(510_000, "up", 23),
        ),
        backend=backend,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=OneShotStallingSleeper(clock, stall_us=250_000),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=10_000,
    )

    assert engine.play() == PLAYBACK_FINISHED
    assert next(call.started_us for call in backend.calls if call.scan_codes == (23,)) == 500_000


def test_repeated_stalls_do_not_accumulate_musical_slowdown():
    clock = FakeClock()
    backend = TimedBackend(clock)
    engine = PlaybackEngine(
        song=Song(name="repeated-stalls", notes=()),
        actions=(
            action(0, "down", 21),
            action(10_000, "up", 21),
            action(100_000, "down", 22),
            action(110_000, "up", 22),
            action(300_000, "down", 23),
            action(310_000, "up", 23),
            action(600_000, "down", 24),
            action(610_000, "up", 24),
        ),
        backend=backend,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=ScheduledStallingSleeper(
            clock,
            stalls=((20_000, 120_000), (200_000, 120_000)),
        ),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=10_000,
    )

    assert engine.play() == PLAYBACK_FINISHED
    assert next(call.started_us for call in backend.calls if call.scan_codes == (24,)) == 600_000


def test_late_pulse_drop_policy_drops_expired_downs_without_rebasing_timeline():
    clock = FakeClock()
    backend = TimedBackend(clock)
    engine = PlaybackEngine(
        song=Song(name="expired-down", notes=()),
        actions=(
            action(10_000, "down", 21),
            action(20_000, "up", 21),
            action(100_000, "down", 22),
            action(110_000, "up", 22),
        ),
        backend=backend,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=OneShotStallingSleeper(clock, stall_us=25_001),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=10_000,
        late_pulse_drop_threshold_us=10_000,
    )

    assert engine.play() == PLAYBACK_FINISHED
    assert not any(call.kind == "down" and call.scan_codes == (21,) for call in backend.calls)
    assert next(call.started_us for call in backend.calls if call.scan_codes == (22,)) == 100_000

    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["expired_dropped_down_count"] == 1
    assert summary["runtime_backend_dropped_down_count"] == 0
    assert summary["down_timeline_drift_us"] == 0


def test_late_pulse_drop_policy_never_drops_late_releases():
    clock = FakeClock()
    backend = TimedBackend(clock)
    engine = PlaybackEngine(
        song=Song(name="late-release", notes=()),
        actions=(action(0, "down", 21), action(10_000, "up", 21)),
        backend=backend,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=ScheduledStallingSleeper(clock, stalls=((1_000, 50_000),)),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=10_000,
        late_pulse_drop_threshold_us=10_000,
    )

    assert engine.play() == PLAYBACK_FINISHED
    assert any(call.kind == "up" and call.scan_codes == (21,) for call in backend.calls)
    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["expired_dropped_down_count"] == 0
    assert summary["sent_up_count"] == 1


def test_late_pulse_drop_policy_keeps_down_exactly_at_threshold():
    clock = FakeClock()
    backend = TimedBackend(clock)
    engine = PlaybackEngine(
        song=Song(name="threshold-boundary", notes=()),
        actions=(action(10_000, "down", 21), action(30_000, "up", 21)),
        backend=backend,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=OneShotStallingSleeper(clock, stall_us=20_000),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=10_000,
        late_pulse_drop_threshold_us=10_000,
    )

    assert engine.play() == PLAYBACK_FINISHED
    assert any(call.kind == "down" and call.scan_codes == (21,) for call in backend.calls)
    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["expired_dropped_down_count"] == 0
