from __future__ import annotations

from dataclasses import dataclass

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
    )
    
    # Run engine play
    res = engine.play()
    assert res == PLAYBACK_FINISHED
    
    # Estimator counts should be 6 for down (bucket 1) and 6 for up
    assert engine.estimator._count_down[1] == 6
    assert engine.estimator._count_up == 6
    
    # Seed value was 800, 6th value updated: 0.2 * 800 + 0.8 * 800 = 800
    assert engine.estimator.get_lead_us(ActionKind.DOWN) == 800
    assert engine.estimator.get_lead_us(ActionKind.UP) == 800
    
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


def test_estimator_linear_extrapolation_for_unseen_bucket() -> None:
    est = SendLatencyEstimator(alpha=0.2, max_lead_us=5_000)
    # Seed buckets 1 and 4 so a linear model (a + b*N) is defined across >=2 sizes.
    for _ in range(5):
        est.update(ActionKind.DOWN, 200, n_keys=1)
    for _ in range(5):
        est.update(ActionKind.DOWN, 800, n_keys=4)
    # Fit through (1,200) and (4,800): slope 200, intercept 0. Unseen sizes are EXTRAPOLATED,
    # not borrowed from a smaller seeded bucket (which would under-lead the chord).
    assert est.get_lead_us(ActionKind.DOWN, 2) == 400
    assert est.get_lead_us(ActionKind.DOWN, 3) == 600
    # Seeded buckets still return their own EMA exactly.
    assert est.get_lead_us(ActionKind.DOWN, 1) == 200
    assert est.get_lead_us(ActionKind.DOWN, 4) == 800


def test_estimator_extrapolates_beyond_default_max_poly() -> None:
    # A song with an 8-key chord sets max_poly=8 on the estimator
    est = SendLatencyEstimator(alpha=0.2, max_lead_us=5_000, max_poly=8)
    # Seed buckets 1 and 4 -> slope 200, intercept 0
    for _ in range(5):
        est.update(ActionKind.DOWN, 200, n_keys=1)
    for _ in range(5):
        est.update(ActionKind.DOWN, 800, n_keys=4)
    # Chord 8 phím nhận lead ngoại suy đúng theo linear model thay vì bucket-6
    assert est.get_lead_us(ActionKind.DOWN, 8) == 1600


def test_estimator_warm_start_uses_first_sample() -> None:
    est = SendLatencyEstimator(alpha=0.2, max_lead_us=5_000)
    # Seed buckets 1 and 2 so the linear model (slope 200, intercept 0) is available.
    for _ in range(5):
        est.update(ActionKind.DOWN, 200, n_keys=1)
    for _ in range(5):
        est.update(ActionKind.DOWN, 400, n_keys=2)
    # First-ever poly-4 chord, with a real send (1000) above the linear prediction (800):
    # the bucket warm-starts from linear then folds in the sample -> 0.2*1000 + 0.8*800 = 840.
    # (Pure linear fallback would ignore the real sample and return 800.)
    est.update(ActionKind.DOWN, 1000, n_keys=4)
    assert est.get_lead_us(ActionKind.DOWN, 4) == 840


def test_linear_model_forgets_old_regime_and_tracks_drift() -> None:
    # Best-practice RLS with exponential forgetting: the linear backbone must track a shift in
    # per-machine send latency, not stay pinned to a lifetime average. Small window for determinism.
    est = SendLatencyEstimator(alpha=0.2, max_lead_us=10_000, lin_forget=0.9)

    # Old regime: send ≈ 200·N (line through (1,200),(2,400)).
    for _ in range(50):
        est.update(ActionKind.DOWN, 200, n_keys=1)
        est.update(ActionKind.DOWN, 400, n_keys=2)
    # Sanity: before drift the unseen bucket extrapolates the old line.
    assert abs(est.get_lead_us(ActionKind.DOWN, 3) - 600) < 30

    # New regime: send ≈ 400·N (line through (1,400),(2,800)). With a ~10-sample window, the old
    # regime decays to negligible weight, so the fit should track the NEW line (predict(3) ≈ 1200).
    for _ in range(60):
        est.update(ActionKind.DOWN, 400, n_keys=1)
        est.update(ActionKind.DOWN, 800, n_keys=2)

    predicted_3 = est.get_lead_us(ActionKind.DOWN, 3)
    assert predicted_3 > 1000, predicted_3  # tracked the drift; a lifetime average would lag near 900


def test_lin_forget_one_reproduces_lifetime_fit() -> None:
    # lin_forget=1.0 disables decay → identical to the old lifetime-sum behaviour.
    est = SendLatencyEstimator(alpha=0.2, max_lead_us=10_000, lin_forget=1.0)
    for _ in range(5):
        est.update(ActionKind.DOWN, 200, n_keys=1)
    for _ in range(5):
        est.update(ActionKind.DOWN, 800, n_keys=4)
    # Fit through (1,200),(4,800): slope 200, intercept 0 → predict(3) = 600.
    assert est.get_lead_us(ActionKind.DOWN, 3) == 600


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
    popped = coord.pop_due_authored(9_900, lead_for_batch=lead_fn)
    assert len(popped) == 1 and len(popped[0].intents) == 1

    # Chord (4 keys) -> lead 400 -> poppable from t=19_600 (4x earlier relative to its schedule).
    assert coord.next_authored_us(lead_for_batch=lead_fn) == 19_600
    assert coord.pop_due_authored(19_599, lead_for_batch=lead_fn) == ()
    popped2 = coord.pop_due_authored(19_600, lead_for_batch=lead_fn)
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


