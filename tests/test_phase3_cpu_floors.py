"""Phase 3 CPU floor reductions: A5 symmetric reprobe + uncontaminated samples.

See docs/2026-07_core-dispatch-refactor-and-isolation-plan.md §6.

These tests pin the three sub-fixes of §6.1:
1. The "already past deadline" branch inside `_wait_until_runtime_deadline` does NOT
   feed `_record_overshoot` — it is contaminated by the previous drain, not timer-wake
   error.
2. The spin branch (after `spin_until_us`) DOES feed `_record_overshoot` — that is the
   only true timer-wake overshoot sample.
3. `_recompute_spin_threshold_from_overshoot` is applied symmetrically (a re-probe may
   LOWER the threshold after calm samples follow a spike), and every applied value is
   appended to telemetry `runtime_options["reprobe_applied_thresholds"]`.

The thin comment fix in `inputs.py` §6.2 has no behavioral test — it is comment-only.
"""

from __future__ import annotations

from typing import Any

from sky_music.domain.scheduler_types import (
    KeyAction,
    Microseconds,
    ScanCode,
)
from sky_music.infrastructure.backend import (
    BackendHealth,
    InputSendResult,
    ReleaseAllOutcome,
)
from sky_music.infrastructure.timing import Clock, SleepPolicy
from sky_music.orchestration.dispatch_loop import (
    DispatchHealthMonitor,
    DispatchLoop,
    PlaybackState,
)
from sky_music.orchestration.playback_supervisor import (
    DirectFocusSignal,
    DirectProgressSink,
)
from sky_music.orchestration.runtime_dispatch import (
    RuntimeDispatchCoordinator,
    compile_runtime_intents,
)
from sky_music.orchestration.telemetry import TelemetryLogger


class FakeClock:
    def __init__(self, time_us: int = 0) -> None:
        self.time_us = time_us

    def now_us(self) -> int:
        return self.time_us


class FakeSleeper:
    """Sleeper whose wake advances the clock by requested microseconds exactly."""

    def __init__(self, clock: FakeClock) -> None:
        self.clock = clock

    def sleep(self, seconds: float) -> None:
        self.clock.time_us += max(1, int(seconds * 1_000_000))


class NullWaitStrategy:
    """Wait strategy that teleports the clock to the target (no busy-spin stall)."""

    def spin_until_us(self, target_system_us: int, clock: Clock) -> None:
        now = clock.now_us()
        if isinstance(now, int):
            if now < target_system_us:
                # FakeClock exposes `time_us` as a plain int via now_us(); mutate it.
                clock.time_us = target_system_us  # type: ignore[attr-defined]
        else:
            if now() < target_system_us:  # type: ignore[operator]
                clock.time_us = target_system_us  # type: ignore[attr-defined]

    def wait_until_us(
        self,
        target_system_us: int,
        clock: Clock,
        sleeper: FakeSleeper,
        spin_threshold_us: int,
        policy: SleepPolicy,
        command_event: int | None = None,
    ) -> bool:
        self.spin_until_us(target_system_us, clock)
        return False


class StaticFocusGuard:
    def __init__(self, active: bool = True) -> None:
        self._active = active

    def is_active(self) -> bool:
        return self._active


class NullControls:
    def poll(self) -> str | None:
        return None


