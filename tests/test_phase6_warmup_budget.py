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
    
    real_clock = Mock()
    real_clock.now_us.return_value = 2000 # now > deadline
    
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
    loop._last_send_completed_us = 0
    now_us = 100_000 # > SEND_COLD_THRESHOLD_US (20_000)
    
    # Hook to record warmup spins
    spins = []
    def core_warmup_hook(us):
        spins.append(us)
        
    loop.core_warmup_hook = core_warmup_hook
    
    from sky_music.orchestration.core.state import PlaybackState
    state = PlaybackState(start_perf=0)
    
    loop._drain_due(now_us, state, lead_up=0, observe=None)
    
    # Assert warmup was not called (or called with <= 0 budget which gets skipped)
    assert len(spins) == 0, f"Warmup was unexpectedly called with spins: {spins}"
    assert coordinator.pop_due_pending.called

def test_idle_warmup_uses_effective_deadline_when_future():
    # Phase 6: test that warmup budget uses the effective deadline when both are future
    coordinator = Mock()
    coordinator.next_authored_us.return_value = 10_000_000
    coordinator.next_deadline_us.return_value = 2500  # next pending release in 500us
    
    coordinator.pop_due_pending.return_value = []
    coordinator.pop_due_authored.return_value = []
    
    real_clock = Mock()
    real_clock.now_us.return_value = 2000
    
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
    
    loop._last_send_completed_us = 0
    now_us = 2000 # > SEND_COLD_THRESHOLD_US
    loop._last_send_completed_us = now_us - 100_000
    
    spins = []
    loop.core_warmup_hook = lambda us: spins.append(us)
    
    from sky_music.orchestration.core.state import PlaybackState
    state = PlaybackState(start_perf=0)
    
    loop._drain_due(now_us, state, lead_up=0, observe=None)
    
    # 2500 - 2000 = 500. Max spin is capped at CORE_WARMUP_SPIN_MAX_US (200).
    assert len(spins) == 1
    assert spins[0] == 200
