from __future__ import annotations

from dataclasses import dataclass

import pytest

import sky_music.orchestration.engine as engine_module
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
from sky_music.orchestration.engine import (
    PLAYBACK_FINISHED,
    PLAYBACK_QUIT,
    PlaybackEngine,
)
from sky_music.orchestration.runtime_dispatch import compile_runtime_intents
from sky_music.orchestration.telemetry import TelemetryLogger


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


class CaptureRenderer:
    def __init__(self) -> None:
        self.input_path_flags: list[bool] = []

    def render(
        self,
        elapsed: float,
        total: float,
        song_name: str,
        *,
        status: str = "playing",
        force: bool = False,
        input_path_degraded: bool = False,
        backend_health: BackendHealth | None = None,
    ) -> None:
        self.input_path_flags.append(input_path_degraded)

    def finish(self, message: str) -> None:
        return

    def update_counters(self, lateness_us: int, **kwargs: object) -> None:
        return


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
        self.release_all_calls: list[int] = []

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
        self.release_all_calls.append(self.clock.time_us)
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

    def set_clock(self, clock: object) -> None:
        return None

    def get_health(self) -> BackendHealth:
        return BackendHealth(
            active_count=len(self.active),
            possibly_active_count=0,
            failed_release_count=0,
            last_error=None,
        )

    def get_send_diagnostics(self) -> dict[str, int]:
        return {}


class AsymmetricTimedBackend(TimedBackend):
    def __init__(self, clock: FakeClock, *, down_duration_us: int, up_duration_us: int) -> None:
        super().__init__(clock)
        self.down_duration_us = down_duration_us
        self.up_duration_us = up_duration_us

    def _finish(self, kind: str, scan_codes: tuple[int, ...]) -> None:
        started_us = self.clock.time_us
        self.clock.time_us += self.down_duration_us if kind == "down" else self.up_duration_us
        self.calls.append(TimedCall(kind, scan_codes, started_us, self.clock.time_us))


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


def test_release_guard_anchors_hold_to_down_dispatch_completion():
    # The game-observed floor is completion-to-completion. The authored up at min_hold is therefore
    # held until down completion + min_hold. That wait is a true floor deferral
    # (release_not_before - scheduled = send_duration), so telemetry reports deferred_release.
    backend, engine = play(
        (action(0, "down", 21), action(1_000, "up", 21)),
        min_hold_us=1_000,
        send_duration_us=300,
    )

    assert [(call.kind, call.started_us, call.completed_us) for call in backend.calls] == [
        ("down", 0, 300),
        ("up", 1_300, 1_600),
    ]
    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["note_hold_duration_us"]["min_us"] == 1_300
    assert summary["note_hold_duration_us"]["p50_us"] == 1_300
    assert summary["observed_hold_us"]["min_us"] == 1_300
    assert summary["observed_hold_us"]["p50_us"] == 1_300
    assert summary["confirmed_hold_lower_bound_us"]["min_us"] == 1_600
    assert summary["confirmed_hold_lower_bound_us"]["p50_us"] == 1_600
    assert summary["confirmed_hold_shortfall_count"] == 0
    assert summary["deferred_release_count"] == 1


def test_observed_hold_never_below_one_frame_under_asymmetric_send_latency():
    min_hold = 1_000
    clock = FakeClock()
    backend = AsymmetricTimedBackend(clock, down_duration_us=250, up_duration_us=20)
    engine = PlaybackEngine(
        song=Song(name="asymmetric", notes=()),
        actions=(
            action(0, "down", 21),
            action(min_hold, "up", 21),
            action(4_000, "down", 22, 23),
            action(4_000 + min_hold, "up", 22, 23),
        ),
        backend=backend,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=min_hold,
        fps=1_000,
    )

    assert engine.play() == PLAYBACK_FINISHED
    down_completions: dict[int, list[int]] = {}
    observed: list[int] = []
    for call in backend.calls:
        for scan_code in call.scan_codes:
            if call.kind == "down":
                down_completions.setdefault(scan_code, []).append(call.completed_us)
            else:
                observed.append(call.completed_us - down_completions[scan_code].pop(0))

    assert observed
    assert all(hold_us >= min_hold for hold_us in observed)
    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["observed_hold_below_frame_count"] == 0


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


