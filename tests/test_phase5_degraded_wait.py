import threading
import time

import sky_music.platform.win32.inputs as inputs
from sky_music.infrastructure.timing import Clock, Sleeper, SleepPolicy
from sky_music.infrastructure.wait_strategy import HybridWaitStrategy
from sky_music.orchestration.core.loop import DispatchHealthMonitor, DispatchLoop


class MockClock(Clock):
    def __init__(self):
        self._now = 0

    def now_us(self) -> int:
        return self._now

    def advance(self, us: int):
        self._now += us


class DegradedSleeper(Sleeper):
    """A sleeper that is NOT high resolution."""
    def __init__(self, clock):
        self.clock = clock
        self.sleeps = []

    def sleep(self, seconds: float):
        self.sleeps.append(seconds)
        self.clock.advance(int(seconds * 1_000_000))


def test_wait_until_us_returns_true_when_event_signalled():
    # Unit-level test
    clock = MockClock()
    sleeper = DegradedSleeper(clock)
    strategy = HybridWaitStrategy(enable_event_wait=True)
    policy = SleepPolicy()

    command_event = inputs.create_auto_reset_event()
    if command_event is not None:
        inputs.set_event(command_event)
    try:
        clock._now = 1000
        target = 10_000
        # When event is already signalled, it should return True immediately (or on first wait)
        woken = strategy.wait_until_us(
            target_system_us=target,
            clock=clock,
            sleeper=sleeper,
            spin_threshold_us=700,
            policy=policy,
            command_event=command_event
        )
        assert woken is True
    finally:
        if command_event is not None:
            inputs.close_handle(command_event)

def test_quit_honoured_when_high_res_timer_unavailable(monkeypatch):
    from unittest.mock import Mock

    from sky_music.orchestration.core.ports import PLAYBACK_QUIT

    strategy = HybridWaitStrategy(enable_event_wait=True)
    
    # We must patch inputs.wait_for_multiple_objects so we can simulate the passage of time
    # and setting the event if needed, but for simplicity, we can just start a background thread
    # that sets the event, and use a real clock!
    
    class RealClock:
        def now_us(self):
            return int(time.perf_counter() * 1_000_000)
    
    class RealDegradedSleeper:
        def sleep(self, seconds: float):
            time.sleep(seconds)

    real_clock = RealClock()
    real_sleeper = RealDegradedSleeper()

    command_event = inputs.create_auto_reset_event()
    
    # Use real dispatch loop
    health_monitor = DispatchHealthMonitor(
        backend=Mock(),
        clock=real_clock,
        focus_guard=Mock(),
        require_focus=False
    )
    
    coordinator = Mock()
    coordinator.is_finished.return_value = False
    coordinator.next_deadline_us.return_value = 5_000_000
    
    telemetry_mock = Mock()
    telemetry_mock.runtime_options = {}

    loop = DispatchLoop(
        coordinator=coordinator,
        clock=real_clock,
        sleeper=real_sleeper,
        wait_strategy=strategy,
        backend=Mock(),
        telemetry=telemetry_mock,
        sleep_policy=SleepPolicy(),
        health_monitor=health_monitor,
        min_hold_us=5000,
        spin_threshold_us=700,
    )
    
    from sky_music.orchestration.core.state import PlaybackState
    state = PlaybackState(start_perf=real_clock.now_us())
    
    class TestCommandSource:
        def __init__(self):
            self.quit_enqueued = False
        def poll(self):
            if self.quit_enqueued:
                return "quit"
            return None
            
    cmd_source = TestCommandSource()
    
    def signal_quit_soon():
        time.sleep(0.1)
        cmd_source.quit_enqueued = True
        if command_event is not None:
            inputs.set_event(command_event)
        
    threading.Thread(target=signal_quit_soon, daemon=True).start()
    
    t0 = time.perf_counter()
    result = loop.run(
        state=state,
        command_source=cmd_source,
        focus_signal=Mock(),
        progress_sink=Mock(),
        total_time_us=5_000_000,
        command_event=command_event
    )
    t1 = time.perf_counter()
    
    if command_event is not None:
        inputs.close_handle(command_event)
    
    assert result == PLAYBACK_QUIT
    assert t1 - t0 < 4.0, "Loop blocked for full duration instead of waking early on quit!"

class HighResSleeper(DegradedSleeper):
    is_high_resolution = True
    handle = 999

def test_wait_failed_degrades_gracefully(monkeypatch):

    import sky_music.platform.win32.inputs as inputs
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.infrastructure.wait_strategy import HybridWaitStrategy
    
    clock = MockClock()
    sleeper = HighResSleeper(clock)
    strategy = HybridWaitStrategy(enable_event_wait=True)
    
    # Force platform wait to return None (WAIT_FAILED)
    monkeypatch.setattr(inputs, "wait_for_multiple_objects", lambda *args, **kwargs: None)
    monkeypatch.setattr(inputs, "set_waitable_timer_relative_us", lambda *args, **kwargs: True)
    
    spin_remaining_at_call = []
    def mock_spin(self_obj, target, clock):
        spin_remaining_at_call.append(target - clock.now_us())
        clock.advance(target - clock.now_us())
        
    monkeypatch.setattr(HybridWaitStrategy, "spin_until_us", mock_spin)
    
    target = 5_000_000
    
    # wait_until_us should return early without full gap spin
    woken = strategy.wait_until_us(
        target_system_us=target,
        clock=clock,
        sleeper=sleeper,
        spin_threshold_us=700,
        policy=SleepPolicy(),
        command_event=123, # fake handle
    )
    
    # Control should return (woken=False meaning time elapsed or degraded poll loop returned control)
    assert woken is False
    
    # Should bounded sleep, not full gap!
    assert len(sleeper.sleeps) > 0
    assert sleeper.sleeps[0] <= 0.002
    
    if spin_remaining_at_call:
        assert spin_remaining_at_call[0] <= 700