def test_down_lead_for_batch_onset_bias_is_onset_only() -> None:
    actions = (
        KeyAction(kind=ActionKind.DOWN, scan_codes=(ScanCode(1),), at_us=Microseconds(0), reason="d"),
        KeyAction(kind=ActionKind.UP, scan_codes=(ScanCode(1),), at_us=Microseconds(500), reason="u"),
    )
    engine = _bias_engine(actions, dispatch_lead_us=1_000, onset_bias_us=500)
    loop = engine._compat_dispatch_loop()
    down_batch = engine.runtime_schedule.batches[0]
    up_batch = engine.runtime_schedule.batches[1]
    assert down_batch.kind == "down" and up_batch.kind == "up"
    # Onset gets fixed lead + onset bias; release gets fixed lead only (no onset bias).
    assert loop._down_lead_for_batch(down_batch) == 1_500
    assert loop._down_lead_for_batch(up_batch) == 1_000


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
    engine = _bias_engine(actions, enable_adaptive_lead=True, dispatch_lead_us=0, onset_bias_us=0)
    for _ in range(5):
        engine.estimator.update(ActionKind.DOWN, 200, n_keys=1)
    for _ in range(5):
        engine.estimator.update(ActionKind.DOWN, 800, n_keys=4)
    loop = engine._compat_dispatch_loop()
    single = engine.runtime_schedule.batches[0]
    chord = engine.runtime_schedule.batches[1]
    assert loop._down_lead_for_batch(single) == 200
    assert loop._down_lead_for_batch(chord) == 800


# --- Per-machine estimator persistence (warm-start across sessions) ---


def test_estimator_export_import_round_trip() -> None:
    src = SendLatencyEstimator()
    for _ in range(6):
        src.update(ActionKind.DOWN, 280, n_keys=1)
        src.update(ActionKind.DOWN, 420, n_keys=2)
        src.update(ActionKind.DOWN, 560, n_keys=3)
        src.update(ActionKind.UP, 200)

    dst = SendLatencyEstimator()
    dst.import_state(src.export_state())

    for n in (1, 2, 3):
        assert dst.get_lead_us(ActionKind.DOWN, n) == src.get_lead_us(ActionKind.DOWN, n)
    assert dst.get_lead_us(ActionKind.UP) == src.get_lead_us(ActionKind.UP)
    # Warm from the very first event: no cold-start zero.
    assert dst.get_lead_us(ActionKind.DOWN, 3) > 0


def test_imported_bucket_is_warm_from_first_event() -> None:
    est = SendLatencyEstimator()
    assert est.get_lead_us(ActionKind.DOWN, 3) == 0  # cold

    est.import_state({"version": 1, "ema_down": {"3": 600.0}})
    # Seeded value used immediately, with no real samples sent.
    assert est.get_lead_us(ActionKind.DOWN, 3) == 600


def test_import_ignores_unversioned_or_corrupt_state() -> None:
    est = SendLatencyEstimator()
    est.import_state({})  # missing version
    est.import_state({"version": 2, "ema_down": {"3": 600.0}})  # wrong version
    est.import_state("not a dict")  # type: ignore[arg-type]
    assert est.get_lead_us(ActionKind.DOWN, 3) == 0


def test_import_rejects_out_of_range_values() -> None:
    est = SendLatencyEstimator()
    est.import_state({"version": 1, "ema_down": {"3": 1e12, "2": -5}})
    assert est.get_lead_us(ActionKind.DOWN, 3) == 0
    assert est.get_lead_us(ActionKind.DOWN, 2) == 0


def test_lead_cache_file_round_trip(tmp_path) -> None:
    from sky_music.orchestration.engine import load_lead_cache, save_lead_cache

    path = tmp_path / "cache" / "lead.json"
    assert load_lead_cache(path) is None  # missing file -> None, no crash

    src = SendLatencyEstimator()
    for _ in range(6):
        src.update(ActionKind.DOWN, 300, n_keys=1)
    save_lead_cache(path, src.export_state())

    loaded = load_lead_cache(path)
    assert loaded is not None
    dst = SendLatencyEstimator()
    dst.import_state(loaded)
    assert dst.get_lead_us(ActionKind.DOWN, 1) == src.get_lead_us(ActionKind.DOWN, 1)


def test_load_lead_cache_handles_corrupt_file(tmp_path) -> None:
    from sky_music.orchestration.engine import load_lead_cache

    path = tmp_path / "lead.json"
    path.write_text("{not valid json", encoding="utf-8")
    assert load_lead_cache(path) is None


def test_lead_cache_disabled_for_dry_run_backend(tmp_path) -> None:
    from sky_music.infrastructure.backend import DryRunBackend

    actions = (
        KeyAction(kind=ActionKind.DOWN, scan_codes=(ScanCode(1),), at_us=Microseconds(0), reason="d"),
    )
    path = tmp_path / "lead.json"

    # Real-ish backend (TimedBackend) with adaptive lead + a path -> cache active.
    real = _bias_engine(actions, enable_adaptive_lead=True, lead_cache_path=path)
    assert real._lead_cache_enabled is True

    # DryRunBackend must never read/write the per-machine cache (its sends are not representative).
    dry = PlaybackEngine(
        song=Song(name="poly", notes=()),
        actions=actions,
        backend=DryRunBackend(),
        require_focus=False,
        use_dispatch_thread=False,
        enable_adaptive_lead=True,
        lead_cache_path=path,
    )
    assert dry._lead_cache_enabled is False