def test_repeat_below_min_hold_is_flagged_infeasible():
    # Without a fixed release-latency margin, the scheduler's feasibility floor is exactly min_hold:
    # a same-key repeat whose interval is below min_hold cannot preserve the hold and is flagged
    # infeasible; a repeat at/above min_hold is feasible.
    from sky_music.domain import Millis, Note, NoteKey
    from sky_music.domain.scheduler import ScheduleBuildError, build_key_actions
    from sky_music.domain.scheduler_types import FrameTimingPolicy, TimingPolicy

    # interval = 1_000us, min_hold = 2_000us -> below the floor.
    song = Song(
        name="below-min-hold-repeat",
        notes=(
            Note(time_ms=Millis(0), key=NoteKey("Key0")),
            Note(time_ms=Millis(1), key=NoteKey("Key0")),
        ),
    )
    policy = FrameTimingPolicy.from_timing_policy(
        TimingPolicy.from_dict({"hold_us": 2_000, "min_hold_us": 2_000}),
    )
    degraded = build_key_actions(song, policy=policy)
    assert degraded.impossible_same_key_repeats == 1

    strict_policy = FrameTimingPolicy.from_timing_policy(
        TimingPolicy.from_dict({"hold_us": 2_000, "min_hold_us": 2_000}),
        same_key_conflict_policy="strict",
    )
    with pytest.raises(ScheduleBuildError):
        build_key_actions(song, policy=strict_policy)

    # A repeat exactly at min_hold is now feasible (no synthetic +500us margin on top).
    feasible_policy = FrameTimingPolicy.from_timing_policy(
        TimingPolicy.from_dict({"hold_us": 1_000, "min_hold_us": 1_000}),
    )
    feasible = build_key_actions(song, policy=feasible_policy)
    assert feasible.impossible_same_key_repeats == 0


def test_scheduler_feasible_repeat_is_runtime_feasible_invariant():
    # Contract: a scheduler-feasible repeat (interval >= min_hold, with comfortable headroom) must
    # NEVER be dropped by the runtime, and completion-to-completion observed hold stays >= min_hold.
    min_hold = 6_945  # local_precise @144fps
    for extra in (500, 700, 1_000, 2_000):
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
            ups = [c for c in backend.calls if c.kind == "up"]
            assert len(downs) == 2, (
                f"dropped a scheduler-feasible repeat: interval=min_hold+{extra}, "
                f"send_duration={send_duration_us}"
            )
            assert ups[0].completed_us - downs[0].completed_us >= min_hold
            assert ups[1].completed_us - downs[1].completed_us >= min_hold


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
        assert summary["observed_hold_below_frame_count"] == 0, f"fps={fps}"


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
    # Verifies via the persisted telemetry snapshot rather than the live coordinator, since the
    # engine now releases its coordinator and runtime_schedule once play() returns so the UI can
    # drop RSS after F9 / song end without waiting for natural ref-count collection.
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


def test_telemetry_evidence_boundaries_keep_sender_clean_separate_from_game_acceptance():
    logger = TelemetryLogger("evidence-clean", enabled=True)
    logger.record(
        event_index=0,
        kind="down",
        scheduled_us=0,
        actual_us=10,
        lateness_us=10,
        send_duration_us=5,
        scan_codes=(21,),
        sent_scan_codes=(21,),
        reason="onset",
    )
    logger.record(
        event_index=1,
        kind="up",
        scheduled_us=20_000,
        actual_us=20_010,
        lateness_us=10,
        send_duration_us=5,
        scan_codes=(21,),
        sent_scan_codes=(21,),
        reason="release",
    )

    summary = logger.get_summary()

    assert summary is not None
    assert summary["intended_down_count"] == 1
    assert summary["sent_down_count"] == 1
    assert summary["before_send_missing_down_count"] == 0
    assert summary["sender_clean"] is True
    assert summary["game_acceptance_unknown"] is True
    assert summary["after_send_missing_count"] is None
    assert summary["evidence_boundaries"]["game_observed"] == {
        "available": False,
        "game_acceptance_unknown": True,
        "heard_onset_count": None,
        "after_send_missing_count": None,
        "note": (
            "Telemetry stops at the SendInput side. Attach audio/onset evidence "
            "before making game-acceptance claims."
        ),
    }


def test_telemetry_evidence_boundaries_flag_before_send_missing_downs():
    logger = TelemetryLogger("evidence-dirty", enabled=True)
    logger.record(
        event_index=0,
        kind="down",
        scheduled_us=0,
        actual_us=10,
        lateness_us=10,
        send_duration_us=5,
        scan_codes=(21, 22),
        sent_scan_codes=(21,),
        skipped_scan_codes=(22,),
        reason="partial-send",
    )
    logger.record(
        event_index=1,
        kind="down",
        scheduled_us=20_000,
        actual_us=20_010,
        lateness_us=10,
        send_duration_us=0,
        scan_codes=(23,),
        sent_scan_codes=(),
        runtime_outcome="dropped_expired",
        reason="expired",
    )

    summary = logger.get_summary()

    assert summary is not None
    assert summary["intended_down_count"] == 3
    assert summary["sent_down_count"] == 1
    assert summary["runtime_backend_dropped_down_count"] == 1
    assert summary["expired_dropped_down_count"] == 1
    assert summary["before_send_missing_down_count"] == 2
    assert summary["sender_clean"] is False
    assert summary["evidence_boundaries"]["sendinput_side"]["sender_clean"] is False


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


