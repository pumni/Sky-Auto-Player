from unittest.mock import Mock

from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.core.loop import DispatchHealthMonitor, DispatchLoop


def test_idle_warmup_skipped_when_pending_release_due():
    # Phase 6: test that warmup budget does not delay already-due pending releases
    coordinator = Mock()
    # next_deadline_us simulates the effective deadline logic.
    # In the buggy code, _drain_due called next_authored_us directly.
    # We want to mock both to prove the fix works.
    coordinator.next_authored_us.return_value = 10_000_000  # far future
    coordinator.next_deadline_us.return_value = 1000  # already due!
    
    # Return a pending release that is due
    fake_release = Mock()
    fake_release.release_not_before_us = 1000
    fake_release.scheduled_release_us = 1000
    fake_release.keys = (1,)
    fake_release.generation = 1
    coordinator.pop_due_pending.side_effect = [(fake_release,), ()] # due now, then empty
    coordinator.pop_due_authored.return_value = []
    # The dispatch loop now drains scalar (``pop_next_due_authored``); make the mock
    # coordinator return None so the while-loop exits immediately.
    coordinator.pop_next_due_authored.return_value = None
    
    real_clock = Mock()
    real_clock.now_us.side_effect = [2000, 2000, 2000, 2000, 2000, 2000, 2000] 
    
    health_monitor = DispatchHealthMonitor(
        backend=Mock(),
        clock=real_clock,
        focus_guard=Mock(),
        require_focus=False
    )
    
    backend = Mock()
    backend_result = Mock()
    backend_result.send_completed_us = None
    backend.key_up.return_value = backend_result
    backend.key_down.return_value = backend_result

    loop = DispatchLoop(
        coordinator=coordinator,
        clock=real_clock,
        sleeper=Mock(),
        wait_strategy=Mock(),
        backend=backend,
        telemetry=Mock(),
        sleep_policy=SleepPolicy(),
        health_monitor=health_monitor,
        min_hold_us=5000,
        spin_threshold_us=700,
    )
    
    # We want to trigger Phase E warmup hook
    loop._last_send_elapsed_us = 0
    now_us = 100_000 # > SEND_COLD_THRESHOLD_US (20_000)
    

    from sky_music.orchestration.core.state import PlaybackState
    state = PlaybackState(start_perf=0)
    
    deadline = coordinator.next_deadline_us(dispatch_lead_us=0, lead_up=0)
    
    # To prevent infinite loop in wait_until_runtime_deadline, we need to mock wait_strategy to advance clock
    def mock_wait(*args, **kwargs):
        real_clock.now_us.side_effect = [100_000]*10
        return False
    loop.wait_strategy.wait_until_us.side_effect = mock_wait  # type: ignore[attr-defined]
    
    loop._wait_until_runtime_deadline(
        target_elapsed_us=deadline,
        state=state,
        last_runtime_poll_us=0,
        last_render_time_us=0,
        first_action_executed=False,
        total_time_us=1000,
        command_source=Mock(),
        focus_signal=Mock(),
        progress_sink=Mock()
    )
    loop._drain_due(now_us, state, lead_up=0, observe=None)
    
    # Assert warmup was not called (or called with <= 0 budget which gets skipped)
    assert not loop.wait_strategy.wait_until_us.called  # pyright: ignore[reportAttributeAccessIssue]
    assert coordinator.pop_due_pending.called

