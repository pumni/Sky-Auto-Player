from unittest.mock import Mock

from sky_music.domain.scheduler_types import (
    ActionKind,
    KeyAction,
    Microseconds,
    ScanCode,
)
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.core.coordinator import RuntimeDispatchCoordinator
from sky_music.orchestration.core.loop import (
    DispatchHealthMonitor,
    DispatchLoop,
)
from sky_music.orchestration.runtime_dispatch import compile_runtime_intents


def test_snapshot_honesty():
    actions = (
        KeyAction(kind=ActionKind.DOWN, scan_codes=(ScanCode(1),), at_us=Microseconds(100), reason="first_down"),
        KeyAction(kind=ActionKind.DOWN, scan_codes=(ScanCode(2),), at_us=Microseconds(150), reason="second_down"),
    )
    schedule = compile_runtime_intents(actions)
    coordinator = RuntimeDispatchCoordinator(schedule, min_hold_us=0)

    estimator = Mock()
    estimator_leads = [50, 60]
    def get_lead_us(*args, **kwargs):
        if estimator_leads:
            return estimator_leads.pop(0)
        return 10
    estimator.get_lead_us.side_effect = get_lead_us
    
    # We want to mock estimator.update so we can check if it changes the lead between
    # the two dispatches. Actually get_lead_us will just return 10 for the second one
    # if it gets called, but pop_due_authored should have called it with 50 and 10...
    # Wait, pop_due_authored calls _down_lead_for_batch which calls get_lead_us.
    # So for batch 1 it will return 50. For batch 2 it returns 10.
    # BUT wait! If estimator.update is not called between them, it's just two calls to get_lead_us!
    # Ah, the bug was that pop_due_authored evaluates ALL due batches.
    # Then it dispatches them one by one. After dispatching batch 1, estimator.update is called.
    # If _down_lead_for_batch was called AGAIN inside the loop (as it was before Phase 7),
    # it would return the NEW lead!
    # By snapshotting in pop_due_authored, the lead passed to _dispatch_down_batch is the one computed during pop!
    
    real_clock = Mock()
    real_clock.now_us.return_value = 100
    
    backend = Mock()
    backend_result = Mock()
    backend_result.send_completed_us = 100
    backend_result.sent = (1,)
    backend_result.skipped_duplicates = ()
    backend.key_down.return_value = backend_result
    
    health_monitor = DispatchHealthMonitor(
        backend=Mock(), clock=real_clock, focus_guard=Mock(), require_focus=False
    )
    
    loop = DispatchLoop(
        coordinator=coordinator,
        clock=real_clock,
        sleeper=Mock(), wait_strategy=Mock(), backend=backend, telemetry=Mock(),
        sleep_policy=SleepPolicy(), health_monitor=health_monitor,
        min_hold_us=5000, spin_threshold_us=700,
    )
    # Patch the loop's estimator
    loop.estimator = estimator
    
    from sky_music.orchestration.core.state import PlaybackState
    state = PlaybackState(start_perf=0)
    
    # We will collect the ExecutionResults to see applied_lead_us
    results = []
    
    loop._drain_due(100, state, lead_up=0, observe=lambda res: results.append(res))
    
    assert len(results) == 2
    assert results[0].applied_lead_us == 50
    assert results[1].applied_lead_us == 60