class TimedBackend:
    """Minimal backend satisfying InputBackend-shape for direct DispatchLoop tests."""

    def __init__(self, clock: FakeClock) -> None:
        self.clock = clock
        self.active: set[int] = set()

    def key_down(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        sent = tuple(sc for sc in scan_codes if sc not in self.active)
        self.active.update(sent)
        return InputSendResult(sent=sent, skipped_duplicates=(), success=True)

    def key_up(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        self.active.difference_update(scan_codes)
        return InputSendResult(sent=scan_codes, skipped_duplicates=(), success=True)

    def release_all(self) -> ReleaseAllOutcome:
        self.active.clear()
        return ReleaseAllOutcome(
            attempted=(),
            released_successfully=True,
            stuck_keys=(),
            verification_inconclusive=False,
        )

    def get_health(self) -> BackendHealth:
        return BackendHealth(0, 0, 0, None)

    def get_send_diagnostics(self) -> dict[str, int]:
        return {}

    def set_clock(self, clock: Any) -> None:
        return


def _action(at_us: int, kind: str, *scan_codes: int) -> KeyAction:
    return KeyAction(
        kind=kind,  # type: ignore[arg-type]
        scan_codes=tuple(ScanCode(sc) for sc in scan_codes),
        at_us=Microseconds(at_us),
        reason="phase3",
    )


def _build_loop(
    *,
    clock: FakeClock,
    backend: TimedBackend,
    actions: tuple[KeyAction, ...],
    enable_reprobe: bool = True,
    spin_threshold_us: int = 1_500,
    min_hold_us: int = 5_000,
) -> DispatchLoop:
    schedule = compile_runtime_intents(actions)
    coordinator = RuntimeDispatchCoordinator(schedule, min_hold_us=min_hold_us)
    health = DispatchHealthMonitor(
        backend=backend,
        clock=clock,
        focus_guard=StaticFocusGuard(active=True),  # type: ignore[arg-type]
        require_focus=False,
    )
    telemetry = TelemetryLogger("phase3", enabled=True)
    return DispatchLoop(
        coordinator=coordinator,
        clock=clock,
        sleeper=FakeSleeper(clock),
        wait_strategy=NullWaitStrategy(),  # type: ignore[arg-type]
        backend=backend,
        telemetry=telemetry,
        sleep_policy=SleepPolicy(spin_threshold_us=spin_threshold_us, poll_s=0.001),
        health_monitor=health,
        min_hold_us=min_hold_us,
        spin_threshold_us=spin_threshold_us,
        enable_reprobe=enable_reprobe,
    )


# --- §6.1.1 contaminated sample on already-late branch -----------------------


def test_already_late_branch_does_not_record_overshoot() -> None:
    """`_wait_until_runtime_deadline` already-past-deadline branch must NOT sample.

    Before §6.1: `_record_overshoot(elapsed, target)` ran on this branch where
    `elapsed >= target`. Any positive overshoot here reflects the previous drain's
    duration, not a timer-wake error — a contaminated sample that drags the
    recomputed threshold toward the 3 000 us cap.

    Discriminating test: prime the clock past the target by an exaggerated margin so
    a contaminated sample would have to be added; assert the deque stays empty.
    """
    clock = FakeClock(time_us=10_000)
    backend = TimedBackend(clock)
    loop = _build_loop(
        clock=clock, backend=backend, actions=(_action(50_000, "down", 21),)
    )

    state = PlaybackState(start_perf=0)
    target_elapsed_us = 5_000  # elapsed (10_000) is already past this deadline
    assert state.get_elapsed_us(clock) > target_elapsed_us

    loop._wait_until_runtime_deadline(
        target_elapsed_us=target_elapsed_us,
        state=state,
        last_runtime_poll_us=0,
        last_render_time_us=0,
        first_action_executed=False,
        total_time_us=1_000_000,
        command_source=NullControls(),  # type: ignore[arg-type]
        focus_signal=DirectFocusSignal(lambda: True),
        progress_sink=DirectProgressSink(renderer=None, song_name="t"),  # type: ignore[arg-type]
        command_event=None,
    )

    assert len(loop._overshoot_samples) == 0, (
        "Already-late branch must not feed _record_overshoot (drain-duration sample, "
        "not timer-wake error)"
    )


# --- §6.1.1 spin branch DOES record overshoot ---------------------------------


def test_spin_branch_records_overshoot_sample() -> None:
    """Branch where `wait_strategy.spin_until_us` ran is the only legitimate sample.

    Spin measures true timer-wake error, so `_record_overshoot(after, target)` must
    append. We verify a single overshoot sample lands when `remaining_us <= threshold`
    and the FakeClock is teleported past target by the spin strategy.
    """
    clock = FakeClock(time_us=0)
    backend = TimedBackend(clock)

    # Custom wait strategy: spin_until_us overshoots by a fixed +250 us so the
    # post-spin clock is strictly past the target.
    class OvershootSpinStrategy(NullWaitStrategy):
        def spin_until_us(self, target_system_us: int, clock: Clock) -> None:
            # Teleport slightly past to mimic hardware overshoot.
            clock.time_us = target_system_us + 250  # type: ignore[attr-defined]

    schedule = compile_runtime_intents((_action(50_000, "down", 21),))
    coordinator = RuntimeDispatchCoordinator(schedule, min_hold_us=5_000)
    health = DispatchHealthMonitor(
        backend=backend,
        clock=clock,
        focus_guard=StaticFocusGuard(active=True),  # type: ignore[arg-type]
        require_focus=False,
    )
    telemetry = TelemetryLogger("phase3-spin", enabled=True)
    loop = DispatchLoop(
        coordinator=coordinator,
        clock=clock,
        sleeper=FakeSleeper(clock),
        wait_strategy=OvershootSpinStrategy(),  # type: ignore[arg-type]
        backend=backend,
        telemetry=telemetry,
        sleep_policy=SleepPolicy(spin_threshold_us=1_500, poll_s=0.001),
        health_monitor=health,
        min_hold_us=5_000,
        spin_threshold_us=1_500,
        enable_reprobe=True,
    )

    state = PlaybackState(start_perf=0)

    # Wait target is above the spin threshold: 2_000 us remaining <= 1_500 us threshold
    # is impossible at 2_000; pick a remaining of 1_000 us so threshold fires.
    # FakeClock at t=0, target_elapsed=1_000; remaining=1_000 <= 1_500 -> spin path.
    target_elapsed_us = 1_000
    assert target_elapsed_us <= loop.spin_threshold_us

    loop._wait_until_runtime_deadline(
        target_elapsed_us=target_elapsed_us,
        state=state,
        last_runtime_poll_us=0,
        last_render_time_us=0,
        first_action_executed=False,
        total_time_us=1_000_000,
        command_source=NullControls(),  # type: ignore[arg-type]
        focus_signal=DirectFocusSignal(lambda: True),
        progress_sink=DirectProgressSink(renderer=None, song_name="t"),  # type: ignore[arg-type]
        command_event=None,
    )

    assert len(loop._overshoot_samples) == 1
    assert loop._overshoot_samples[0] == 250


# --- §6.1.2 symmetric apply + telemetry trail --------------------------------


def test_recompute_recovers_downward_after_spike() -> None:
    """A spike must NOT lock the threshold at the spike's value across calm samples.

    Section §6.1.2 mandates the threshold recomputes BOTH directions. With a 200-
    sample rolling window, feeding a burst of large samples then a burst of calm
    samples must bring the threshold back down. This test drives the recomputation
    directly so it does not depend on interval gating inside the loop.
    """
    clock = FakeClock(time_us=0)
    backend = TimedBackend(clock)
    loop = _build_loop(
        clock=clock, backend=backend, actions=(_action(50_000, "down", 21),)
    )

    # Force enough samples for `_recompute_spin_threshold_from_overshoot` to compute;
    # then spike then calm. Deque maxlen is 200 so a full window rotates.
    for _ in range(100):
        loop._record_overshoot(5_000, 0)  # overshoot = 5_000 us
    peak = loop._recompute_spin_threshold_from_overshoot()
    assert peak >= 2_000, f"spike floor failed: {peak}"

    # Calm samples now dominate after the rolling window rotates the spike out.
    for _ in range(200):
        loop._record_overshoot(50, 0)  # overshoot = 50 us
    calm = loop._recompute_spin_threshold_from_overshoot()
    assert calm <= 800, f"calm floor failed: {calm}"
    assert calm < peak, "threshold must recover downwards after a spike"


def test_threshold_applied_symmetric_down_to_telemetry() -> None:
    """When reprobe recomputes a SMALLER threshold it LOWERS — symmetric apply.

    Pre-§6.1.2 the run-loop apply block gated with
    `if new_threshold > self.spin_threshold_us` — it ratcheted up only. After the
    fix the apply is unconditional and every applied value is appended to
    telemetry `runtime_options["reprobe_applied_thresholds"]`.

    Drives `DispatchLoop._maybe_apply_reprobe_threshold(now_us)` directly so the
    fail-first criterion is the ratchet guard's behavior, not the whole run() path.
    """
    clock = FakeClock(time_us=20_000_000)  # advance past the 5 s interval on first call
    backend = TimedBackend(clock)
    loop = _build_loop(
        clock=clock,
        backend=backend,
        actions=(_action(50_000, "down", 21),),
        enable_reprobe=True,
        spin_threshold_us=1_500,
    )

    # Seed a calm-overshoot sample window so recompute returns the 700 us floor.
    for _ in range(20):
        loop._record_overshoot(50, 0)
    recomputed = loop._recompute_spin_threshold_from_overshoot()
    assert recomputed <= 700, f"calm recomputation failed: {recomputed}"
    assert recomputed < loop.spin_threshold_us

    # Run the apply seam. Before the fix this returns None (ratchet guard rejected
    # the lower value) AND the threshold stayed at 1 500. After the fix it returns
    # the applied threshold and appends to the telemetry trail.
    applied = loop._maybe_apply_reprobe_threshold(clock.now_us())

    assert applied == recomputed
    assert loop.spin_threshold_us == recomputed
    trail = loop.telemetry.runtime_options.get("reprobe_applied_thresholds")
    assert isinstance(trail, list)
    assert trail[-1] == recomputed


def test_reprobe_skips_without_enough_samples() -> None:
    """Recompute under 10 samples stays no-op (guarded), and no telemetry entry added."""
    clock = FakeClock(time_us=20_000_000)
    backend = TimedBackend(clock)
    loop = _build_loop(
        clock=clock,
        backend=backend,
        actions=(_action(50_000, "down", 21),),
        enable_reprobe=True,
        spin_threshold_us=1_500,
    )
    # Only 3 samples (below the 10-sample floor): recompute returns current threshold.
    for _ in range(3):
        loop._record_overshoot(500, 0)
    applied = loop._maybe_apply_reprobe_threshold(clock.now_us())
    assert applied is None, "sub-threshold-sample reprobe must be a no-op"
    assert loop.spin_threshold_us == 1_500
    trail = loop.telemetry.runtime_options.get("reprobe_applied_thresholds")
    assert trail is None or trail == []


# --- Sanity: golden timeline is shipped separately in test_golden_dispatch_timeline ---