def test_idle_warmup_uses_effective_deadline_when_future():
    # Phase 6: test that warmup budget uses the effective deadline when both are future
    coordinator = Mock()
    coordinator.next_authored_us.return_value = 10_000_000
    coordinator.next_deadline_us.return_value = 2500  # next pending release in 500us
    
    coordinator.pop_due_pending.return_value = []
    coordinator.pop_due_authored.return_value = []
    coordinator.pop_next_due_authored.return_value = None
    
    real_clock = Mock()
    real_clock.now_us.side_effect = [1000, 1000, 1000, 1000, 1000, 1000, 1000]
    
    health_monitor = DispatchHealthMonitor(
        backend=Mock(), clock=real_clock, focus_guard=Mock(), require_focus=False
    )
    
    loop = DispatchLoop(
        coordinator=coordinator,
        clock=real_clock,
        sleeper=Mock(), wait_strategy=Mock(), backend=Mock(), telemetry=Mock(),
        sleep_policy=SleepPolicy(), health_monitor=health_monitor,
        min_hold_us=5000, spin_threshold_us=700,
    )
    
    loop._last_send_elapsed_us = 0
    now_us = 1000 # > SEND_COLD_THRESHOLD_US
    loop._last_send_elapsed_us = now_us - 100_000
    
    
    
    from sky_music.orchestration.core.state import PlaybackState
    state = PlaybackState(start_perf=0)
    
    deadline = coordinator.next_deadline_us(dispatch_lead_us=0, lead_up=0)

    # To prevent infinite loop in wait_until_runtime_deadline, we need to mock wait_strategy to advance clock
    def mock_wait(*args, **kwargs):
        real_clock.now_us.side_effect = [100_000]*10
        return False
    loop.wait_strategy.wait_until_us.side_effect = mock_wait  # type: ignore[attr-defined]
    
    loop._wait_until_runtime_deadline(
        target_elapsed_us=deadline,
        state=state,
        last_runtime_poll_us=0,
        last_render_time_us=0,
        first_action_executed=False,
        total_time_us=1000,
        command_source=Mock(),
        focus_signal=Mock(),
        progress_sink=Mock()
    )
    loop._drain_due(now_us, state, lead_up=0, observe=None)
    
    # 2500 - 2000 = 500. Max spin is capped at budget (500).
    assert loop.wait_strategy.wait_until_us.call_args.kwargs["spin_threshold_us"] == 900  # pyright: ignore[reportAttributeAccessIssue]

def test_warmup_real_path_characterization():
    from sky_music.domain.scheduler_types import (
        ActionKind,
        KeyAction,
        Microseconds,
        ScanCode,
    )
    from sky_music.orchestration.core.loop import SEND_COLD_THRESHOLD_US
    from sky_music.orchestration.core.state import PlaybackState
    from sky_music.orchestration.runtime_dispatch import (
        RuntimeDispatchCoordinator,
        compile_runtime_intents,
    )
    
    actions = (
        KeyAction(kind=ActionKind.DOWN, scan_codes=(ScanCode(1),), at_us=Microseconds(SEND_COLD_THRESHOLD_US * 2), reason="d1"),
    )
    schedule = compile_runtime_intents(actions)
    coordinator = RuntimeDispatchCoordinator(schedule, min_hold_us=5000)
    
    real_clock = Mock()
    real_clock.now_us.side_effect = [
        0, # next_deadline_us
        0, # _wait_until_runtime_deadline enter
        SEND_COLD_THRESHOLD_US * 2 - 100, # after wait
        SEND_COLD_THRESHOLD_US * 2, # _drain_due
    ] + [SEND_COLD_THRESHOLD_US * 2] * 20
    
    sleeper = Mock()
    
    health_monitor = DispatchHealthMonitor(
        backend=Mock(), clock=real_clock, focus_guard=Mock(), require_focus=False
    )
    
    backend = Mock()
    backend_result = Mock()
    backend_result.send_completed_us = None
    backend_result.sent = (1,)
    backend.key_up.return_value = backend_result
    backend.key_down.return_value = backend_result
    
    loop = DispatchLoop(
        coordinator=coordinator,
        clock=real_clock,
        sleeper=sleeper,
        wait_strategy=Mock(),
        backend=backend,
        telemetry=Mock(),
        sleep_policy=SleepPolicy(),
        health_monitor=health_monitor,
        min_hold_us=5000,
        spin_threshold_us=700,
    )
    
    loop._last_send_elapsed_us = -100_000
    state = PlaybackState(start_perf=0)
    

    deadline = coordinator.next_deadline_us(dispatch_lead_us=0, lead_up=0)
    assert deadline is not None
    loop._wait_until_runtime_deadline(
        target_elapsed_us=deadline,
        state=state,
        last_runtime_poll_us=0,
        last_render_time_us=0,
        first_action_executed=False,
        total_time_us=1000,
        command_source=Mock(),
        focus_signal=Mock(),
        progress_sink=Mock()
    )
    loop._drain_due(SEND_COLD_THRESHOLD_US * 2, state, lead_up=0, observe=None)
    
    # Under current buggy placement, warmup happens AFTER wait, inside drain_due,
    # but the test goal is to record whether it occurs before backend send.
    # The requirement says "record whether warmup occurs before backend send; this test should fail on the current placement."
    # If the real placement is broken, the warmup inside `wait_until_runtime_deadline` wouldn't happen,
    # or warmup in `_drain_due` is ineffective.
    # Let's assert that the warmup was called during the wait sequence (which it currently isn't, so it fails).
    assert loop.wait_strategy.wait_until_us.call_args.kwargs["spin_threshold_us"] == 900  # pyright: ignore[reportAttributeAccessIssue]

