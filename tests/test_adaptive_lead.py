from __future__ import annotations

from dataclasses import dataclass
from sky_music.domain import Song
from sky_music.domain.domain import ScanCode
from sky_music.domain.scheduler_types import KeyAction, Microseconds
from sky_music.infrastructure.backend import BackendHealth, InputSendResult, ReleaseAllOutcome
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.engine import SendLatencyEstimator, PlaybackEngine, PLAYBACK_FINISHED
from sky_music.orchestration.runtime_dispatch import RuntimeDispatchCoordinator, compile_runtime_intents


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


def test_send_latency_estimator_ema() -> None:
    # Test that alpha=0.2 EMA and capping works
    estimator = SendLatencyEstimator(alpha=0.2, max_lead_us=2000)
    
    # First 4 sends should yield 0
    for _ in range(4):
        estimator.update("down", 1000)
        assert estimator.get_lead_us("down") == 0
    
    # 5th send seeds the EMA
    estimator.update("down", 1000)
    assert estimator.get_lead_us("down") == 1000
    
    # 6th send updates EMA: 0.2 * 1500 + 0.8 * 1000 = 1100
    estimator.update("down", 1500)
    assert estimator.get_lead_us("down") == 1100
    
    # Max cap check: update with high value
    for _ in range(20):
        estimator.update("down", 5000)
    assert estimator.get_lead_us("down") == 2000  # capped

    # Up estimator is independent
    assert estimator.get_lead_us("up") == 0


def test_no_early_conflict_guard() -> None:
    # Test that down action batch is not popped early if there is a conflict (active or pending key release)
    actions = (
        KeyAction(kind="down", scan_codes=(ScanCode(1),), at_us=Microseconds(0), reason="first_down"),
        KeyAction(kind="up", scan_codes=(ScanCode(1),), at_us=Microseconds(100), reason="first_up"),
        KeyAction(kind="down", scan_codes=(ScanCode(1),), at_us=Microseconds(150), reason="second_down"),
        KeyAction(kind="up", scan_codes=(ScanCode(1),), at_us=Microseconds(250), reason="second_up"),
    )
    schedule = compile_runtime_intents(actions)
    coordinator = RuntimeDispatchCoordinator(schedule, min_hold_us=0)
    
    # At now_us = 0, first down is due
    due = coordinator.pop_due_authored(now_us=0, dispatch_lead_us=0)
    assert len(due) == 1
    assert due[0].kind == "down"
    
    # Activate first down
    coordinator.activate_sent_downs(due[0].intents, sent_scan_codes=(1,), dispatch_started_us=0, dispatch_completed_us=10)
    
    # Now, scan code 1 is active.
    # At now_us = 20, let's say we have dispatch_lead_us = 130.
    # The second down action is scheduled at 150.
    # Since 150 <= now_us (20) + dispatch_lead_us (130), it would normally pop early.
    # BUT, scan code 1 is active (no release requested yet).
    # So the second down should NOT pop early.
    # The first_up batch is at 100 <= 150, so it will pop early.
    due_early = coordinator.pop_due_authored(now_us=20, dispatch_lead_us=130)
    assert len(due_early) == 1
    assert due_early[0].kind == "up"
    assert due_early[0].scheduled_us == 100
    
    # Now, request release of the popped up batch
    coordinator.request_releases(due_early[0].intents)
    
    # Now scan code 1 is pending release (in coordinator.pending_by_generation).
    # At now_us = 100, with dispatch_lead_us = 50, the second down action is at 150 (due soon).
    # It still should not pop early because the release has not completed.
    due_early2 = coordinator.pop_due_authored(now_us=100, dispatch_lead_us=50)
    assert len(due_early2) == 0
    
    # Now complete release at now_us = 105
    pending = coordinator.pop_due_pending(now_us=105, lead_up=0)
    assert len(pending) == 1
    coordinator.complete_releases(pending, sent_scan_codes=(1,), skipped_scan_codes=())
    
    # Now that the key is released, it is safe to pop the next down early.
    # At now_us = 110, with dispatch_lead_us = 45, second down is at 150 <= 110 + 45 = 155.
    due_early3 = coordinator.pop_due_authored(now_us=110, dispatch_lead_us=45)
    assert len(due_early3) == 1
    assert due_early3[0].kind == "down"


