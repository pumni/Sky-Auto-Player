"""TDD for Patch A2: HUD/ProgressCounters must use visible_lateness_us for onsets.

Regression for the ChatGPT-review finding A1: ``observe_result`` inside
``DispatchLoop.run()`` fed ``exec_result.lateness_us`` (sender call-entry)
into onset counters (``_max_lateness_us``, ``_late_2ms/5ms/10ms``, ``_latencies``
deque) even though ``ExecutionResult`` already separates
``visible_lateness_us`` (completion_us - scheduled_us) as the player-perceived
onset metric. HUD ``SnapshotRenderer`` consumes these under the label
"Onset-only counters (key-down) — these are what the player hears" — but the
value was dispatch-thread call-entry lateness, not player-visible onset.

The fix extracts ``observe_result`` into ``DispatchLoop._observe_exec_result``
so the unit test can drive it with a synthetic ``ExecutionResult`` where
``lateness_us == 0`` (call-entry exactly on deadline) but
``visible_lateness_us == 350`` (SendInput completion landed 350 µs after the
deadline). The onset counters must read 350, not 0.

Release counters (kind='up') continue to use ``lateness_us`` — the release
path's metric is the bounded-retry contract, not game-observed onset
(regression per AGENTS.md priority stack P0: no perf change, no
contract-class change).
"""

from __future__ import annotations

from unittest.mock import Mock

from sky_music.domain.scheduler_types import (
    ActionKind,
    KeyAction,
    Microseconds,
    ScanCode,
)
from sky_music.infrastructure.backend import DryRunBackend
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.core.coordinator import RuntimeDispatchCoordinator
from sky_music.orchestration.core.loop import (
    DispatchHealthMonitor,
    DispatchLoop,
    ExecutionResult,
)
from sky_music.orchestration.core.state import PlaybackState
from sky_music.orchestration.runtime_dispatch import compile_runtime_intents


def _make_exec_result(
    *,
    kind: str,
    lateness_us: int,
    visible_lateness_us: int,
    runtime_outcome: str = "sent",
) -> ExecutionResult:
    return ExecutionResult(
        event_index=0,
        scheduled_us=0,
        actual_us=0,
        lateness_us=lateness_us,
        send_duration_us=0,
        is_late=lateness_us > 0,
        is_critically_late=lateness_us > 10_000,
        kind=kind,
        runtime_outcome=runtime_outcome,
        visible_lateness_us=visible_lateness_us,
    )


def _build_loop_with_counters() -> DispatchLoop:
    """Build a DispatchLoop with the per-run counters that ``run()`` normally
    initialises on first entry. Lets the test drive ``_observe_exec_result``
    directly without running the whole pipeline.
    """
    actions = (
        KeyAction(
            kind=ActionKind.DOWN,
            scan_codes=(ScanCode(1),),
            at_us=Microseconds(0),
            reason="hud-onset-test",
        ),
    )
    schedule = compile_runtime_intents(actions)
    coordinator = RuntimeDispatchCoordinator(schedule, min_hold_us=0)
    backend = DryRunBackend()
    clock = Mock()
    clock.now_us.return_value = 0
    health_monitor = DispatchHealthMonitor(
        backend=backend, clock=clock, focus_guard=Mock(), require_focus=False
    )

    loop = DispatchLoop(
        coordinator=coordinator,
        clock=clock,
        sleeper=Mock(),
        wait_strategy=Mock(),
        backend=backend,
        telemetry=Mock(),
        sleep_policy=SleepPolicy(),
        health_monitor=health_monitor,
        min_hold_us=0,
        spin_threshold_us=700,
    )
    # Initialise the per-run counters the way ``run()`` does on first entry,
    # so ``_get_progress_counters()`` can return a non-None snapshot.
    loop._max_lateness_us = 0
    loop._late_2ms = 0
    loop._late_5ms = 0
    loop._late_10ms = 0
    loop._release_max_us = 0
    loop._release_late_2ms = 0
    import collections
    loop._latencies = collections.deque(maxlen=2000)
    # Stub ``PlaybackState`` because ``_observe_exec_result`` does not touch
    # the state directly; provided for parity with the production runtime.
    _ = PlaybackState(start_perf=0)
    return loop


def test_observe_exec_result_onset_uses_visible_lateness_us() -> None:
    """An onset whose call-entry lateness is 0 but completion lands 350 µs
    late must report ``max_lateness_us == 350`` (the visible onset), not 0.
    """
    loop = _build_loop_with_counters()
    loop._observe_exec_result(
        _make_exec_result(
            kind="down",
            lateness_us=0,                # call-entry exactly on deadline
            visible_lateness_us=350,      # completion landed 350 µs late
        )
    )

    counters = loop._get_progress_counters()
    assert counters is not None, (
        "_get_progress_counters must return a non-None snapshot after init"
    )
    assert counters.max_lateness_us == 350, (
        f"HUD max_lateness_us must use visible_lateness_us for onset, "
        f"got {counters.max_lateness_us}"
    )
    assert counters.recent_latencies_us == (350,), (
        f"the per-onset latency ring must carry visible_lateness_us, "
        f"got {counters.recent_latencies_us}"
    )


def test_observe_exec_result_onset_threshold_buckets_use_visible_lateness_us() -> None:
    """Onset ``_late_2ms``/``_late_5ms``/``_late_10ms`` counters must trip off
    the visible onset. A 2 500 µs completion-only late dispatch with on-time
    call-entry must count toward ``_late_2ms`` (but not 5 ms / 10 ms).
    """
    loop = _build_loop_with_counters()
    loop._observe_exec_result(
        _make_exec_result(
            kind="down",
            lateness_us=0,
            visible_lateness_us=2_500,
        )
    )

    counters = loop._get_progress_counters()
    assert counters is not None
    assert counters.late_2ms == 1, (
        f"expected 1 onset over 2 ms (visible), got {counters.late_2ms}"
    )
    assert counters.late_5ms == 0
    assert counters.late_10ms == 0
    assert counters.max_lateness_us == 2_500


def test_observe_exec_result_release_uses_completion_lateness() -> None:
    """Release counters include syscall/retry time through completion."""
    loop = _build_loop_with_counters()
    loop._observe_exec_result(
        _make_exec_result(
            kind="up",
            lateness_us=4_000,           # call-entry lateness
            visible_lateness_us=2_000,   # completion-lateness metric
        )
    )

    counters = loop._get_progress_counters()
    assert counters is not None
    assert counters.release_max_us == 2_000, (
        f"release counter must use completion lateness, got {counters.release_max_us}"
    )
    assert counters.release_late_2ms == 0
    # Release counter must NOT have polluted the onset counter.
    assert counters.max_lateness_us == 0
    assert counters.recent_latencies_us == (), (
        "release path must not append to the onset latency ring buffer"
    )


def test_observe_exec_result_ignores_deferred_release() -> None:
    """Deferred releases bypass the running totals — same as production
    observe_result's ``runtime_outcome != 'deferred_release'`` guard.
    """
    loop = _build_loop_with_counters()
    loop._observe_exec_result(
        _make_exec_result(
            kind="down",
            lateness_us=5_000,
            visible_lateness_us=5_000,
            runtime_outcome="deferred_release",
        )
    )

    counters = loop._get_progress_counters()
    assert counters is not None
    assert counters.max_lateness_us == 0
    assert counters.late_2ms == 0
    assert counters.late_5ms == 0
    assert counters.recent_latencies_us == ()
