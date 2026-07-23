from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from sky_music.domain import Song
from sky_music.domain.domain import ScanCode
from sky_music.domain.scheduler_types import ActionKind, KeyAction, Microseconds
from sky_music.infrastructure.backend import (
    BackendHealth,
    InputSendResult,
    ReleaseAllOutcome,
)
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.engine import (
    PLAYBACK_FINISHED,
    PlaybackEngine,
    SendLatencyEstimator,
)
from sky_music.orchestration.runtime_dispatch import (
    RuntimeDispatchCoordinator,
    compile_runtime_intents,
)


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


def test_send_latency_estimator_ema() -> None:
    # Test that alpha=0.2 EMA and capping works
    estimator = SendLatencyEstimator(alpha=0.2, max_lead_us=2000)
    
    # First 4 sends should yield 0
    for _ in range(4):
        estimator.update(ActionKind.DOWN, 1000)
        assert estimator.get_lead_us(ActionKind.DOWN) == 0
    
    # 5th send seeds the EMA
    estimator.update(ActionKind.DOWN, 1000)
    assert estimator.get_lead_us(ActionKind.DOWN) == 1000
    
    # 6th send updates EMA: 0.2 * 1500 + 0.8 * 1000 = 1100
    estimator.update(ActionKind.DOWN, 1500)
    assert estimator.get_lead_us(ActionKind.DOWN) == 1100
    
    # Max cap check: update with high value
    for _ in range(20):
        estimator.update(ActionKind.DOWN, 5000)
    assert estimator.get_lead_us(ActionKind.DOWN) == 2000  # capped

    # Up estimator is independent
    assert estimator.get_lead_us(ActionKind.UP) == 0


def test_send_latency_estimator_residual_prologue_bias() -> None:
    """Systematic positive completion error is folded into down lead (not up)."""
    estimator = SendLatencyEstimator(alpha=0.2, max_lead_us=2000)
    for _ in range(5):
        estimator.update(ActionKind.DOWN, 800)
        estimator.update(ActionKind.UP, 400)
    assert estimator.get_lead_us(ActionKind.DOWN) == 800
    assert estimator.get_lead_us(ActionKind.UP) == 400

    # Cold residual: no bias until 5 samples
    for _ in range(4):
        estimator.update_completion_error(ActionKind.DOWN, 100)
    assert estimator.get_lead_us(ActionKind.DOWN) == 800

    estimator.update_completion_error(ActionKind.DOWN, 100)
    assert estimator.get_lead_us(ActionKind.DOWN) == 900  # 800 send + 100 residual
    # Ups ignore residual prologue bias
    assert estimator.get_lead_us(ActionKind.UP) == 400

    # Early residual must not shrink lead
    for _ in range(20):
        estimator.update_completion_error(ActionKind.DOWN, -200)
    assert estimator.get_lead_us(ActionKind.DOWN) == 800


def test_no_early_conflict_guard() -> None:
    # Test that down action batch is not popped early if there is a conflict (active or pending key release)
    actions = (
        KeyAction(kind=ActionKind.DOWN, scan_codes=(ScanCode(1),), at_us=Microseconds(0), reason="first_down"),
        KeyAction(kind=ActionKind.UP, scan_codes=(ScanCode(1),), at_us=Microseconds(100), reason="first_up"),
        KeyAction(kind=ActionKind.DOWN, scan_codes=(ScanCode(1),), at_us=Microseconds(150), reason="second_down"),
        KeyAction(kind=ActionKind.UP, scan_codes=(ScanCode(1),), at_us=Microseconds(250), reason="second_up"),
    )
    schedule = compile_runtime_intents(actions)
    coordinator = RuntimeDispatchCoordinator(schedule, min_hold_us=0)
    
    # At now_us = 0, first down is due
    due = tuple(b for b, _ in coordinator.pop_due_authored(now_us=0, dispatch_lead_us=0))
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
    due_early = tuple(b for b, _ in coordinator.pop_due_authored(now_us=20, dispatch_lead_us=130))
    assert len(due_early) == 1
    assert due_early[0].kind == "up"
    assert due_early[0].scheduled_us == 100
    
    # Now, request release of the popped up batch
    coordinator.request_releases(due_early[0].intents)
    
    # Now scan code 1 is pending release (in coordinator.pending_by_generation).
    # At now_us = 100, with dispatch_lead_us = 50, the second down action is at 150 (due soon).
    # It still should not pop early because the release has not completed.
    due_early2 = tuple(b for b, _ in coordinator.pop_due_authored(now_us=100, dispatch_lead_us=50))
    assert len(due_early2) == 0
    
    # Now complete release at now_us = 105
    pending = coordinator.pop_due_pending(now_us=105, lead_up=0)
    assert len(pending) == 1
    coordinator.complete_releases(pending, sent_scan_codes=(1,), skipped_scan_codes=())
    
    # Now that the key is released, it is safe to pop the next down early.
    # At now_us = 110, with dispatch_lead_us = 45, second down is at 150 <= 110 + 45 = 155.
    due_early3 = tuple(b for b, _ in coordinator.pop_due_authored(now_us=110, dispatch_lead_us=45))
    assert len(due_early3) == 1
    assert due_early3[0].kind == "down"


