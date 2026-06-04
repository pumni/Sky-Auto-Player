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


def test_release_guard_anchors_hold_to_down_dispatch_completion():
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
    assert summary["confirmed_hold_lower_bound_us"]["min_us"] == 1_000
    assert summary["confirmed_hold_lower_bound_us"]["p50_us"] == 1_000
    assert summary["confirmed_hold_shortfall_count"] == 0
    assert summary["deferred_release_count"] == 1


def test_deferred_release_does_not_delay_unrelated_down():
    backend, _ = play(
        (
            action(0, "down", 21),
            action(1_000, "up", 21),
            action(1_100, "down", 22),
            action(3_000, "up", 22),
        ),
        min_hold_us=1_000,
        send_duration_us=300,
    )

    assert next(
        call.started_us for call in backend.calls if call.kind == "down" and call.scan_codes == (22,)
    ) == 1_100
    assert next(
        call.started_us for call in backend.calls if call.kind == "up" and call.scan_codes == (21,)
    ) >= 1_300


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


def test_strict_runtime_conflict_stops_cleanly_and_releases_active_key():
    clock = FakeClock()
    backend = TimedBackend(clock, send_duration_us=300)
    engine = PlaybackEngine(
        song=Song(name="strict", notes=()),
        actions=(
            action(0, "down", 21),
            action(1_000, "up", 21),
            action(1_000, "down", 21),
            action(2_000, "up", 21),
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