def test_input_path_health_flags_sustained_slow_send_duration() -> None:
    clock = FakeClock()
    renderer = CaptureRenderer()
    backend = TimedBackend(clock, send_duration_us=400)
    actions = tuple(
        action(index * 20_000, "down", 100 + index)
        for index in range(70)
    )
    engine = PlaybackEngine(
        song=Song(name="slow-input-path", notes=()),
        actions=actions,
        backend=backend,
        renderer=renderer,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        input_path_warn_us=300,
    )

    assert engine.play() == PLAYBACK_FINISHED

    assert engine.input_path_degraded is True
    assert any(renderer.input_path_flags)
    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["input_path_degraded"] is True
    assert summary["input_path_warn_us"] == 300


def test_health_estimator_equivalence_with_random_sequences() -> None:
    import random
    from collections import deque

    random.seed(42)
    warn_us = 300
    maxlen = 64

    def old_logic(window: deque[int]) -> bool:
        if not window:
            return True
        sorted_window = sorted(window)
        p95_idx = round(0.95 * (len(sorted_window) - 1))
        p95_us = sorted_window[p95_idx]
        return p95_us <= warn_us

    window: deque[int] = deque(maxlen=maxlen)
    over_warn_count = 0

    for _ in range(10000):
        val = random.randint(100, 500)
        evicted = None
        if len(window) == window.maxlen:
            evicted = window[0]

        window.append(val)

        if evicted is not None and evicted > warn_us:
            over_warn_count -= 1
        if val > warn_us:
            over_warn_count += 1

        L = len(window)
        new_result = over_warn_count <= L - 1 - round(0.95 * (L - 1))
        old_result = old_logic(window)

        assert new_result == old_result, (
            f"Equivalence failed for window={list(window)}, "
            f"over_warn_count={over_warn_count}, new={new_result}, old={old_result}"
        )


def test_telemetry_lazy_dict_materialization_and_compatibility() -> None:
    from sky_music.orchestration.telemetry import TelemetryLogger, TelemetryRecord
    logger = TelemetryLogger("test-lazy", enabled=True)
    logger.record(
        event_index=0,
        kind="down",
        scheduled_us=1000,
        actual_us=1005,
        lateness_us=5,
        send_duration_us=10,
        scan_codes=(21, 22),
        reason="onset",
    )
    
    # Assert logger.records contains a TelemetryRecord
    assert len(logger.records) == 1
    record = logger.records[0]
    assert isinstance(record, TelemetryRecord)
    
    # Assert dictionary emulation works
    assert record["song"] == "test-lazy"
    assert record["event_index"] == 0
    assert record["kind"] == "down"
    assert record["scheduled_us"] == 1000
    assert record["actual_us"] == 1005
    assert record["lateness_us"] == 5
    assert record["send_duration_us"] == 10
    assert record["scan_codes"] == "21;22"
    assert record.get("reason") == "onset"
    assert record.get("nonexistent", "default") == "default"
    assert "song" in record
    assert len(record) == 24

    # Assert keys/items/values are correct
    assert "song" in record
    assert ("event_index", 0) in record.items()
    assert "down" in record.values()


def test_send_scan_code_batch_cache_and_retry() -> None:
    from unittest.mock import patch

    import sky_music.platform.win32.inputs as win32_inputs

    # 1. Test cache reuse
    win32_inputs._ARRAY_CACHE.clear()
    chord = (21, 22)

    with patch.object(win32_inputs.user32, "SendInput", return_value=2) as mock_send:
        assert win32_inputs.send_scan_code_batch(chord, key_up=False) == 2
        assert mock_send.call_count == 1

        # Second call should hit the cache (cache size should be 1)
        assert len(win32_inputs._ARRAY_CACHE) == 1
        win32_inputs.send_scan_code_batch(chord, key_up=False)
        assert mock_send.call_count == 2

    # 2. release/safety path (send_scan_code_batch always complete_remainder=True)
    with patch.object(win32_inputs.user32, "SendInput", return_value=1) as mock_send, \
         patch("sky_music.platform.win32.inputs.send_input_batch") as mock_send_batch:
        assert win32_inputs.send_scan_code_batch(chord, key_up=True) == 2
        assert mock_send.call_count == 1
        assert mock_send_batch.call_count == 1
        called_inputs = mock_send_batch.call_args[0][0]
        assert len(called_inputs) == 1
        assert called_inputs[0].ki.wScan == 22

    # 3. Musical note-on path: partial → exactly one same-frame retry, then drop remainder
    win32_inputs.reset_send_diagnostics()
    with patch.object(win32_inputs.user32, "SendInput", side_effect=[1, 0]) as mock_send, \
         patch("sky_music.platform.win32.inputs.send_input_batch") as mock_send_batch:
        assert win32_inputs.send_scan_code_batch_trusted(chord, key_up=False) == 1
        assert mock_send.call_count == 2
        assert mock_send_batch.call_count == 0
        diag = win32_inputs.get_send_diagnostics()
        assert diag["keys_dropped"] == 1
        assert diag["keys_retried"] == 0


