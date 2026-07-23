"""Phase 1 correctness fixes: A1 pre-down focus gate, A6a clock injection, A6b estimator.

See docs/2026-07_core-dispatch-refactor-and-isolation-plan.md §4.1, §4.3, §4.4.
"""

from __future__ import annotations

from typing import Any

from sky_music.domain import Song
from sky_music.domain.scheduler_types import (
    ActionKind,
    KeyAction,
    Microseconds,
    ScanCode,
)
from sky_music.infrastructure.backend import (
    BackendHealth,
    DryRunBackend,
    InputSendResult,
    ReleaseAllOutcome,
    WinSendInputBackend,
)
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.dispatch_loop import (
    DispatchHealthMonitor,
    DispatchLoop,
    PlaybackState,
)
from sky_music.orchestration.engine import (
    PLAYBACK_FINISHED,
    PlaybackEngine,
    SendLatencyEstimator,
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
    def __init__(self, clock: FakeClock) -> None:
        self.clock = clock

    def sleep(self, seconds: float) -> None:
        self.clock.time_us += max(1, int(seconds * 1_000_000))


class NullWaitStrategy:
    def spin_until_us(self, target_system_us: int, clock: FakeClock) -> None:
        if clock.now_us() < target_system_us:
            clock.time_us = target_system_us

    def wait_until_us(
        self,
        target_system_us: int,
        clock: FakeClock,
        sleeper: FakeSleeper,
        spin_threshold_us: int,
        policy: SleepPolicy,
        command_event: int | None = None,
    ) -> bool:
        if clock.now_us() < target_system_us:
            clock.time_us = target_system_us
        return False


class InactiveFocusSignal:
    def __init__(self) -> None:
        self.is_active_calls = 0

    def is_active(self) -> bool:
        self.is_active_calls += 1
        return False


class ActiveFocusSignal:
    def __init__(self) -> None:
        self.is_active_calls = 0

    def is_active(self) -> bool:
        self.is_active_calls += 1
        return True


class TimedBackend:
    def __init__(self, clock: FakeClock) -> None:
        self.clock = clock
        self.active: set[int] = set()
        self.calls: list[tuple[str, tuple[int, ...], int]] = []

    def key_down(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        sent = tuple(sc for sc in scan_codes if sc not in self.active)
        skipped = tuple(sc for sc in scan_codes if sc in self.active)
        if sent:
            self.calls.append(("down", sent, self.clock.now_us()))
            self.active.update(sent)
        return InputSendResult(sent=sent, skipped_duplicates=skipped, success=True)

    def key_up(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        sent = tuple(sc for sc in scan_codes if sc in self.active)
        skipped = tuple(sc for sc in scan_codes if sc not in self.active)
        if sent:
            self.calls.append(("up", sent, self.clock.now_us()))
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

    def get_health(self) -> BackendHealth:
        return BackendHealth(
            active_count=len(self.active),
            possibly_active_count=0,
            failed_release_count=0,
            last_error=None,
        )

    def get_send_diagnostics(self) -> dict[str, int]:
        return {}

    def set_clock(self, clock: Any) -> None:
        return


def _action(at_us: int, kind: str, *scan_codes: int) -> KeyAction:
    return KeyAction(
        kind=kind,  # type: ignore[arg-type]
        scan_codes=tuple(ScanCode(sc) for sc in scan_codes),
        at_us=Microseconds(at_us),
        reason="phase1",
    )


def _build_loop(
    *,
    clock: FakeClock,
    backend: TimedBackend,
    actions: tuple[KeyAction, ...],
    require_focus: bool = True,
    estimator: SendLatencyEstimator | None = None,
) -> DispatchLoop:
    schedule = compile_runtime_intents(actions)
    coordinator = RuntimeDispatchCoordinator(schedule, min_hold_us=5_000)
    focus = ActiveFocusSignal()
    health = DispatchHealthMonitor(
        backend=backend,
        clock=clock,
        focus_guard=focus,  # type: ignore[arg-type]
        require_focus=require_focus,
    )
    return DispatchLoop(
        coordinator=coordinator,
        clock=clock,
        sleeper=FakeSleeper(clock),
        wait_strategy=NullWaitStrategy(),  # type: ignore[arg-type]
        backend=backend,
        telemetry=TelemetryLogger("phase1", enabled=True),
        sleep_policy=SleepPolicy(spin_threshold_us=-1, poll_s=0.001),
        health_monitor=health,
        min_hold_us=5_000,
        spin_threshold_us=-1,
        estimator=estimator,
    )


# --- A1: pre-down focus gate -------------------------------------------------


def test_pre_down_gate_records_blocked_unfocused() -> None:
    """Discriminating gate test: only the Phase-2 path emits blocked_unfocused.

    The polled focus-pause gate never labels a down with that outcome — it pauses
    the timeline instead. Calling _dispatch_down_batch directly after arming the
    gate isolates the pre-down check.
    """
    clock = FakeClock(25_000)
    backend = TimedBackend(clock)
    actions = (
        _action(0, "down", 21),
        _action(25_000, "down", 22),
        _action(40_000, "up", 21),
    )
    loop = _build_loop(clock=clock, backend=backend, actions=actions)
    # Simulate run() entry wiring (the bug was overwriting this with None).
    inactive = InactiveFocusSignal()
    loop._runtime_focus_signal = inactive

    batch = loop.coordinator.schedule.batches[1]  # second down
    state = PlaybackState(start_perf=0)
    result = loop._dispatch_down_batch(batch, state, lead_down=0, now_us=25_000)

    assert result is None
    assert not any(c[0] == "down" and c[1] == (22,) for c in backend.calls)
    assert inactive.is_active_calls >= 1

    outcomes = [
        r.runtime_outcome
        for r in loop.telemetry.records
        if getattr(r, "runtime_outcome", None) == "blocked_unfocused"
    ]
    assert outcomes, "Phase-2 gate must record runtime_outcome=blocked_unfocused"
    assert any(c.kind == "down" for c in loop.telemetry.records if getattr(c, "runtime_outcome", None) == "blocked_unfocused")


def test_runtime_focus_signal_used_by_pre_down_gate() -> None:
    """Micro-test: with ≥2 downs and require_focus, the gate calls is_active after arming."""
    clock = FakeClock()
    backend = TimedBackend(clock)
    actions = (
        _action(0, "down", 21),
        _action(20_000, "up", 21),
        _action(30_000, "down", 22),
        _action(50_000, "up", 22),
    )
    loop = _build_loop(clock=clock, backend=backend, actions=actions)
    signal = ActiveFocusSignal()
    loop._runtime_focus_signal = signal

    batch = loop.coordinator.schedule.batches[2]  # second down
    state = PlaybackState(start_perf=0)
    clock.time_us = 30_000
    result = loop._dispatch_down_batch(batch, state, lead_down=0, now_us=30_000)
    assert result is not None
    assert signal.is_active_calls >= 1


def test_pre_down_gate_uses_fresh_foreground_probe_when_signal_is_stale() -> None:
    """§2.1: a fresh HWND probe must gate the down even while the signal reads active.

    The supervisor's SharedFocusSignal is sampled only every focus_poll_s (20–50 ms),
    so on alt-tab it can still read *active* for up to a poll cadence — during which
    every down batch would inject into whatever window is now foreground. A cheap
    GetForegroundWindow==sky compare on the dispatch thread (no OpenProcess) closes
    that race so a down never leaks into the window the user just switched to.
    """
    clock = FakeClock(25_000)
    backend = TimedBackend(clock)
    actions = (
        _action(0, "down", 21),
        _action(25_000, "down", 22),
        _action(40_000, "up", 21),
    )
    loop = _build_loop(clock=clock, backend=backend, actions=actions)
    # Signal is (stale) active, but the fresh foreground probe says NOT on Sky.
    stale_active = ActiveFocusSignal()
    loop._runtime_focus_signal = stale_active
    loop.cheap_foreground_probe = lambda: False

    batch = loop.coordinator.schedule.batches[1]  # second down
    state = PlaybackState(start_perf=0)
    result = loop._dispatch_down_batch(batch, state, lead_down=0, now_us=25_000)

    assert result is None, "down must be gated when the fresh probe says not-foreground"
    assert not any(c[0] == "down" and c[1] == (22,) for c in backend.calls), (
        "scan code 22 must never reach the backend when foreground is not Sky"
    )
    assert any(
        getattr(r, "runtime_outcome", None) == "blocked_unfocused"
        for r in loop.telemetry.records
    ), "stale-signal leak must be recorded as blocked_unfocused"


def test_pre_down_gate_proceeds_when_probe_confirms_foreground() -> None:
    """Guard against over-blocking: active signal + probe True must still dispatch."""
    clock = FakeClock()
    backend = TimedBackend(clock)
    actions = (
        _action(0, "down", 21),
        _action(20_000, "up", 21),
        _action(30_000, "down", 22),
        _action(50_000, "up", 22),
    )
    loop = _build_loop(clock=clock, backend=backend, actions=actions)
    loop._runtime_focus_signal = ActiveFocusSignal()
    loop.cheap_foreground_probe = lambda: True

    batch = loop.coordinator.schedule.batches[2]  # second down
    state = PlaybackState(start_perf=0)
    clock.time_us = 30_000
    result = loop._dispatch_down_batch(batch, state, lead_down=0, now_us=30_000)
    assert result is not None
    assert any(c[0] == "down" and c[1] == (22,) for c in backend.calls)


def test_run_wires_runtime_focus_signal_not_none() -> None:
    """run() must keep the FocusSignal assigned (regression for the merge overwrite)."""
    clock = FakeClock()
    backend = TimedBackend(clock)
    actions = (
        _action(0, "down", 21),
        _action(10_000, "up", 21),
    )
    loop = _build_loop(clock=clock, backend=backend, actions=actions)
    signal = ActiveFocusSignal()

    class _NullCmd:
        def poll(self) -> str | None:
            return None

    class _NullProgress:
        def publish(self, **kwargs: object) -> None:
            return

        def update_counters(self, *args: object, **kwargs: object) -> None:
            return

        def finish(self, message: str = "") -> None:
            return

    state = PlaybackState(start_perf=0)
    loop.run(
        state=state,
        command_source=_NullCmd(),  # type: ignore[arg-type]
        focus_signal=signal,  # type: ignore[arg-type]
        progress_sink=_NullProgress(),  # type: ignore[arg-type]
        total_time_us=20_000,
    )
    # After run completes, the last assigned signal must STILL be the exact one we
    # passed — the A1 merge bug clobbered it to None after assignment, so a strict
    # identity check is the real regression guard. A disjunction with
    # ``signal.is_active_calls >= 1`` would silently pass even with the bug back,
    # because the health monitor holds the same signal object via a separate
    # attribute (set at run() entry) and bumps that counter through the A4 path.
    assert loop._runtime_focus_signal is signal
    # And it was actually consulted during dispatch (gate/diagnostics live).
    assert signal.is_active_calls >= 1


# --- A6a: clock injection ---------------------------------------------------


def test_winsendinput_backend_send_completed_us_uses_injected_clock(monkeypatch) -> None:
    """send_completed_us must come from the injected Clock, not bare perf_counter."""
    clock = FakeClock(1_234_567)

    class _FakeInputs:
        def send_scan_code_batch_trusted(self, scan_codes, *, key_up: bool) -> int:
            return len(scan_codes)

        def get_send_diagnostics(self) -> dict[str, int]:
            return {}

    # Avoid starting the real watchdog during unit construction.
    monkeypatch.setattr(
        "sky_music.infrastructure.backend._start_watchdog_once",
        lambda: None,
    )
    backend = WinSendInputBackend()
    backend.inputs_module = _FakeInputs()  # type: ignore[assignment]
    backend.set_clock(clock)

    result = backend.key_down((0x15,))
    assert result.send_completed_us == 1_234_567


# --- A6b: estimator no-op guard ---------------------------------------------


def test_estimator_not_updated_on_empty_sent() -> None:
    """Unit: SendLatencyEstimator itself is fine; loop must skip update when sent empty."""
    est = SendLatencyEstimator(alpha=0.5, max_lead_us=5_000)
    # Seed with a real sample so lead is non-zero after warm-up.
    for _ in range(5):
        est.update(ActionKind.DOWN, 800, n_keys=1)
    lead_before = est.get_lead_us(ActionKind.DOWN, n_keys=1)
    assert lead_before > 0

    clock = FakeClock()
    backend = TimedBackend(clock)
    # First down then duplicate down of same key (second send is empty / skipped).
    actions = (
        _action(0, "down", 21),
        _action(5_000, "down", 21),  # duplicate while still held (no up yet)
        _action(20_000, "up", 21),
    )
    loop = _build_loop(
        clock=clock,
        backend=backend,
        actions=actions,
        require_focus=False,
        estimator=est,
    )
    loop._runtime_focus_signal = ActiveFocusSignal()
    state = PlaybackState(start_perf=0)

    # Dispatch first down for real.
    b0 = loop.coordinator.schedule.batches[0]
    clock.time_us = 0
    r0 = loop._dispatch_down_batch(b0, state, lead_down=0, now_us=0)
    assert r0 is not None
    assert r0.sent_scan_codes == (21,)

    lead_after_first = est.get_lead_us(ActionKind.DOWN, n_keys=1)

    # Second down: same key still active → sent empty (coordinator may drop as conflict,
    # or backend skips). Either way estimator must not be pulled toward 0.
    b1 = loop.coordinator.schedule.batches[1]
    clock.time_us = 5_000
    r1 = loop._dispatch_down_batch(b1, state, lead_down=0, now_us=5_000)
    # Conflict drop returns None without estimator update; or noop send with empty sent.
    if r1 is not None:
        assert r1.sent_scan_codes == ()

    lead_after = est.get_lead_us(ActionKind.DOWN, n_keys=1)
    assert lead_after == lead_after_first


def test_noop_duplicate_down_via_engine_leaves_lead_stable() -> None:
    """Loop-level: duplicate-down schedule does not drag adaptive lead toward 0."""
    clock = FakeClock()

    class _NullControls:
        def poll(self) -> str | None:
            return None

    # Force adaptive lead path: dispatch_lead_us=0 and enable_adaptive_lead.
    # We inject a pre-warmed estimator via monkeypatch after construction.
    actions = (
        _action(10_000, "down", 21),
        _action(15_000, "down", 21),  # same-key while held → conflict or noop
        _action(40_000, "up", 21),
    )
    backend = DryRunBackend()
    engine = PlaybackEngine(
        song=Song(name="noop-est", notes=()),
        actions=actions,
        backend=backend,
        controls=_NullControls(),
        telemetry_enabled=False,
        require_focus=False,
        clock=clock,
        sleeper=FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=10_000,
        use_dispatch_thread=False,
        dispatch_lead_us=0,
        enable_adaptive_lead=True,
    )
    est = engine.estimator
    for _ in range(5):
        est.update(ActionKind.DOWN, 900, n_keys=1)
    lead_before = est.get_lead_us(ActionKind.DOWN, n_keys=1)

    assert engine.play() == PLAYBACK_FINISHED
    lead_after = est.get_lead_us(ActionKind.DOWN, n_keys=1)
    # Lead may refine from the first real send, but must not collapse to 0 from no-ops.
    assert lead_after > 0
    # The no-op path must not have applied a pure-duration-0 EMA pull that halves lead.
    assert lead_after >= lead_before * 0.3