def test_adaptive_lead_integration() -> None:
    # Test integration with PlaybackEngine and adaptive lead
    # Create actions that will trigger 6 down dispatches and 6 up dispatches
    actions = []
    for i in range(1, 7):
        actions.append(KeyAction(kind="down", scan_codes=(ScanCode(i),), at_us=Microseconds(i * 1000), reason=f"d{i}"))
        actions.append(KeyAction(kind="up", scan_codes=(ScanCode(i),), at_us=Microseconds(i * 1000 + 500), reason=f"u{i}"))
    
    actions_tuple = tuple(actions)
    song = Song(name="test_adaptive", notes=())
    
    clock = FakeClock()
    # Mock send_duration_us = 800 us inside TimedBackend
    backend = TimedBackend(clock, send_duration_us=800)
    sleeper = FakeSleeper(clock)
    
    engine = PlaybackEngine(
        song=song,
        actions=actions_tuple,
        backend=backend,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=sleeper,
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        use_dispatch_thread=False,
        enable_adaptive_lead=True,
    )
    
    # Run engine play
    res = engine.play()
    assert res == PLAYBACK_FINISHED
    
    # Estimator counts should be 6 for down and 6 for up
    assert engine.estimator._count_down == 6
    assert engine.estimator._count_up == 6
    
    # Seed value was 800, 6th value updated: 0.2 * 800 + 0.8 * 800 = 800
    assert engine.estimator.get_lead_us("down") == 800
    assert engine.estimator.get_lead_us("up") == 800
    
    # Telemetry records should exist and have non-zero applied_lead_us for later records
    records = engine.telemetry.records
    assert len(records) > 0
    
    # For the first 5 downs, applied_lead_us should be 0 because estimator count was < 5
    # The 6th down should have applied_lead_us = 800 because estimator count is 5 (seeded) when we query it
    down_records = [r for r in records if r.kind == "down"]
    assert len(down_records) == 6
    
    # First 5 downs: lead_down retrieved when count was 0, 1, 2, 3, 4
    for r in down_records[:5]:
        assert r.applied_lead_us == 0
        
    # 6th down: lead_down retrieved when count was 5
    assert down_records[5].applied_lead_us == 800


# ---------------------------------------------------------------------------
# G5 prime-directive tests: the 1-frame floor and completion-targeting under lead
# ---------------------------------------------------------------------------