def test_playback_state_epoch_based_continuity() -> None:
    from sky_music.orchestration.engine import PlaybackState
    
    clock = FakeClock()
    clock.time_us = 1000
    state = PlaybackState(start_perf=1000)
    
    # Steady running
    clock.time_us = 2000
    assert state.get_elapsed_us(clock) == 1000

    # Pause
    state.enter_pause("manual", clock.time_us)
    clock.time_us = 3000
    assert state.get_elapsed_us(clock) == 1000

    # Resume after 1000 us of pause (single-interval owner accumulates once)
    closed = state.exit_pause("manual", clock.time_us)
    assert closed is not None
    duration_us, attribution = closed
    assert duration_us == 1000
    assert attribution == "manual"

    assert state.pause_time_us == 1000
    assert state.epoch_us == 2000

    # Resume running
    clock.time_us = 4000
    assert state.get_elapsed_us(clock) == 2000


class FakeFocusGuard:
    def __init__(self, clock: FakeClock, focuses: tuple[tuple[int, bool], ...]) -> None:
        self.clock = clock
        self.focuses = list(focuses)
        self.active = True

    def is_active(self) -> bool:
        while self.focuses and self.clock.time_us >= self.focuses[0][0]:
            _, active = self.focuses.pop(0)
            self.active = active
        print(f"FakeFocusGuard is_active at {self.clock.time_us} returning {self.active}")
        return self.active

    def focus(self) -> bool:
        self.active = True
        return True


def test_focus_loss_suspends_and_regain_releases():
    clock = FakeClock()
    sleeper = FakeSleeper(clock)
    backend = TimedBackend(clock, send_duration_us=0)
    # Start focused, lose focus at 15_000, regain at 50_000
    focus_guard = FakeFocusGuard(clock, ((15_000, False), (50_000, True)))
    
    actions = [
        KeyAction(ActionKind.DOWN, (ScanCode(1),), Microseconds(0)),
        KeyAction(ActionKind.DOWN, (ScanCode(2),), Microseconds(10_000)),
        KeyAction(ActionKind.DOWN, (ScanCode(3),), Microseconds(20_000)),
        KeyAction(ActionKind.UP, (ScanCode(1), ScanCode(2)), Microseconds(30_000)),
        KeyAction(ActionKind.DOWN, (ScanCode(4),), Microseconds(60_000)),
        KeyAction(ActionKind.UP, (ScanCode(3), ScanCode(4)), Microseconds(70_000)),
    ]
    
    engine = PlaybackEngine(
        song=Song(name="test", notes=()),
        actions=tuple(actions),
        backend=backend,
        telemetry_enabled=False,
        require_focus=True,
        focus_guard=focus_guard,
        clock=clock,
        sleeper=sleeper,
        sleep_policy=SleepPolicy(spin_threshold_us=-1, poll_s=0.001),
        min_hold_us=5_000,
        focus_restore_grace_us=1000,
        late_pulse_drop_threshold_us=10_000,
        use_dispatch_thread=False,
    )
    engine.play()
    
    downs = [c for c in backend.calls if c.kind == "down"]
    assert [c.scan_codes for c in downs] == [(1,), (2,), (3,), (4,)]
    # Phase 1 dual-release contract (SendInput lifecycle plan §1.3):
    #   1. KEYUP on focus LOSS   (at 15_000 — clears OS keyboard state immediately)
    #   2. KEYUP on focus REGAIN  (at 55_000 — clears game-side half-holds while Sky is foreground)
    #   3. KEYUP on finally      (at 110_000 — idempotent teardown; generations already cancelled)
    # Pre-Phase-1 this asserted only [55_000, 110_000] because focus-loss did cancel_all without
    # a release — the very L1/L2 gap the plan closed. Manual pause / panic share helper 1 + 3.
    assert backend.release_all_calls == [15_000, 55_000, 110_000]
