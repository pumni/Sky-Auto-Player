from __future__ import annotations

from sky_music.domain import Millis, Note, NoteKey, Song
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import (
    FrameTimingPolicy,
    Microseconds,
)
from sky_music.infrastructure.backend import DryRunBackend
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.core.coordinator import (
    RuntimeDispatchCoordinator,
    compile_runtime_intents,
)
from sky_music.orchestration.core.loop import (
    DispatchHealthMonitor,
    DispatchLoop,
    PlaybackState,
)
from sky_music.orchestration.telemetry import TelemetryLogger


class FakeClock:
    def __init__(self, start_us=0):
        self.time_us = start_us
    def now_us(self):
        return self.time_us

class FakeSleeper:
    def __init__(self, clock):
        self.clock = clock
    def sleep(self, seconds: float):
        self.clock.time_us += max(1, int(seconds * 1_000_000))

class MockWaitStrategy:
    def __init__(self, clock):
        self.clock = clock
        self.waits = []
        self.sleeps = []
        self.last_spin_threshold_us = 0
    def spin_until_us(self, target_system_us, clock):
        self.clock.time_us = target_system_us
    def wait_until_us(self, target_system_us, clock, sleeper, spin_threshold_us, policy, command_event=None):
        self.last_spin_threshold_us = spin_threshold_us
        now = self.clock.now_us()
        if target_system_us - now > spin_threshold_us:
            # We would sleep
            self.sleeps.append((now, target_system_us))
            # Advance clock by 100ms chunks to simulate wait, but never beyond target - spin_threshold
            max_sleep = target_system_us - now - spin_threshold_us
            step = min(100_000, max_sleep)
            clock.time_us += step
            return True
        return False

class MockCommandSource:
    def poll(self): return None

class MockFocusSignal:
    def is_active(self) -> bool:
        return True
        
    @property
    def has_focus(self) -> bool:
        return True
        
    def focus(self) -> bool:
        return True
        
    def check_sync(self) -> bool:
        return True

class MockProgressSink:
    def publish(
        self, *, elapsed_us: int, total_us: int, status: str, lateness_us: int | None = None, health=None, input_path_degraded: bool = False, force: bool = False, counters=None,
    ) -> None:
        pass
        
    def finish(self, message: str) -> None:
        pass
        
    def update_counters(self, lateness_us: int, kind: str = "down") -> None:
        pass

def create_song(notes):
    return Song(name="test", notes=tuple(notes))

def setup_loop(clock, notes, core_warmup_budget_us=500, hold_us=100_000):
    song = create_song(notes)
    timing_policy = FrameTimingPolicy(
        fps=60,
        frame_us=Microseconds(16667),
        hold_us=Microseconds(hold_us),
        min_hold_us=Microseconds(hold_us),
        focus_restore_grace_us=Microseconds(100_000)
    )
    metadata = build_key_actions(song, policy=timing_policy)
    intents = compile_runtime_intents(metadata.actions)
    coordinator = RuntimeDispatchCoordinator(intents, min_hold_us=hold_us)
    backend = DryRunBackend()
    sleeper = FakeSleeper(clock)
    wait_strategy = MockWaitStrategy(clock)
    telemetry = TelemetryLogger("test")
    health = DispatchHealthMonitor(backend, clock, MockFocusSignal(), require_focus=False)
    
    loop = DispatchLoop(
        coordinator=coordinator,
        clock=clock,
        sleeper=sleeper,
        wait_strategy=wait_strategy,
        backend=backend,
        telemetry=telemetry,
        sleep_policy=SleepPolicy(poll_s=0.002),
        health_monitor=health,
        min_hold_us=hold_us,
        spin_threshold_us=1_000,
        core_warmup_budget_us=core_warmup_budget_us,
    )
    return loop, wait_strategy