def test_adaptive_lead_integration() -> None:
    # Test integration with PlaybackEngine and adaptive lead
    # Create actions that will trigger 7 down dispatches and 7 up dispatches
    actions = []
    for i in range(1, 8):
        actions.append(KeyAction(kind=ActionKind.DOWN, scan_codes=(ScanCode(i),), at_us=Microseconds(i * 1000), reason=f"d{i}"))
        actions.append(KeyAction(kind=ActionKind.UP, scan_codes=(ScanCode(i),), at_us=Microseconds(i * 1000 + 500), reason=f"u{i}"))
    
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
        # Asserts on engine.telemetry.records after play(); production hygiene clears
        # records inside save(), so opt in to retention for the assertion window only.
        retain_telemetry_records_after_save=True,
    )
    
    # Run engine play
    res = engine.play()
    assert res == PLAYBACK_FINISHED
    
    # Estimator counts should be 7 for down (bucket 1) and 7 for up
    assert engine.estimator._count_down[1] == 7
    assert engine.estimator._count_up == 7
    
    # Seed value was 800, 7th value updated: 0.2 * 800 + 0.8 * 800 = 800
    assert engine.estimator.get_lead_us(ActionKind.DOWN) == 800
    assert engine.estimator.get_lead_us(ActionKind.UP) == 800
    
    # Telemetry records should exist and have non-zero applied_lead_us for later records
    records = engine.telemetry.records
    assert len(records) > 0
    
    down_records = [r for r in records if r.kind == "down"]
    assert len(down_records) == 7
    
    # First 6 downs pop early due to falling behind (d5 and d6 pop together when count was 4).
    # d7 is scheduled far enough ahead that it pops in a separate batch AFTER count reached 5+.
    for r in down_records[:6]:
        assert r.applied_lead_us == 0
        
    # 7th down: lead_down retrieved when count was >= 5
    assert down_records[6].applied_lead_us == 800


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
        KeyAction(kind=ActionKind.DOWN, scan_codes=(ScanCode(1),), at_us=Microseconds(10_000), reason="d1"),
        KeyAction(kind=ActionKind.UP, scan_codes=(ScanCode(1),), at_us=Microseconds(10_000 + min_hold_us), reason="u1"),
        KeyAction(kind=ActionKind.DOWN, scan_codes=(ScanCode(1),), at_us=Microseconds(95_000), reason="d2"),
        KeyAction(kind=ActionKind.UP, scan_codes=(ScanCode(1),), at_us=Microseconds(95_000 + min_hold_us), reason="u2"),
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
        engine.estimator.update(ActionKind.DOWN, 5_000)
        engine.estimator.update(ActionKind.UP, 5_000)
    assert engine.estimator.get_lead_us(ActionKind.DOWN) == 2_000
    assert engine.estimator.get_lead_us(ActionKind.UP) == 2_000

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
        actions.append(KeyAction(kind=ActionKind.DOWN, scan_codes=(ScanCode(i),), at_us=Microseconds(i * 20_000), reason=f"d{i}"))
        actions.append(KeyAction(kind=ActionKind.UP, scan_codes=(ScanCode(i),), at_us=Microseconds(i * 20_000 + 5_000), reason=f"u{i}"))

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
        engine.estimator.update(ActionKind.DOWN, send_duration_us)
        engine.estimator.update(ActionKind.UP, send_duration_us)

    assert engine.play() == PLAYBACK_FINISHED

    downs = [c for c in backend.calls if c.kind == "down"]
    ups = [c for c in backend.calls if c.kind == "up"]
    assert [c.completed_us for c in downs] == [i * 20_000 for i in range(1, 5)]
    assert [c.completed_us for c in ups] == [i * 20_000 + 5_000 for i in range(1, 5)]