class AsymmetricTimedBackend(TimedBackend):
    """TimedBackend with different SendInput durations for downs and ups."""

    def __init__(self, clock: FakeClock, down_duration_us: int, up_duration_us: int) -> None:
        super().__init__(clock, send_duration_us=0)
        self.down_duration_us = down_duration_us
        self.up_duration_us = up_duration_us

    def key_down(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        self.send_duration_us = self.down_duration_us
        return super().key_down(scan_codes)

    def key_up(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        self.send_duration_us = self.up_duration_us
        return super().key_up(scan_codes)


def _observed_holds(calls: list[TimedCall]) -> list[int]:
    """Completion-to-completion hold per generation — the game-observed hold model."""
    holds: list[int] = []
    last_down_completed: dict[int, int] = {}
    for call in calls:
        for scan_code in call.scan_codes:
            if call.kind == "down":
                last_down_completed[scan_code] = call.completed_us
            elif call.kind == "up" and scan_code in last_down_completed:
                holds.append(call.completed_us - last_down_completed.pop(scan_code))
    return holds


def _floor_engine(actions, backend, clock, *, min_hold_us, **kwargs):
    return PlaybackEngine(
        song=Song(name="floor_lead", notes=()),
        actions=actions,
        backend=backend,
        telemetry_enabled=False,
        require_focus=False,
        clock=clock,
        sleeper=FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        use_dispatch_thread=False,
        min_hold_us=min_hold_us,
        **kwargs,
    )


def _tight_same_key_actions(min_hold_us: int):
    return (
        KeyAction(kind="down", scan_codes=(ScanCode(1),), at_us=Microseconds(10_000), reason="d1"),
        KeyAction(kind="up", scan_codes=(ScanCode(1),), at_us=Microseconds(10_000 + min_hold_us), reason="u1"),
        KeyAction(kind="down", scan_codes=(ScanCode(1),), at_us=Microseconds(95_000), reason="d2"),
        KeyAction(kind="up", scan_codes=(ScanCode(1),), at_us=Microseconds(95_000 + min_hold_us), reason="u2"),
    )


def test_observed_hold_never_below_floor_under_manual_lead() -> None:
    """A symmetric manual lead may pull the up earlier, but never below the completion-anchored
    floor: observed (completion-to-completion) hold stays >= min_hold under asymmetric send
    latency. This is the 1-frame prime directive under dispatch lead."""
    min_hold_us = 17_000
    clock = FakeClock()
    backend = AsymmetricTimedBackend(clock, down_duration_us=1_500, up_duration_us=100)
    engine = _floor_engine(
        _tight_same_key_actions(min_hold_us),
        backend,
        clock,
        min_hold_us=min_hold_us,
        dispatch_lead_us=2_000,
    )

    assert engine.play() == PLAYBACK_FINISHED

    holds = _observed_holds(backend.calls)
    assert len(holds) == 2
    for hold in holds:
        assert hold >= min_hold_us
    # Nothing was dropped: every authored down and up reached the backend.
    assert sum(1 for c in backend.calls if c.kind == "down") == 2
    assert sum(1 for c in backend.calls if c.kind == "up") == 2


def test_observed_hold_never_below_floor_under_adaptive_lead() -> None:
    """Same floor invariant with the adaptive estimator pre-warmed to the maximum lead clamp."""
    min_hold_us = 17_000
    clock = FakeClock()
    backend = AsymmetricTimedBackend(clock, down_duration_us=1_500, up_duration_us=100)
    engine = _floor_engine(
        _tight_same_key_actions(min_hold_us),
        backend,
        clock,
        min_hold_us=min_hold_us,
        enable_adaptive_lead=True,
    )
    # Pre-warm the estimator to the worst case: the 2 ms clamp on both kinds.
    for _ in range(5):
        engine.estimator.update("down", 5_000)
        engine.estimator.update("up", 5_000)
    assert engine.estimator.get_lead_us("down") == 2_000
    assert engine.estimator.get_lead_us("up") == 2_000

    assert engine.play() == PLAYBACK_FINISHED

    holds = _observed_holds(backend.calls)
    assert len(holds) == 2
    for hold in holds:
        assert hold >= min_hold_us


def test_dispatch_completion_lands_on_schedule_with_warm_estimator() -> None:
    """Onset = dispatch completion: with a warm estimator and a constant fake send duration,
    every SendInput completion lands exactly on the scheduled timestamp."""
    send_duration_us = 800
    actions: list[KeyAction] = []
    for i in range(1, 5):
        actions.append(KeyAction(kind="down", scan_codes=(ScanCode(i),), at_us=Microseconds(i * 20_000), reason=f"d{i}"))
        actions.append(KeyAction(kind="up", scan_codes=(ScanCode(i),), at_us=Microseconds(i * 20_000 + 5_000), reason=f"u{i}"))

    clock = FakeClock()
    backend = TimedBackend(clock, send_duration_us=send_duration_us)
    engine = _floor_engine(
        tuple(actions),
        backend,
        clock,
        min_hold_us=0,
        enable_adaptive_lead=True,
    )
    for _ in range(5):
        engine.estimator.update("down", send_duration_us)
        engine.estimator.update("up", send_duration_us)

    assert engine.play() == PLAYBACK_FINISHED

    downs = [c for c in backend.calls if c.kind == "down"]
    ups = [c for c in backend.calls if c.kind == "up"]
    assert [c.completed_us for c in downs] == [i * 20_000 for i in range(1, 5)]
    assert [c.completed_us for c in ups] == [i * 20_000 + 5_000 for i in range(1, 5)]