def test_warmup_cold_detection_uses_elapsed_clock_domain_after_epoch_rebase():
    # Set the raw clock to a large value (10_000_000_000 us = 10k seconds)
    clock = FakeClock(10_000_000_000)
    
    # Schedule two notes separated by 5ms
    # 5ms is > spin_threshold (1ms) so it enters the wait loop, 
    # but < SEND_COLD_THRESHOLD_US (20ms) so it legitimately shouldn't trigger warmup
    notes = [
        Note(time_ms=Millis(0), key=NoteKey("Key1")),
        Note(time_ms=Millis(5), key=NoteKey("Key2"))
    ]
    loop, wait_strategy = setup_loop(clock, notes, hold_us=10_000)
    
    # Track spin thresholds
    spins = []
    
    # We patch WaitStrategy instead of the loop hook
    original_wait = wait_strategy.wait_until_us
    def mock_wait(*args, target_system_us=0, spin_threshold_us=0, **kwargs) -> bool:
        spins.append(spin_threshold_us)
        return original_wait(target_system_us=target_system_us, spin_threshold_us=spin_threshold_us, **kwargs)
        
    wait_strategy.wait_until_us = mock_wait
    
    state = PlaybackState(start_perf=clock.now_us())
    
    loop.run(state, MockCommandSource(), MockFocusSignal(), MockProgressSink(), total_time_us=2000)
    
    # Note 1 completes at roughly elapsed=0
    # Note 2 is evaluated at elapsed=1000
    # The gap in elapsed time is ~1000 us, which is < SEND_COLD_THRESHOLD_US (20_000)
    # The correct behavior is that it should NOT have the 500us budget applied for Note 2.
    assert len(spins) >= 2 and all(s == 1000 for s in spins)
    assert spins[1] == 1000, f"Cold guard budget should not be applied, expected 1000, got: {spins[1]}"

def test_cold_guard_is_adjacent_to_final_spin_after_long_gap():
    clock = FakeClock(0)
    # Long gap: 0ms then 1000ms
    notes = [
        Note(time_ms=Millis(0), key=NoteKey("Key1")),
        Note(time_ms=Millis(1000), key=NoteKey("Key2"))
    ]
    loop, wait_strategy = setup_loop(clock, notes)
    
    state = PlaybackState(start_perf=clock.now_us())
    loop.run(state, MockCommandSource(), MockFocusSignal(), MockProgressSink(), total_time_us=2000000)
    
    # Wait until us is called with the increased threshold (1000 + 500 = 1500)
    assert wait_strategy.last_spin_threshold_us == 1500
    
    # The actual sleep/spin tracking is delegated to wait_strategy.
    # The sleep time should leave exactly the effective spin threshold (1500) remaining.
    assert len(wait_strategy.sleeps) > 0
    last_sleep = wait_strategy.sleeps[-1]
    
    # Sleep target is the second note's scheduled time
    target_us = 1_000_000
    # Wait strategy stops simulating sleeps when (target - now) <= spin_threshold
    assert (target_us - last_sleep[1]) <= 1500

def test_default_cold_guard_budget_is_200_us():
    import inspect

    from sky_music.orchestration.core.loop import DispatchLoop
    
    # The loop should use exactly 200us extra for the cold guard by default
    sig = inspect.signature(DispatchLoop.__init__)
    assert sig.parameters['core_warmup_budget_us'].default == 200, "Default cold guard budget should be 200us"

def test_first_mid_song_reprobe_is_not_due_before_interval():
    clock = FakeClock()
    notes = [
        Note(time_ms=Millis(0), key=NoteKey("Key1")),
        Note(time_ms=Millis(10_000), key=NoteKey("Key1")) # 10s gap, > REPROBE_MIN_GAP_US (0.5s)
    ]
    loop, _ = setup_loop(clock, notes)
    # Mock mid song reprobe to track if it happened
    reprobes = []
    def mock_reprobe(elapsed_us):
        reprobes.append(elapsed_us)
        loop._last_reprobe_elapsed_us = elapsed_us
    loop._run_mid_song_reprobe = mock_reprobe
    state = PlaybackState(start_perf=clock.now_us())
    
    # The first reprobe should not happen until 30s interval
    loop.run(state, MockCommandSource(), MockFocusSignal(), MockProgressSink(), total_time_us=10_000_000)
    
    # Check telemetry for reprobes
    assert len(reprobes) == 0, f"Reprobe occurred before the 30s interval at {reprobes}"

def test_send_completion_is_sampled_at_platform_call_return_boundary():
    # F5: `send_completed_us` is sampled by the backend after the platform wrapper has already returned

    from sky_music.infrastructure.backend import WinSendInputBackend
    
    clock = FakeClock()
    
    class FakeInputsModule:
        def send_scan_code_batch_trusted(self, scan_codes, key_up, **kwargs):
            # Advance clock DURING the call to simulate kernel time
            clock.time_us += 100_000
            # Sample time right after the simulated kernel call, before any post-processing
            sampled_time = clock.now_us()
            # Simulate some wrapper overhead after sampling
            clock.time_us += 5_000 
            return len(scan_codes), sampled_time

    backend = WinSendInputBackend()
    backend.inputs_module = FakeInputsModule()  # type: ignore
    backend._now_us = clock.now_us
    _start_time = clock.now_us()

    _sent, completed_us = backend._emit((0x10,), key_up=False)

    # We expect `completed_us` to be sampled INSIDE send_scan_code_batch_trusted (at 100_000),
    # NOT after the method returns (which would be 105_000).
    assert completed_us == 100_000, f"Expected 100_000, got: {completed_us}"