class PolyphonyScaledBackend(TimedBackend):
    """SendInput duration grows with chord size (per_key_us × n_keys).

    Models the real cost: a bigger chord takes proportionally longer to inject, so without a
    polyphony-aware lead a 4-key chord's onset lands ~4× later than a single note.
    """

    def __init__(self, clock: FakeClock, per_key_us: int) -> None:
        super().__init__(clock, send_duration_us=0)
        self.per_key_us = per_key_us

    def key_down(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        self.send_duration_us = self.per_key_us * len(
            [sc for sc in scan_codes if sc not in self.active]
        )
        return super().key_down(scan_codes)


def test_chord_completion_lands_on_schedule_with_polyphony_scaled_send() -> None:
    """Accuracy guarantee for chords: with a polyphony-scaled SendInput cost and a warm
    per-bucket estimator, a multi-key chord's completion lands on its scheduled onset — NOT
    (n_keys × per_key) late as it would with a scalar/no lead. This is the end-to-end property
    that makes the game receive a chord's notes together instead of skewed. Locks the composite
    of estimator buckets + coordinator per-batch lead + completion targeting against regression."""
    per_key_us = 150
    # Alternate single notes and 4-key chords, generously spaced so every note has room to lead.
    actions: list[KeyAction] = []
    t = 60_000  # start past t=0 so even the first note can be led (t=0 physically cannot)
    for i in range(6):
        scs = (ScanCode(1),) if i % 2 == 0 else (ScanCode(2), ScanCode(3), ScanCode(4), ScanCode(5))
        actions.append(KeyAction(kind=ActionKind.DOWN, scan_codes=scs, at_us=Microseconds(t), reason="d"))
        actions.append(KeyAction(kind=ActionKind.UP, scan_codes=scs, at_us=Microseconds(t + 30_000), reason="u"))
        t += 60_000

    clock = FakeClock()
    backend = PolyphonyScaledBackend(clock, per_key_us=per_key_us)
    engine = _floor_engine(tuple(actions), backend, clock, min_hold_us=0, enable_adaptive_lead=True)
    # Warm the per-polyphony buckets to their steady-state send cost.
    for _ in range(6):
        engine.estimator.update(ActionKind.DOWN, per_key_us * 1, n_keys=1)
        engine.estimator.update(ActionKind.DOWN, per_key_us * 4, n_keys=4)
        engine.estimator.update(ActionKind.UP, per_key_us)

    assert engine.play() == PLAYBACK_FINISHED

    downs = [c for c in backend.calls if c.kind == "down"]
    scheduled = [int(a.at_us) for a in actions if a.kind == "down"]
    # Every onset (completion) lands exactly on schedule regardless of chord size — including the
    # 4-key chords, which a scalar lead would leave 4×per_key late.
    assert [c.completed_us for c in downs] == scheduled
    # And the chords were genuinely 4-key (not silently split/dropped).
    assert [len(c.scan_codes) for c in downs] == [1, 4, 1, 4, 1, 4]


# ---------------------------------------------------------------------------
# Polyphony-aware lead (chord fix) — estimator buckets, coordinator per-batch
# lead, and DispatchLoop._down_lead_for_batch (onset-bias is onset-only).
# ---------------------------------------------------------------------------


def test_estimator_polyphony_buckets() -> None:
    est = SendLatencyEstimator(alpha=0.2, max_lead_us=2_000)
    for _ in range(5):
        est.update(ActionKind.DOWN, 200, n_keys=1)
    for _ in range(5):
        est.update(ActionKind.DOWN, 800, n_keys=4)
    assert est.get_lead_us(ActionKind.DOWN, 1) == 200
    assert est.get_lead_us(ActionKind.DOWN, 4) == 800
    # The whole point of the fix: a 4-key chord is led more than a single note.
    assert est.get_lead_us(ActionKind.DOWN, 4) > est.get_lead_us(ActionKind.DOWN, 1)





def test_estimator_nearest_bucket_when_linear_undefined() -> None:
    est = SendLatencyEstimator(alpha=0.2, max_lead_us=5_000)
    # Only one polyphony seen -> slope undefined -> linear model unavailable.
    for _ in range(5):
        est.update(ActionKind.DOWN, 300, n_keys=2)
    # Unseen bucket 4 falls back to nearest seeded <= 4 (bucket 2 = 300).
    assert est.get_lead_us(ActionKind.DOWN, 4) == 300


def test_estimator_total_fallback_when_no_lower_bucket() -> None:
    est = SendLatencyEstimator(alpha=0.2, max_lead_us=2_000)
    # Only high-polyphony history exists.
    for _ in range(5):
        est.update(ActionKind.DOWN, 800, n_keys=4)
    # A single-note query: no bucket <= 1 seeded -> total fallback (800).
    assert est.get_lead_us(ActionKind.DOWN, 1) == 800
    assert est.get_lead_us(ActionKind.DOWN, 4) == 800


def test_estimator_bucket_cold_then_clamp() -> None:
    est = SendLatencyEstimator(alpha=0.2, max_lead_us=2_000)
    # Cold: fewer than seed samples in the bucket -> lead 0.
    for _ in range(4):
        est.update(ActionKind.DOWN, 5_000, n_keys=3)
    assert est.get_lead_us(ActionKind.DOWN, 3) == 0
    # Seed then push high -> clamp at max_lead_us.
    est.update(ActionKind.DOWN, 5_000, n_keys=3)
    for _ in range(20):
        est.update(ActionKind.DOWN, 9_000, n_keys=3)
    assert est.get_lead_us(ActionKind.DOWN, 3) == 2_000


def test_coordinator_per_batch_lead_scales_with_polyphony() -> None:
    actions = (
        KeyAction(kind=ActionKind.DOWN, scan_codes=(ScanCode(1),), at_us=Microseconds(10_000), reason="single"),
        KeyAction(
            kind=ActionKind.DOWN,
            scan_codes=(ScanCode(2), ScanCode(3), ScanCode(4), ScanCode(5)),
            at_us=Microseconds(20_000),
            reason="chord",
        ),
    )
    coord = RuntimeDispatchCoordinator(compile_runtime_intents(actions), min_hold_us=0)

    def lead_fn(batch: object) -> int:
        return 100 * len(batch.intents) if batch.kind == "down" else 0  # type: ignore[attr-defined]

    # Single note (1 key) -> lead 100 -> poppable from t=9_900.
    assert coord.next_authored_us(lead_for_batch=lead_fn) == 9_900
    assert coord.pop_due_authored(9_899, lead_for_batch=lead_fn) == ()
    popped = tuple(b for b, _ in coord.pop_due_authored(9_900, lead_for_batch=lead_fn))
    assert len(popped) == 1 and len(popped[0].intents) == 1

    # Chord (4 keys) -> lead 400 -> poppable from t=19_600 (4x earlier relative to its schedule).
    assert coord.next_authored_us(lead_for_batch=lead_fn) == 19_600
    assert coord.pop_due_authored(19_599, lead_for_batch=lead_fn) == ()
    popped2 = tuple(b for b, _ in coord.pop_due_authored(19_600, lead_for_batch=lead_fn))
    assert len(popped2) == 1 and len(popped2[0].intents) == 4


def _bias_engine(actions: tuple, **kwargs: object) -> PlaybackEngine:
    return PlaybackEngine(
        song=Song(name="poly", notes=()),
        actions=actions,
        backend=TimedBackend(FakeClock(), send_duration_us=0),
        require_focus=False,
        use_dispatch_thread=False,
        **kwargs,  # type: ignore[arg-type]
    )


def test_down_lead_for_batch_scales_with_polyphony() -> None:
    actions = (
        KeyAction(kind=ActionKind.DOWN, scan_codes=(ScanCode(1),), at_us=Microseconds(0), reason="single"),
        KeyAction(
            kind=ActionKind.DOWN,
            scan_codes=(ScanCode(2), ScanCode(3), ScanCode(4), ScanCode(5)),
            at_us=Microseconds(1_000),
            reason="chord",
        ),
    )
    engine = _bias_engine(actions, enable_adaptive_lead=True, dispatch_lead_us=0)
    for _ in range(5):
        engine.estimator.update(ActionKind.DOWN, 200, n_keys=1)
    for _ in range(5):
        engine.estimator.update(ActionKind.DOWN, 800, n_keys=4)
    loop = engine._compat_dispatch_loop()
    assert engine.runtime_schedule is not None
    single = engine.runtime_schedule.batches[0]
    chord = engine.runtime_schedule.batches[1]
    assert loop._down_lead_for_batch(single) == 200
    assert loop._down_lead_for_batch(chord) == 800


# --- Per-machine estimator persistence (warm-start across sessions) ---


# --- Phase 4 SendInput-lifecycle plan §4.2 cold-start regression tests ---





def test_first_three_key_chord_lead_positive_after_linear_seed_from_singles() -> None:
    """Phase 4 §4.2 cold-start hardening regression: an ``N=3`` chord authored BEFORE its
    bucket has any samples gets a non-zero lead via warm-start from a singles bucket.

    Pre-Phase-4 the cold-start gap (T1) let the first notes of a session fire with
    ``lead=0`` because every per-polyphony bucket seeded to zero for its first
    ``_SEED_SAMPLES=5`` updates. Phase 4 keeps the seed-zero behaviour for buckets
    WITH samples, but warm-starts unseen buckets from the nearest seeded bucket below
    (the linear backbone or the nearest lower-sized fallback) — so an N=3 chord's
    cold-session lead is still positive after singles have been seeded.
    """
    seed_n = SendLatencyEstimator._SEED_SAMPLES
    est = SendLatencyEstimator(alpha=0.2, max_lead_us=5_000)
    # Seed ONLY singles (N=1) so the linear backbone is degenerate (slope undefined).
    # The estimator falls back to the nearest seeded bucket ≤ N=3 — i.e. the singles
    # bucket's EMA — still > 0. This is the documented cold-start safeguard the plan row
    # "First-chord polyphony" sets out.
    for _ in range(seed_n):
        est.update(ActionKind.DOWN, 250, n_keys=1)

    # Cold N=3 chord with no bucket of its own yet.
    lead_cold = est.get_lead_us(ActionKind.DOWN, 3)
    assert lead_cold > 0, (
        "Phase 4 cold-start: an N=3 chord after singles-only seed must lead>0 "
        "(linear warm-start OR nearest-bucket fallback — both close the cold gap)"
    )

    # After the first real N=3 sample the bucket warm-starts — a subsequent N=4 query
    # STILL extrapolates > 0.
    est.update(ActionKind.DOWN, 900, n_keys=3)
    lead_after_sample = est.get_lead_us(ActionKind.DOWN, 4)
    assert lead_after_sample > 0, (
        "Phase 4: N=4 estimation after seeding (1, 3) stays positive"
    )


def test_residual_prologue_bias_is_positive_only_and_capped_at_500us() -> None:
    """Phase 4 §4.2 row "residual bias: keep positive-only cap 500us — document".

    The estimator's residual prologue bias is intentionally positive-only: lead must NEVER
    shrink because an early completion under-shot (that would compound latency). Negative
    residuals would cancel future lead and cause systematic lateness. The cap bounds the
    bias influence so a single slow first event cannot dominate lead.
    """
    seed_n = SendLatencyEstimator._SEED_SAMPLES
    est = SendLatencyEstimator(alpha=0.2, max_lead_us=5_000)
    # Seed a bucket so the residual can take effect.
    for _ in range(seed_n):
        est.update(ActionKind.DOWN, 200, n_keys=1)

    # Force an early completion: lead was 200, completion landed at -1000us early.
    # The residual bias must NOT become -1000 (negative forbidden); it stays at 0.
    est.update_completion_error(ActionKind.DOWN, -1_000)
    assert est.get_lead_us(ActionKind.DOWN, 1) == 200, (
        "negative residuals must not reduce lead (positive-only invariant)"
    )

    # Force a late completion: residual folds in (positive side) but is capped at 500us.
    est.update_completion_error(ActionKind.DOWN, 1_500)
    lead_after_late = est.get_lead_us(ActionKind.DOWN, 1)
    assert lead_after_late >= 200, "residual folds in positive side (>= EMA)"
    assert lead_after_late <= 200 + 500, "residual prologue bias capped at 500us"


# --- Phase D: Cross-session EMA cache ---


def test_lead_export_import_round_trip() -> None:
    """Phase D: export_state then import_state preserves lead values within 1 us."""
    est = SendLatencyEstimator(alpha=0.2, max_lead_us=2_000)
    for _ in range(6):
        est.update(ActionKind.DOWN, 800, n_keys=1)
        est.update(ActionKind.DOWN, 1500, n_keys=4)
        est.update(ActionKind.UP, 400)
    lead_down_1_before = est.get_lead_us(ActionKind.DOWN, 1)
    lead_down_4_before = est.get_lead_us(ActionKind.DOWN, 4)
    lead_up_before = est.get_lead_us(ActionKind.UP)

    state = est.export_state()
    est2 = SendLatencyEstimator(alpha=0.2, max_lead_us=2_000)
    assert est2.import_state(state)

    assert abs(est2.get_lead_us(ActionKind.DOWN, 1) - lead_down_1_before) <= 1
    assert abs(est2.get_lead_us(ActionKind.DOWN, 4) - lead_down_4_before) <= 1
    assert abs(est2.get_lead_us(ActionKind.UP) - lead_up_before) <= 1


def test_lead_import_never_shrinks_song_sized_buckets() -> None:
    """A cache from a lower-polyphony song must not collapse the current song's top buckets.

    The engine sizes max_poly = max(6, max_chord of the CURRENT song) before importing the
    cross-session cache; import must keep that sizing (padding cold buckets), so an 8-key
    chord updates bucket 8 instead of being clamped into a blended bucket 6.
    """
    donor = SendLatencyEstimator(alpha=0.2, max_lead_us=2_000, max_poly=6)
    for _ in range(6):
        donor.update(ActionKind.DOWN, 400, n_keys=1)
        donor.update(ActionKind.DOWN, 900, n_keys=6)
    state = donor.export_state()

    est = SendLatencyEstimator(alpha=0.2, max_lead_us=2_000, max_poly=8)
    assert est.import_state(state)
    assert est.max_poly == 8, "import must not shrink below the song-derived sizing"
    # Cached buckets survive; the cold poly-8 bucket falls back to the nearest seeded one (6).
    assert abs(est.get_lead_us(ActionKind.DOWN, 6) - donor.get_lead_us(ActionKind.DOWN, 6)) <= 1
    assert abs(est.get_lead_us(ActionKind.DOWN, 8) - donor.get_lead_us(ActionKind.DOWN, 6)) <= 1
    # Real 8-key samples seed bucket 8 itself — no clamp into bucket 6.
    for _ in range(6):
        est.update(ActionKind.DOWN, 1_400, n_keys=8)
    assert abs(est.get_lead_us(ActionKind.DOWN, 8) - 1_400) <= 1
    assert abs(est.get_lead_us(ActionKind.DOWN, 6) - donor.get_lead_us(ActionKind.DOWN, 6)) <= 1
    # A larger cached sizing still grows the estimator (existing behavior preserved).
    est_small = SendLatencyEstimator(alpha=0.2, max_lead_us=2_000, max_poly=6)
    assert est_small.import_state(est.export_state())
    assert est_small.max_poly == 8


def test_lead_import_poison_rejected() -> None:
    """Phase D: Corrupt or invalid import data is rejected without changing estimator."""
    est = SendLatencyEstimator(alpha=0.2, max_lead_us=2_000)
    for _ in range(5):
        est.update(ActionKind.DOWN, 500, n_keys=1)

    # Wrong version
    assert not est.import_state({"version": 1, "max_poly": 6, "ema_down": [0.0] * 7, "warm_down": [False] * 7})
    # Negative EMA
    state_bad = est.export_state()
    state_bad["ema_down"] = [-100.0] * (est.max_poly + 1)
    assert not est.import_state(state_bad)
    # Truncated lists
    state_trunc = est.export_state()
    state_trunc["ema_down"] = [0.0] * 2
    assert not est.import_state(state_trunc)
    # Wrong max_poly
    state_wp = est.export_state()
    state_wp["max_poly"] = -1
    assert not est.import_state(state_wp)
    # Non-dict
    assert not est.import_state(["not", "a", "dict"])  # type: ignore[arg-type]
    # Estimator unchanged after poisoning
    assert est.get_lead_us(ActionKind.DOWN, 1) == 500


def test_lead_cache_cross_engine_persistence(tmp_path: Path) -> None:
    """Phase D: First engine play seeds estimator; second engine loads warm lead > 0."""
    from sky_music.infrastructure.timing import SleepPolicy

    actions = tuple(
        KeyAction(kind=ActionKind.DOWN if i % 2 == 0 else ActionKind.UP,
                  scan_codes=(ScanCode(i // 2 + 1),),
                  at_us=Microseconds(i * 20_000), reason=f"a{i}")
        for i in range(12)
    )
    song = Song(name="persist", notes=())
    cache_path = str(tmp_path / ".cache" / "lead_estimator.json")

    clock1 = FakeClock()
    backend1 = TimedBackend(clock1, send_duration_us=800)
    engine1 = PlaybackEngine(
        song=song, actions=actions, backend=backend1,
        telemetry_enabled=False, require_focus=False,
        clock=clock1, sleeper=FakeSleeper(clock1),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        use_dispatch_thread=False, enable_adaptive_lead=True,
        lead_cache_path=cache_path,
    )
    assert engine1.play() == PLAYBACK_FINISHED
    # Cache file should exist after play
    assert Path(cache_path).exists()

    clock2 = FakeClock()
    backend2 = TimedBackend(clock2, send_duration_us=800)
    engine2 = PlaybackEngine(
        song=song, actions=actions, backend=backend2,
        telemetry_enabled=False, require_focus=False,
        clock=clock2, sleeper=FakeSleeper(clock2),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        use_dispatch_thread=False, enable_adaptive_lead=True,
        lead_cache_path=cache_path,
    )
    # Before any update, the loaded estimator should give non-zero lead
    assert engine2.estimator.get_lead_us(ActionKind.DOWN, 1) > 0
    assert engine2.estimator.get_lead_us(ActionKind.UP) > 0
    assert engine2.play() == PLAYBACK_FINISHED


def test_lead_cache_default_path_not_written(tmp_path: Path) -> None:
    """Phase D: No lead_cache_path set does not create cache file."""
    from sky_music.infrastructure.backend import DryRunBackend
    from sky_music.infrastructure.timing import SleepPolicy

    actions = (KeyAction(kind=ActionKind.DOWN, scan_codes=(ScanCode(1),),
                         at_us=Microseconds(0), reason="d"),
               KeyAction(kind=ActionKind.UP, scan_codes=(ScanCode(1),),
                         at_us=Microseconds(10_000), reason="u"))
    song = Song(name="drytest", notes=())
    clock = FakeClock()
    backend = DryRunBackend()
    nonexistent = str(tmp_path / "nonexistent_lead.json")
    engine = PlaybackEngine(
        song=song, actions=actions, backend=backend,
        telemetry_enabled=False, require_focus=False,
        clock=clock, sleeper=FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        use_dispatch_thread=False, enable_adaptive_lead=True,
    )
    assert engine.play() == PLAYBACK_FINISHED
    # No default cache path set — ensure no stray file created
    assert not Path(nonexistent).exists()