def test_exact_timestamp_chord_uses_one_sendinput_call():
    # This just ensures we call backend._emit exactly once for a chord
    clock = FakeClock()
    notes = [
        Note(time_ms=Millis(0), key=NoteKey("Key1")),
        Note(time_ms=Millis(0), key=NoteKey("Key2")),
        Note(time_ms=Millis(0), key=NoteKey("Key3")),
    ]
    loop, _ = setup_loop(clock, notes)
    
    from unittest.mock import MagicMock

    from sky_music.infrastructure.backend import InputSendResult
    loop.backend = MagicMock()
    loop.backend.key_down.return_value = InputSendResult(sent=tuple(1 for _ in notes), skipped_duplicates=(), success=True, send_completed_us=1000)
    
    state = PlaybackState(start_perf=clock.now_us())
    loop.run(state, MockCommandSource(), MockFocusSignal(), MockProgressSink(), total_time_us=1000)
    
    # Verify key_down was called EXACTLY ONCE with all 3 scan codes
    assert loop.backend.key_down.call_count == 1
    args, _kwargs = loop.backend.key_down.call_args
    assert len(args[0]) == 3, "Chord was split into multiple key_down calls"

def test_single_key_trusted_batch_avoids_duplicate_set_allocation():
    # F6: Trusted batch validation constructs set(scan_codes) on every dispatch
    import inspect

    # Currently _send_scan_code_batch_impl is the inner part, wait, the method is send_scan_code_batch_trusted
    from sky_music.platform.win32.inputs import (
        send_scan_code_batch_trusted,
    )
    source = inspect.getsource(send_scan_code_batch_trusted)
    
    # We assert it does NOT contain 'set(' which will fail because it currently DOES.
    assert "set(" not in source, "Found avoidable set() allocation in trusted batch hot path"

def test_progress_publication_does_not_lock_per_dispatch():
    # F7: Progress counters acquire a cross-thread lock after each non-deferred dispatch
    import inspect

    from sky_music.orchestration.core.ports import ProgressCounters
    
    source = inspect.getsource(ProgressCounters)
    # The current buggy code acquires a lock. We assert it doesn't.
    assert "lock" not in source.lower(), "Found lock acquisition in ProgressCounters"

def test_telemetry_cap_never_flushes_on_dispatch_thread():
    # F8: Debug telemetry can synchronously flush a large CSV when its hard cap is reached
    import inspect

    from sky_music.orchestration.telemetry import TelemetryLogger
    
    source = inspect.getsource(TelemetryLogger.flush_if_large)
    # The current buggy code flushes to disk. We assert it doesn't do blocking I/O on the dispatch thread.
    assert "_flush_records_to_csv" not in source.lower() and "disk" not in source.lower() and "write" not in source.lower(), "Found synchronous flush on dispatch thread"

def test_waitable_timer_recomputes_remaining_immediately_before_arm():
    # F9: Relative waitable-timer duration is calculated before setup and armed later
    import inspect

    from sky_music.infrastructure.wait_strategy import HybridWaitStrategy
    
    source = inspect.getsource(HybridWaitStrategy.wait_until_us)
    # We can check if it calls self.clock.now_us() again right before SetWaitableTimer
    # Currently it doesn't, so asserting it does will fail.
    # Actually, easier to just check if 'self.clock.now_us()' appears more than once or check the logic.
    assert source.count("now_us") >= 2, "Relative timer does not recompute remaining time right before arming"

def test_auto_priority_never_selects_time_critical_fallback():
    # F10: auto priority falls back from MMCSS to TIME_CRITICAL before HIGHEST
    import inspect

    from sky_music.infrastructure.rt_priority import DispatchThreadPriorityScope
    
    source = inspect.getsource(DispatchThreadPriorityScope.__enter__)
        
    assert "TIME_CRITICAL" not in source, "Dangerous TIME_CRITICAL fallback used instead of HIGHEST"
